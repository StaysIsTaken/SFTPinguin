//! WebDAV (HTTP / HTTPS) – works with Nextcloud, ownCloud, Synology, Apache, nginx, ...

use async_trait::async_trait;
use futures_util::StreamExt;
use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, CONTROLS};
use quick_xml::events::Event;
use quick_xml::Reader as XmlReader;
use reqwest::{header, Client, Method, StatusCode};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::{join, Reader, RemoteFs, Writer};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::model::{AuthMethod, Capabilities, ConnectConfig, EntryKind, FileEntry, Protocol};

/// Characters that must be escaped inside a URL path segment.
const SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}')
    .add(b'[')
    .add(b']')
    .add(b'^')
    .add(b'|')
    .add(b'\\')
    .add(b';')
    .add(b'&')
    .add(b'+')
    .add(b'=');

const PROPFIND_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:">
  <d:prop>
    <d:resourcetype/>
    <d:getcontentlength/>
    <d:getlastmodified/>
  </d:prop>
</d:propfind>"#;

pub struct WebDavFs {
    client: Client,
    /// `https://host:port` without trailing slash
    origin: String,
    user: Option<String>,
    password: Option<String>,
    home: String,
}

fn http_err(e: reqwest::Error) -> AppError {
    // include the root cause (e.g. "invalid peer certificate: UnknownIssuer")
    let mut msg = e.to_string();
    let mut source = std::error::Error::source(&e);
    while let Some(s) = source {
        let text = s.to_string();
        if !msg.contains(&text) {
            msg = format!("{msg}: {text}");
        }
        source = s.source();
    }
    if e.is_timeout() || e.is_connect() {
        AppError::connection(msg)
    } else {
        AppError::protocol(msg)
    }
}

fn status_err(status: StatusCode, what: &str) -> AppError {
    let code = match status {
        StatusCode::UNAUTHORIZED => ErrorCode::AuthFailed,
        StatusCode::FORBIDDEN => ErrorCode::PermissionDenied,
        StatusCode::NOT_FOUND => ErrorCode::NotFound,
        StatusCode::METHOD_NOT_ALLOWED if what == "MKCOL" => ErrorCode::AlreadyExists,
        StatusCode::PRECONDITION_FAILED => ErrorCode::AlreadyExists,
        StatusCode::NOT_IMPLEMENTED => ErrorCode::Unsupported,
        _ => ErrorCode::Protocol,
    };
    AppError::new(code, format!("{what}: HTTP {status}"))
}

fn encode_path(path: &str) -> String {
    path.split('/')
        .map(|seg| utf8_percent_encode(seg, SEGMENT).to_string())
        .collect::<Vec<_>>()
        .join("/")
}

impl WebDavFs {
    pub async fn connect(cfg: &ConnectConfig) -> AppResult<Self> {
        let site = &cfg.site;
        let scheme = if site.protocol == Protocol::Webdavs {
            "https"
        } else {
            "http"
        };
        // Allow users to paste a full URL into the host field.
        let mut host = site.host.trim().to_string();
        let mut url_path = String::new();
        for prefix in ["https://", "http://", "davs://", "dav://"] {
            if let Some(rest) = host.strip_prefix(prefix) {
                host = rest.to_string();
            }
        }
        if let Some(i) = host.find('/') {
            url_path = host[i..].to_string();
            host.truncate(i);
        }
        let origin = if site.port.is_some_and(|p| p != 0) {
            format!("{scheme}://{host}:{}", site.port())
        } else {
            format!("{scheme}://{host}")
        };

        let client = Client::builder()
            .user_agent(concat!("SFTPinguin/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(std::time::Duration::from_secs(site.timeout.clamp(3, 300)))
            // webpki roots + ring: works identically on desktop and mobile
            .tls_backend_preconfigured(
                crate::tls::client_config(site.insecure_tls, false)?
                    .as_ref()
                    .clone(),
            )
            .build()
            .map_err(http_err)?;

        let (user, password) = match site.auth {
            AuthMethod::Anonymous => (None, None),
            _ => {
                if cfg.password.is_none() {
                    return Err(AppError::new(
                        ErrorCode::PasswordRequired,
                        "A password is required",
                    ));
                }
                (Some(site.username.clone()), cfg.password.clone())
            }
        };

        let mut home = if !site.remote_path.trim().is_empty() {
            site.remote_path.trim().to_string()
        } else if !url_path.is_empty() {
            url_path
        } else {
            "/".to_string()
        };
        if !home.starts_with('/') {
            home.insert(0, '/');
        }

        let fs = Self {
            client,
            origin,
            user,
            password,
            home,
        };
        // Verify credentials and the base path.
        let resp = fs
            .request(Method::from_bytes(b"PROPFIND").unwrap(), &fs.home)
            .header("Depth", "0")
            .header(header::CONTENT_TYPE, "application/xml")
            .body(PROPFIND_BODY)
            .send()
            .await
            .map_err(http_err)?;
        let status = resp.status();
        if status == StatusCode::UNAUTHORIZED {
            return Err(AppError::new(ErrorCode::AuthFailed, "Login incorrect"));
        }
        if !status.is_success() {
            return Err(status_err(status, "PROPFIND"));
        }
        Ok(fs)
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.origin, encode_path(path))
    }

    fn request(&self, method: Method, path: &str) -> reqwest::RequestBuilder {
        let mut req = self.client.request(method, self.url(path));
        if let Some(user) = &self.user {
            req = req.basic_auth(user, self.password.as_ref());
        }
        req
    }

    async fn propfind(&self, path: &str, depth: &str) -> AppResult<Option<Vec<FileEntry>>> {
        let resp = self
            .request(Method::from_bytes(b"PROPFIND").unwrap(), path)
            .header("Depth", depth)
            .header(header::CONTENT_TYPE, "application/xml")
            .body(PROPFIND_BODY)
            .send()
            .await
            .map_err(http_err)?;
        if resp.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(status_err(resp.status(), "PROPFIND"));
        }
        let body = resp.text().await.map_err(http_err)?;
        Ok(Some(parse_multistatus(&body)?))
    }
}

fn parse_multistatus(xml: &str) -> AppResult<Vec<FileEntry>> {
    let mut reader = XmlReader::from_str(xml);
    let mut out = Vec::new();

    let mut in_response = false;
    let mut current_tag = String::new();
    let mut text = String::new();
    let mut href = String::new();
    let mut is_dir = false;
    let mut size = 0u64;
    let mut modified: Option<i64> = None;

    loop {
        let ev = reader
            .read_event()
            .map_err(|e| AppError::protocol(format!("Invalid WebDAV response: {e}")))?;
        match ev {
            Event::Start(e) | Event::Empty(e) => {
                let name = e.local_name().as_ref().to_lowercase();
                match name.as_str() {
                    "response" => {
                        in_response = true;
                        href.clear();
                        is_dir = false;
                        size = 0;
                        modified = None;
                    }
                    "collection" if in_response => is_dir = true,
                    _ => {}
                }
                current_tag = name;
                text.clear();
            }
            Event::Text(t) => text.push_str(&t.xml10_content()),
            Event::GeneralRef(r) => {
                if let Ok(Some(c)) = r.resolve_char_ref() {
                    text.push(c);
                } else {
                    text.push_str(match r.xml10_content().as_ref() {
                        "amp" => "&",
                        "lt" => "<",
                        "gt" => ">",
                        "quot" => "\"",
                        "apos" => "'",
                        _ => "",
                    });
                }
            }
            Event::End(e) => {
                let name = e.local_name().as_ref().to_lowercase();
                if in_response {
                    match name.as_str() {
                        "href" => href = text.trim().to_string(),
                        "getcontentlength" => size = text.trim().parse().unwrap_or(0),
                        "getlastmodified" => {
                            modified = chrono::DateTime::parse_from_rfc2822(text.trim())
                                .ok()
                                .map(|d| d.timestamp_millis())
                        }
                        "response" => {
                            in_response = false;
                            // href may be absolute (http://host/path) or a path
                            let mut path = href.clone();
                            if let Some(idx) = path.find("://") {
                                let rest = &path[idx + 3..];
                                path = rest
                                    .find('/')
                                    .map(|i| rest[i..].to_string())
                                    .unwrap_or_default();
                            }
                            let decoded = percent_decode_str(&path).decode_utf8_lossy().to_string();
                            let clean = decoded.trim_end_matches('/').to_string();
                            let clean = if clean.is_empty() {
                                "/".to_string()
                            } else {
                                clean
                            };
                            out.push(FileEntry {
                                name: super::file_name(&clean),
                                path: clean,
                                kind: if is_dir {
                                    EntryKind::Dir
                                } else {
                                    EntryKind::File
                                },
                                size,
                                modified,
                                mode: None,
                                owner: None,
                                group: None,
                                is_link: false,
                                link_target: None,
                            });
                        }
                        _ => {}
                    }
                }
                let _ = &current_tag;
                text.clear();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

#[async_trait]
impl RemoteFs for WebDavFs {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            chmod: false,
            rename_dirs: true,
            symlinks: false,
            server_side_copy: true,
        }
    }

    async fn home(&self) -> AppResult<String> {
        Ok(self.home.clone())
    }

    async fn list(&self, path: &str) -> AppResult<Vec<FileEntry>> {
        let entries = self
            .propfind(path, "1")
            .await?
            .ok_or_else(|| AppError::not_found(format!("{path} not found")))?;
        let own = path.trim_end_matches('/');
        let own = if own.is_empty() { "/" } else { own };
        Ok(entries
            .into_iter()
            .filter(|e| e.path != own)
            .map(|mut e| {
                // keep the requested prefix (servers may normalise differently)
                e.path = join(path, &e.name);
                e
            })
            .collect())
    }

    async fn stat(&self, path: &str) -> AppResult<Option<FileEntry>> {
        Ok(self.propfind(path, "0").await?.and_then(|mut v| {
            if v.is_empty() {
                None
            } else {
                Some(v.remove(0))
            }
        }))
    }

    async fn mkdir(&self, path: &str) -> AppResult<()> {
        let resp = self
            .request(Method::from_bytes(b"MKCOL").unwrap(), path)
            .send()
            .await
            .map_err(http_err)?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(status_err(resp.status(), "MKCOL"))
        }
    }

    async fn remove_file(&self, path: &str) -> AppResult<()> {
        let resp = self
            .request(Method::DELETE, path)
            .send()
            .await
            .map_err(http_err)?;
        if resp.status().is_success() || resp.status() == StatusCode::NOT_FOUND {
            Ok(())
        } else {
            Err(status_err(resp.status(), "DELETE"))
        }
    }

    async fn remove_dir(&self, path: &str) -> AppResult<()> {
        let dir = format!("{}/", path.trim_end_matches('/'));
        self.remove_file(&dir).await
    }

    async fn rename(&self, from: &str, to: &str) -> AppResult<()> {
        let resp = self
            .request(Method::from_bytes(b"MOVE").unwrap(), from)
            .header("Destination", self.url(to))
            .header("Overwrite", "F")
            .send()
            .await
            .map_err(http_err)?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(status_err(resp.status(), "MOVE"))
        }
    }

    async fn download(&self, path: &str, sink: Writer<'_>) -> AppResult<()> {
        let resp = self
            .request(Method::GET, path)
            .send()
            .await
            .map_err(http_err)?;
        if !resp.status().is_success() {
            return Err(status_err(resp.status(), "GET"));
        }
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(http_err)?;
            sink.write_all(&chunk).await?;
        }
        sink.flush().await?;
        Ok(())
    }

    async fn upload(&self, source: Reader<'_>, path: &str, size: u64) -> AppResult<()> {
        // Feed the request body from the (borrowed) reader through a channel so that no
        // 'static reader is required; both futures run concurrently in this task.
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<Vec<u8>, std::io::Error>>(4);
        let body =
            reqwest::Body::wrap_stream(futures_util::stream::poll_fn(move |cx| rx.poll_recv(cx)));
        let send = self
            .request(Method::PUT, path)
            .header(header::CONTENT_LENGTH, size)
            .body(body)
            .send();
        let pump = async move {
            let mut buf = vec![0u8; 256 * 1024];
            loop {
                let n = source.read(&mut buf).await?;
                if n == 0 {
                    break;
                }
                if tx.send(Ok(buf[..n].to_vec())).await.is_err() {
                    break; // request finished early (error response)
                }
            }
            Ok::<_, std::io::Error>(())
        };
        let (resp, pumped) = futures_util::join!(send, pump);
        pumped?;
        let resp = resp.map_err(http_err)?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(status_err(resp.status(), "PUT"))
        }
    }

    async fn is_alive(&self) -> bool {
        true
    }

    async fn close(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multistatus() {
        let xml = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:">
 <d:response><d:href>/remote.php/dav/files/me/</d:href>
  <d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat></d:response>
 <d:response><d:href>/remote.php/dav/files/me/My%20File.txt</d:href>
  <d:propstat><d:prop><d:resourcetype/><d:getcontentlength>42</d:getcontentlength>
  <d:getlastmodified>Tue, 01 Sep 2026 10:00:00 GMT</d:getlastmodified></d:prop></d:propstat></d:response>
 <d:response><d:href>https://cloud.example.com/remote.php/dav/files/me/Fotos/</d:href>
  <d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat></d:response>
</d:multistatus>"#;
        let entries = parse_multistatus(xml).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[1].name, "My File.txt");
        assert_eq!(entries[1].size, 42);
        assert!(entries[1].modified.is_some());
        assert_eq!(entries[2].path, "/remote.php/dav/files/me/Fotos");
        assert!(entries[2].is_dir());
        assert_eq!(encode_path("/a b/c#d"), "/a%20b/c%23d");
    }
}
