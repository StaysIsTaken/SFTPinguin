//! FTP, FTPS (explicit, AUTH TLS) and FTPS (implicit) via suppaftp.

use std::time::{Duration, UNIX_EPOCH};

use async_trait::async_trait;
use suppaftp::list::{File as FtpFile, ListParser, PosixPexQuery};
use suppaftp::tokio::{AsyncFtpStream, AsyncRustlsConnector, AsyncRustlsFtpStream};
use suppaftp::types::FileType;
use suppaftp::{FtpError, Status};
use tokio::sync::Mutex;

use super::{join, Reader, RemoteFs, Writer};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::model::{AuthMethod, Capabilities, ConnectConfig, EntryKind, FileEntry, Protocol};
use crate::tls::{CertStore, TlsContext};

enum Conn {
    Plain(AsyncFtpStream),
    Tls(AsyncRustlsFtpStream),
}

/// Runs the same code for plain and TLS connections.
macro_rules! with_conn {
    ($self:ident, $s:ident => $body:expr) => {{
        let mut guard = $self.conn.lock().await;
        match &mut *guard {
            Conn::Plain($s) => $body,
            Conn::Tls($s) => $body,
        }
    }};
}

pub struct FtpFs {
    conn: Mutex<Conn>,
    mlsd: bool,
    mlst: bool,
}

fn ftp_err(e: FtpError) -> AppError {
    match e {
        FtpError::ConnectionError(io) => AppError::connection(io.to_string()),
        FtpError::SecureError(msg) => AppError::connection(format!("TLS: {msg}")),
        FtpError::UnexpectedResponse(resp) => {
            let text = resp.as_string().unwrap_or_default();
            let code = match resp.status {
                Status::NotLoggedIn => ErrorCode::AuthFailed,
                Status::FileUnavailable => ErrorCode::NotFound,
                Status::CommandNotImplemented
                | Status::NotImplemented
                | Status::NotImplementedParameter => ErrorCode::Unsupported,
                _ => ErrorCode::Protocol,
            };
            let text = text.trim();
            let msg = if text.starts_with(&resp.status.code().to_string()) {
                text.to_string()
            } else {
                format!("{} {text}", resp.status.code())
            };
            AppError::new(code, msg)
        }
        FtpError::DataConnectionAlreadyOpen => {
            AppError::connection("A data connection is already open")
        }
        other => AppError::protocol(other.to_string()),
    }
}

impl FtpFs {
    pub async fn connect(cfg: &ConnectConfig, certs: &CertStore) -> AppResult<Self> {
        let site = &cfg.site;
        let host = site.host.trim();
        let addr = (host, site.port());
        let tls_ctx = TlsContext::new(host, site.port(), certs);

        let tls = |tls12_only: bool| -> AppResult<AsyncRustlsConnector> {
            let config = tls_ctx.client_config(tls12_only)?;
            Ok(AsyncRustlsConnector::from(
                suppaftp::tokio_rustls::TlsConnector::from(config),
            ))
        };
        let open_tls = |tls12_only: bool| async move {
            match site.protocol {
                Protocol::Ftps => {
                    let plain = AsyncRustlsFtpStream::connect(addr).await.map_err(ftp_err)?;
                    plain
                        .into_secure(tls(tls12_only)?, host)
                        .await
                        .map_err(|e| match e {
                            // AUTH TLS was refused: the server has no encryption at all
                            FtpError::UnexpectedResponse(resp) => AppError::new(
                                ErrorCode::TlsNotSupported,
                                format!(
                                    "The server does not support encrypted FTP (AUTH TLS): {}",
                                    resp.as_string().unwrap_or_default().trim()
                                ),
                            ),
                            other => ftp_err(other),
                        })
                }
                _ => AsyncRustlsFtpStream::connect_secure_implicit(addr, tls(tls12_only)?, host)
                    .await
                    .map_err(ftp_err),
            }
        };

        let mut conn = match site.protocol {
            Protocol::Ftp => Conn::Plain(AsyncFtpStream::connect(addr).await.map_err(ftp_err)?),
            Protocol::Ftps | Protocol::FtpsImplicit => {
                // Prefer TLS 1.2: several popular servers (e.g. vsftpd) break TLS 1.3 data
                // connections. Fall back to TLS 1.3 for servers that no longer offer 1.2.
                let first = open_tls(true).await;
                let result = match first {
                    Err(e)
                        if e.message.to_lowercase().contains("protocolversion")
                            || e.message.to_lowercase().contains("protocol version") =>
                    {
                        open_tls(false).await
                    }
                    other => other,
                };
                match result {
                    Ok(s) => Conn::Tls(s),
                    // untrusted / changed certificate: let the user decide
                    Err(e) => return Err(tls_ctx.certificate_error().unwrap_or(e)),
                }
            }
            _ => return Err(AppError::invalid("Not an FTP protocol")),
        };

        let (user, pass) = match site.auth {
            AuthMethod::Anonymous => (
                "anonymous".to_string(),
                cfg.password
                    .clone()
                    .unwrap_or_else(|| "anonymous@".to_string()),
            ),
            _ => {
                let Some(pass) = cfg.password.clone() else {
                    return Err(AppError::new(
                        ErrorCode::PasswordRequired,
                        "A password is required",
                    ));
                };
                (site.username.clone(), pass)
            }
        };

        let passive = site.passive;
        macro_rules! setup {
            ($s:ident) => {{
                if passive {
                    $s.set_passive_nat_workaround(true);
                }
                $s.login(user.as_str(), pass.as_str())
                    .await
                    .map_err(|e| match ftp_err(e) {
                        e if e.code == ErrorCode::AuthFailed => {
                            AppError::new(ErrorCode::AuthFailed, "Login incorrect")
                        }
                        e => e,
                    })?;
                $s.transfer_type(FileType::Binary).await.map_err(ftp_err)?;
                // Ask for UTF-8 file names, ignore servers that don't know it.
                let _ = $s.opts("UTF8", Some("ON")).await;
                let features = $s.feat().await.unwrap_or_default();
                if features.keys().any(|k| k.eq_ignore_ascii_case("MLST")) {
                    // request permissions and owners in MLSD listings
                    let _ = $s
                        .opts("MLST", Some("type;size;modify;perm;unix.mode;"))
                        .await;
                }
                features
            }};
        }
        let features = match &mut conn {
            Conn::Plain(s) => setup!(s),
            Conn::Tls(s) => setup!(s),
        };
        if !passive {
            conn = match conn {
                Conn::Plain(s) => Conn::Plain(s.active_mode(Duration::from_secs(30))),
                Conn::Tls(s) => Conn::Tls(s.active_mode(Duration::from_secs(30))),
            };
        }
        let has = |f: &str| features.keys().any(|k| k.eq_ignore_ascii_case(f));
        Ok(Self {
            mlsd: has("MLSD") || has("MLST"),
            mlst: has("MLST"),
            conn: Mutex::new(conn),
        })
    }

    fn to_entry(dir: &str, f: &FtpFile) -> FileEntry {
        let mut mode = 0u32;
        for (i, who) in [
            PosixPexQuery::Owner,
            PosixPexQuery::Group,
            PosixPexQuery::Others,
        ]
        .into_iter()
        .enumerate()
        {
            let shift = 6 - 3 * i as u32;
            let bits = (f.can_read(who) as u32) << 2
                | (f.can_write(who) as u32) << 1
                | f.can_execute(who) as u32;
            mode |= bits << shift;
        }
        let modified = f
            .modified()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|d| d.as_millis() as i64)
            .filter(|m| *m > 0);
        FileEntry {
            name: f.name().to_string(),
            path: join(dir, f.name()),
            kind: if f.is_directory() {
                EntryKind::Dir
            } else {
                EntryKind::File
            },
            size: f.size() as u64,
            modified,
            mode: if mode == 0 { None } else { Some(mode) },
            owner: f.uid().map(|u| u.to_string()),
            group: f.gid().map(|g| g.to_string()),
            is_link: f.is_symlink(),
            link_target: f.symlink().map(|p| p.to_string_lossy().to_string()),
        }
    }

    fn parse_lines(dir: &str, lines: &[String], mlsd: bool) -> Vec<FileEntry> {
        lines
            .iter()
            .filter_map(|line| {
                if mlsd {
                    parse_mlsx(dir, line)
                } else {
                    ListParser::parse_posix(line)
                        .or_else(|_| ListParser::parse_dos(line))
                        .ok()
                        .map(|f| Self::to_entry(dir, &f))
                }
            })
            .filter(|e| e.name != "." && e.name != ".." && !e.name.is_empty())
            .collect()
    }
}

/// Tolerant parser for MLSD / MLST lines (RFC 3659), e.g.
/// `modify=20240101120000;perm=r;size=12;type=file;unix.mode=0644; name.txt`
fn parse_mlsx(dir: &str, line: &str) -> Option<FileEntry> {
    let line = line.trim_start();
    let (facts, name) = match line.find("; ") {
        Some(i) => (&line[..i], &line[i + 2..]),
        None => ("", line.strip_prefix(' ').unwrap_or(line)),
    };
    // MLST returns the full path, MLSD the plain name
    let name = name.trim_end_matches(['\r', '\n']);
    let name = name.rsplit('/').next().unwrap_or(name).to_string();
    let mut kind = EntryKind::File;
    let mut size = 0u64;
    let mut modified = None;
    let mut mode = None;
    let mut owner = None;
    let mut group = None;
    let mut is_link = false;
    let mut link_target = None;
    for fact in facts.split(';') {
        let Some((key, value)) = fact.split_once('=') else {
            continue;
        };
        match key.to_ascii_lowercase().as_str() {
            "type" => {
                let v = value.to_ascii_lowercase();
                if v == "dir" || v == "cdir" || v == "pdir" {
                    kind = EntryKind::Dir;
                    if v != "dir" {
                        return None; // "." and ".."
                    }
                } else if let Some(target) = v.strip_prefix("os.unix=slink:") {
                    is_link = true;
                    link_target = Some(value[value.len() - target.len()..].to_string());
                } else if v.starts_with("os.unix=symlink") || v == "link" {
                    is_link = true;
                }
            }
            "size" | "sizd" => size = value.parse().unwrap_or(0),
            "modify" => {
                let digits: String = value.chars().take(14).collect();
                modified = chrono::NaiveDateTime::parse_from_str(&digits, "%Y%m%d%H%M%S")
                    .ok()
                    .map(|t| t.and_utc().timestamp_millis());
            }
            "unix.mode" => {
                let v = value.trim_start_matches("0o");
                mode = u32::from_str_radix(v, 8).ok().map(|m| m & 0o7777);
            }
            "unix.owner" | "unix.uid" => owner = Some(value.to_string()),
            "unix.group" | "unix.gid" => group = Some(value.to_string()),
            _ => {}
        }
    }
    if name.is_empty() {
        return None;
    }
    Some(FileEntry {
        path: join(dir, &name),
        name,
        kind,
        size: if kind == EntryKind::Dir { 0 } else { size },
        modified,
        mode,
        owner,
        group,
        is_link,
        link_target,
    })
}

#[async_trait]
impl RemoteFs for FtpFs {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            chmod: true,
            rename_dirs: true,
            symlinks: false,
            server_side_copy: false,
        }
    }

    async fn home(&self) -> AppResult<String> {
        with_conn!(self, s => s.pwd().await.map_err(ftp_err))
    }

    async fn list(&self, path: &str) -> AppResult<Vec<FileEntry>> {
        if self.mlsd {
            let lines = with_conn!(self, s => s.mlsd(Some(path)).await);
            if let Ok(lines) = lines {
                return Ok(Self::parse_lines(path, &lines, true));
            }
        }
        // LIST output of paths with spaces is unreliable, so change into the directory.
        let lines = with_conn!(self, s => {
            s.cwd(path).await.map_err(ftp_err)?;
            s.list(None).await.map_err(ftp_err)?
        });
        Ok(Self::parse_lines(path, &lines, false))
    }

    async fn stat(&self, path: &str) -> AppResult<Option<FileEntry>> {
        if self.mlst {
            let res = with_conn!(self, s => s.mlst(Some(path)).await);
            return match res {
                Ok(line) => Ok(parse_mlsx(&super::parent(path), &line).map(|mut e| {
                    e.name = super::file_name(path);
                    e.path = path.to_string();
                    e
                })),
                Err(FtpError::UnexpectedResponse(_)) => Ok(None),
                Err(e) => Err(ftp_err(e)),
            };
        }
        let size = with_conn!(self, s => s.size(path).await);
        match size {
            Ok(size) => {
                let modified = with_conn!(self, s => s.mdtm(path).await)
                    .ok()
                    .map(|t| t.and_utc().timestamp_millis());
                Ok(Some(FileEntry {
                    name: super::file_name(path),
                    path: path.to_string(),
                    kind: EntryKind::File,
                    size: size as u64,
                    modified,
                    mode: None,
                    owner: None,
                    group: None,
                    is_link: false,
                    link_target: None,
                }))
            }
            Err(FtpError::UnexpectedResponse(_)) => Ok(None),
            Err(e) => Err(ftp_err(e)),
        }
    }

    async fn mkdir(&self, path: &str) -> AppResult<()> {
        with_conn!(self, s => s.mkdir(path).await.map_err(ftp_err))
    }

    async fn remove_file(&self, path: &str) -> AppResult<()> {
        with_conn!(self, s => s.rm(path).await.map_err(ftp_err))
    }

    async fn remove_dir(&self, path: &str) -> AppResult<()> {
        with_conn!(self, s => s.rmdir(path).await.map_err(ftp_err))
    }

    async fn rename(&self, from: &str, to: &str) -> AppResult<()> {
        with_conn!(self, s => s.rename(from, to).await.map_err(ftp_err))
    }

    async fn chmod(&self, path: &str, mode: u32) -> AppResult<()> {
        with_conn!(self, s => s
            .site(format!("CHMOD {:o} {}", mode & 0o7777, path))
            .await
            .map(|_| ())
            .map_err(ftp_err))
    }

    async fn download(&self, path: &str, sink: Writer<'_>) -> AppResult<()> {
        with_conn!(self, s => {
            let mut stream = s.retr_as_stream(path).await.map_err(ftp_err)?;
            tokio::io::copy(&mut stream, sink).await?;
            stream.finish().await.map_err(ftp_err)?;
        });
        Ok(())
    }

    async fn upload(&self, source: Reader<'_>, path: &str, _size: u64) -> AppResult<()> {
        with_conn!(self, s => {
            let mut stream = s.put_with_stream(path).await.map_err(ftp_err)?;
            tokio::io::copy(source, &mut stream).await?;
            stream.finish().await.map_err(ftp_err)?;
        });
        Ok(())
    }

    async fn is_alive(&self) -> bool {
        with_conn!(self, s => s.noop().await.is_ok())
    }

    async fn close(&self) {
        with_conn!(self, s => {
            let _ = s.quit().await;
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mlsx() {
        let e = parse_mlsx(
            "/d",
            "modify=20260925092833;perm=el;size=4096;type=dir;unix.mode=0o755; my dir",
        )
        .unwrap();
        assert_eq!(e.name, "my dir");
        assert_eq!(e.path, "/d/my dir");
        assert!(e.is_dir());
        assert_eq!(e.mode, Some(0o755));
        let f = parse_mlsx(
            "/",
            "type=file;size=12;modify=20240101120000.123;UNIX.mode=0644; a.txt",
        )
        .unwrap();
        assert_eq!(f.size, 12);
        assert_eq!(f.mode, Some(0o644));
        assert!(f.modified.is_some());
        let l = parse_mlsx("/", "type=OS.unix=slink:/etc/x;size=1; link").unwrap();
        assert!(l.is_link);
        assert_eq!(l.link_target.as_deref(), Some("/etc/x"));
        assert!(parse_mlsx("/", "type=cdir;modify=20240101120000; .").is_none());
        let s = parse_mlsx("/", " type=file;size=1; /full/path/b.bin").unwrap();
        assert_eq!(s.name, "b.bin");
    }
}
