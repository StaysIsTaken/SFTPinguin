//! Amazon S3 and S3 compatible storage (MinIO, Cloudflare R2, Wasabi, Backblaze B2,
//! Hetzner, DigitalOcean Spaces, ...).
//!
//! Paths are mapped as `/<bucket>/<key>`. The root lists all buckets.

use async_trait::async_trait;
use s3::creds::Credentials;
use s3::error::S3Error;
use s3::{Bucket, Region};

use super::{RemoteFs, Reader, Writer};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::model::{Capabilities, ConnectConfig, EntryKind, FileEntry};

pub struct S3Fs {
    region: Region,
    credentials: Credentials,
    path_style: bool,
    home: String,
}

fn s3_err(e: S3Error) -> AppError {
    match e {
        S3Error::HttpFailWithBody(code, body) => {
            let msg = extract_message(&body).unwrap_or_else(|| format!("HTTP {code}"));
            let code = match code {
                401 | 403 => ErrorCode::PermissionDenied,
                404 => ErrorCode::NotFound,
                _ => ErrorCode::Protocol,
            };
            AppError::new(code, msg)
        }
        S3Error::Io(io) => io.into(),
        other => AppError::protocol(other.to_string()),
    }
}

fn extract_message(body: &str) -> Option<String> {
    let start = body.find("<Message>")? + "<Message>".len();
    let end = body[start..].find("</Message>")? + start;
    Some(body[start..end].to_string())
}

fn parse_time(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .or_else(|| chrono::DateTime::parse_from_rfc2822(s).ok())
        .map(|d| d.timestamp_millis())
}

/// Splits `/bucket/some/key` into (`bucket`, `some/key`).
fn split(path: &str) -> (String, String) {
    let trimmed = path.trim_start_matches('/');
    match trimmed.split_once('/') {
        Some((b, k)) => (b.to_string(), k.to_string()),
        None => (trimmed.to_string(), String::new()),
    }
}

impl S3Fs {
    pub async fn connect(cfg: &ConnectConfig) -> AppResult<Self> {
        let site = &cfg.site;
        let region_name = if site.region.trim().is_empty() {
            "us-east-1".to_string()
        } else {
            site.region.trim().to_string()
        };
        // Endpoint may be given in "endpoint" or (for convenience) in "host".
        let mut endpoint = site.endpoint.trim().to_string();
        if endpoint.is_empty() && !site.host.trim().is_empty() {
            endpoint = site.host.trim().to_string();
        }
        let region = if endpoint.is_empty() || endpoint.ends_with("amazonaws.com") {
            region_name
                .parse::<Region>()
                .map_err(|e| AppError::invalid(format!("Invalid region: {e}")))?
        } else {
            if !endpoint.starts_with("http://") && !endpoint.starts_with("https://") {
                endpoint = format!("https://{endpoint}");
            }
            if let Some(port) = site.port.filter(|p| *p != 0 && *p != 443) {
                // only append the port when the endpoint has none
                let without_scheme = endpoint.split("://").nth(1).unwrap_or("");
                if !without_scheme.contains(':') {
                    endpoint = format!("{}:{port}", endpoint.trim_end_matches('/'));
                }
            }
            Region::Custom {
                region: region_name,
                endpoint: endpoint.trim_end_matches('/').to_string(),
            }
        };

        let Some(secret) = cfg.password.clone() else {
            return Err(AppError::new(
                ErrorCode::PasswordRequired,
                "The secret access key is required",
            ));
        };
        let credentials = Credentials::new(Some(&site.username), Some(&secret), None, None, None)
            .map_err(|e| AppError::invalid(format!("Credentials: {e}")))?;

        let mut home = site.remote_path.trim().to_string();
        if home.is_empty() {
            home = "/".into();
        }
        if !home.starts_with('/') {
            home.insert(0, '/');
        }

        let fs = Self {
            region,
            credentials,
            path_style: site.path_style || !endpoint.is_empty(),
            home,
        };

        // Validate the credentials.
        let (bucket, _) = split(&fs.home);
        if bucket.is_empty() {
            Bucket::list_buckets(fs.region.clone(), fs.credentials.clone())
                .await
                .map_err(|e| match s3_err(e) {
                    e if e.code == ErrorCode::PermissionDenied => AppError::new(
                        ErrorCode::AuthFailed,
                        format!(
                            "{} – if your key may not list buckets, enter the bucket as remote path",
                            e.message
                        ),
                    ),
                    e => e,
                })?;
        } else {
            fs.bucket(&bucket)?
                .list_page(String::new(), Some("/".into()), None, None, Some(1))
                .await
                .map_err(s3_err)?;
        }
        Ok(fs)
    }

    fn bucket(&self, name: &str) -> AppResult<Box<Bucket>> {
        let bucket = Bucket::new(name, self.region.clone(), self.credentials.clone())
            .map_err(s3_err)?;
        Ok(if self.path_style {
            bucket.with_path_style()
        } else {
            bucket
        })
    }

    fn require_key(path: &str) -> AppResult<(String, String)> {
        let (b, k) = split(path);
        if b.is_empty() || k.is_empty() {
            return Err(AppError::unsupported(
                "This operation is not possible on buckets",
            ));
        }
        Ok((b, k))
    }
}

#[async_trait]
impl RemoteFs for S3Fs {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            chmod: false,
            rename_dirs: false,
            symlinks: false,
            server_side_copy: true,
        }
    }

    async fn home(&self) -> AppResult<String> {
        Ok(self.home.clone())
    }

    async fn list(&self, path: &str) -> AppResult<Vec<FileEntry>> {
        let (bucket_name, key) = split(path);
        if bucket_name.is_empty() {
            let resp = Bucket::list_buckets(self.region.clone(), self.credentials.clone())
                .await
                .map_err(s3_err)?;
            return Ok(resp
                .buckets
                .bucket
                .into_iter()
                .map(|b| FileEntry {
                    path: format!("/{}", b.name),
                    modified: parse_time(&b.creation_date),
                    name: b.name,
                    kind: EntryKind::Dir,
                    size: 0,
                    mode: None,
                    owner: None,
                    group: None,
                    is_link: false,
                    link_target: None,
                })
                .collect());
        }
        let bucket = self.bucket(&bucket_name)?;
        let prefix = if key.is_empty() {
            String::new()
        } else {
            format!("{}/", key.trim_end_matches('/'))
        };
        let pages = bucket
            .list(prefix.clone(), Some("/".to_string()))
            .await
            .map_err(s3_err)?;
        let base = path.trim_end_matches('/');
        let mut out = Vec::new();
        for page in pages {
            for p in page.common_prefixes.unwrap_or_default() {
                let name = p
                    .prefix
                    .strip_prefix(&prefix)
                    .unwrap_or(&p.prefix)
                    .trim_end_matches('/')
                    .to_string();
                if name.is_empty() {
                    continue;
                }
                out.push(FileEntry {
                    path: format!("{base}/{name}"),
                    name,
                    kind: EntryKind::Dir,
                    size: 0,
                    modified: None,
                    mode: None,
                    owner: None,
                    group: None,
                    is_link: false,
                    link_target: None,
                });
            }
            for obj in page.contents {
                let name = obj.key.strip_prefix(&prefix).unwrap_or(&obj.key).to_string();
                if name.is_empty() || name.ends_with('/') {
                    continue; // directory marker object
                }
                out.push(FileEntry {
                    path: format!("{base}/{name}"),
                    name,
                    kind: EntryKind::File,
                    size: obj.size,
                    modified: parse_time(&obj.last_modified),
                    mode: None,
                    owner: obj.owner.and_then(|o| o.display_name),
                    group: None,
                    is_link: false,
                    link_target: None,
                });
            }
        }
        Ok(out)
    }

    async fn stat(&self, path: &str) -> AppResult<Option<FileEntry>> {
        let (bucket_name, key) = split(path);
        if bucket_name.is_empty() || key.is_empty() {
            return Ok(None);
        }
        let bucket = self.bucket(&bucket_name)?;
        let head = bucket.head_object(&key).await;
        match head {
            Ok((h, 200)) => Ok(Some(FileEntry {
                name: super::file_name(path),
                path: path.to_string(),
                kind: EntryKind::File,
                size: h.content_length.unwrap_or(0).max(0) as u64,
                modified: h.last_modified.as_deref().and_then(parse_time),
                mode: None,
                owner: None,
                group: None,
                is_link: false,
                link_target: None,
            })),
            Ok(_) | Err(S3Error::HttpFailWithBody(404, _)) => {
                // maybe a "directory"
                let (page, _) = bucket
                    .list_page(
                        format!("{}/", key.trim_end_matches('/')),
                        Some("/".into()),
                        None,
                        None,
                        Some(1),
                    )
                    .await
                    .map_err(s3_err)?;
                let exists = !page.contents.is_empty()
                    || page.common_prefixes.is_some_and(|p| !p.is_empty());
                Ok(exists.then(|| FileEntry {
                    name: super::file_name(path),
                    path: path.to_string(),
                    kind: EntryKind::Dir,
                    size: 0,
                    modified: None,
                    mode: None,
                    owner: None,
                    group: None,
                    is_link: false,
                    link_target: None,
                }))
            }
            Err(e) => Err(s3_err(e)),
        }
    }

    async fn mkdir(&self, path: &str) -> AppResult<()> {
        let (b, k) = split(path);
        if k.is_empty() {
            return Err(AppError::unsupported(
                "Creating buckets is not supported – please create it in your provider's console",
            ));
        }
        self.bucket(&b)?
            .put_object(format!("{}/", k.trim_end_matches('/')), &[])
            .await
            .map_err(s3_err)?;
        Ok(())
    }

    async fn remove_file(&self, path: &str) -> AppResult<()> {
        let (b, k) = Self::require_key(path)?;
        self.bucket(&b)?.delete_object(k).await.map_err(s3_err)?;
        Ok(())
    }

    async fn remove_dir(&self, path: &str) -> AppResult<()> {
        let (b, k) = Self::require_key(path)?;
        match self
            .bucket(&b)?
            .delete_object(format!("{}/", k.trim_end_matches('/')))
            .await
        {
            Ok(_) | Err(S3Error::HttpFailWithBody(404, _)) => Ok(()),
            Err(e) => Err(s3_err(e)),
        }
    }

    async fn rename(&self, from: &str, to: &str) -> AppResult<()> {
        let (fb, fk) = Self::require_key(from)?;
        let (tb, tk) = Self::require_key(to)?;
        if fb != tb {
            return Err(AppError::unsupported("Moving between buckets is not supported"));
        }
        if let Some(entry) = self.stat(from).await? {
            if entry.is_dir() {
                return Err(AppError::unsupported(
                    "Renaming folders is not supported on S3",
                ));
            }
        }
        let bucket = self.bucket(&fb)?;
        bucket.copy_object_internal(&fk, &tk).await.map_err(s3_err)?;
        bucket.delete_object(&fk).await.map_err(s3_err)?;
        Ok(())
    }

    async fn download(&self, path: &str, sink: Writer<'_>) -> AppResult<()> {
        let (b, k) = Self::require_key(path)?;
        self.bucket(&b)?
            .get_object_to_writer(k, sink)
            .await
            .map_err(s3_err)?;
        Ok(())
    }

    async fn upload(&self, source: Reader<'_>, path: &str, _size: u64) -> AppResult<()> {
        let (b, k) = Self::require_key(path)?;
        self.bucket(&b)?
            .put_object_stream(source, k)
            .await
            .map_err(s3_err)?;
        Ok(())
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
    fn split_paths() {
        assert_eq!(split("/"), ("".into(), "".into()));
        assert_eq!(split("/b"), ("b".into(), "".into()));
        assert_eq!(split("/b/a/c.txt"), ("b".into(), "a/c.txt".into()));
        assert_eq!(
            extract_message("<Error><Message>Access Denied</Message></Error>").as_deref(),
            Some("Access Denied")
        );
    }
}
