//! FTP, FTPS (explicit, AUTH TLS) and FTPS (implicit) via suppaftp.

use std::time::{Duration, UNIX_EPOCH};

use async_trait::async_trait;
use suppaftp::list::{File as FtpFile, ListParser, PosixPexQuery};
use suppaftp::tokio::{AsyncFtpStream, AsyncRustlsConnector, AsyncRustlsFtpStream};
use suppaftp::types::FileType;
use suppaftp::{FtpError, Status};
use tokio::sync::Mutex;

use super::{join, RemoteFs, Reader, Writer};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::model::{AuthMethod, Capabilities, ConnectConfig, EntryKind, FileEntry, Protocol};

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
                Status::CommandNotImplemented | Status::NotImplemented | Status::NotImplementedParameter => {
                    ErrorCode::Unsupported
                }
                _ => ErrorCode::Protocol,
            };
            AppError::new(code, format!("{} {}", resp.status.code(), text.trim()))
        }
        FtpError::DataConnectionAlreadyOpen => {
            AppError::connection("A data connection is already open")
        }
        other => AppError::protocol(other.to_string()),
    }
}

impl FtpFs {
    pub async fn connect(cfg: &ConnectConfig) -> AppResult<Self> {
        let site = &cfg.site;
        let host = site.host.trim();
        let addr = (host, site.port());

        let tls = || -> AppResult<AsyncRustlsConnector> {
            let config = crate::tls::client_config(site.insecure_tls)?;
            Ok(AsyncRustlsConnector::from(
                suppaftp::tokio_rustls::TlsConnector::from(config),
            ))
        };

        let mut conn = match site.protocol {
            Protocol::Ftp => Conn::Plain(AsyncFtpStream::connect(addr).await.map_err(ftp_err)?),
            Protocol::Ftps => {
                let plain = AsyncRustlsFtpStream::connect(addr).await.map_err(ftp_err)?;
                Conn::Tls(plain.into_secure(tls()?, host).await.map_err(ftp_err)?)
            }
            Protocol::FtpsImplicit => Conn::Tls(
                AsyncRustlsFtpStream::connect_secure_implicit(addr, tls()?, host)
                    .await
                    .map_err(ftp_err)?,
            ),
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
                $s.feat().await.unwrap_or_default()
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
        for (i, who) in [PosixPexQuery::Owner, PosixPexQuery::Group, PosixPexQuery::Others]
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
                let parsed = if mlsd {
                    ListParser::parse_mlsd(line)
                } else {
                    ListParser::parse_posix(line).or_else(|_| ListParser::parse_dos(line))
                };
                parsed.ok()
            })
            .filter(|f| f.name() != "." && f.name() != ".." && !f.name().is_empty())
            .map(|f| Self::to_entry(dir, &f))
            .collect()
    }
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
                Ok(line) => Ok(ListParser::parse_mlst(&line)
                    .ok()
                    .map(|f| {
                        let mut e = Self::to_entry(&super::parent(path), &f);
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
