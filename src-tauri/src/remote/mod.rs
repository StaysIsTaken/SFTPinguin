//! Protocol abstraction. Every protocol implements [`RemoteFs`], the rest of the
//! application (sessions, transfer queue, editor) only talks to this trait.

mod ftp;
mod s3;
mod sftp;
mod webdav;

#[cfg(test)]
mod integration_tests;

use std::sync::Arc;

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::error::{AppError, AppResult};
use crate::known_hosts::KnownHosts;
use crate::model::{Capabilities, ConnectConfig, FileEntry, Protocol};

pub type Reader<'a> = &'a mut (dyn AsyncRead + Send + Unpin);
pub type Writer<'a> = &'a mut (dyn AsyncWrite + Send + Unpin);

#[async_trait]
pub trait RemoteFs: Send + Sync {
    fn capabilities(&self) -> Capabilities;

    /// Initial directory after login.
    async fn home(&self) -> AppResult<String>;

    async fn list(&self, path: &str) -> AppResult<Vec<FileEntry>>;

    /// Returns `None` if the path does not exist.
    async fn stat(&self, path: &str) -> AppResult<Option<FileEntry>>;

    async fn mkdir(&self, path: &str) -> AppResult<()>;

    async fn remove_file(&self, path: &str) -> AppResult<()>;

    /// Removes an (empty) directory.
    async fn remove_dir(&self, path: &str) -> AppResult<()>;

    async fn rename(&self, from: &str, to: &str) -> AppResult<()>;

    async fn chmod(&self, _path: &str, _mode: u32) -> AppResult<()> {
        Err(AppError::unsupported(
            "Changing permissions is not supported by this protocol",
        ))
    }

    /// Sets the modification time (seconds since epoch). Best effort.
    async fn set_mtime(&self, _path: &str, _mtime: i64) -> AppResult<()> {
        Ok(())
    }

    /// Streams the remote file into `sink`.
    async fn download(&self, path: &str, sink: Writer<'_>) -> AppResult<()>;

    /// Streams `source` (of `size` bytes) into the remote file, replacing it.
    async fn upload(&self, source: Reader<'_>, path: &str, size: u64) -> AppResult<()>;

    /// Cheap liveness check (used before reusing pooled connections).
    async fn is_alive(&self) -> bool;

    async fn close(&self);
}

pub type RemoteHandle = Arc<dyn RemoteFs>;

/// Opens a new connection for the given configuration.
pub async fn connect(cfg: &ConnectConfig, known_hosts: &KnownHosts) -> AppResult<RemoteHandle> {
    if cfg.site.host.trim().is_empty() && cfg.site.protocol != Protocol::S3 {
        return Err(AppError::invalid("No host given"));
    }
    let timeout = std::time::Duration::from_secs(cfg.site.timeout.clamp(3, 300));
    let fut = async {
        let handle: RemoteHandle = match cfg.site.protocol {
            Protocol::Sftp => Arc::new(sftp::SftpFs::connect(cfg, known_hosts).await?),
            Protocol::Ftp | Protocol::Ftps | Protocol::FtpsImplicit => {
                Arc::new(ftp::FtpFs::connect(cfg).await?)
            }
            Protocol::Webdav | Protocol::Webdavs => Arc::new(webdav::WebDavFs::connect(cfg).await?),
            Protocol::S3 => Arc::new(s3::S3Fs::connect(cfg).await?),
        };
        Ok::<_, AppError>(handle)
    };
    match tokio::time::timeout(timeout, fut).await {
        Ok(r) => r,
        Err(_) => Err(AppError::connection(format!(
            "Connection timed out after {} s",
            timeout.as_secs()
        ))),
    }
}

// ---------------------------------------------------------------------------
// Remote path helpers (remote paths always use '/')
// ---------------------------------------------------------------------------

pub fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        return name.to_string();
    }
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

pub fn parent(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) => "/".to_string(),
        Some(i) => trimmed[..i].to_string(),
        None => String::new(),
    }
}

pub fn file_name(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    trimmed.rsplit('/').next().unwrap_or(trimmed).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_helpers() {
        assert_eq!(join("/", "a"), "/a");
        assert_eq!(join("/a", "b"), "/a/b");
        assert_eq!(parent("/a/b"), "/a");
        assert_eq!(parent("/a"), "/");
        assert_eq!(parent("/a/b/"), "/a");
        assert_eq!(file_name("/a/b.txt"), "b.txt");
        assert_eq!(file_name("/a/dir/"), "dir");
    }
}
