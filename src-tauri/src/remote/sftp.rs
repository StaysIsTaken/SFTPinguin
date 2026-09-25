//! SFTP via russh + russh-sftp (pure Rust, works on desktop and mobile).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use russh::client::{self, Handle, KeyboardInteractiveAuthResponse};
use russh::keys::{self, HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::{FileAttributes, OpenFlags, StatusCode};
use tokio::io::AsyncWriteExt;

use super::{join, RemoteFs, Reader, Writer};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::known_hosts::{HostKey, HostKeyStatus, KnownHosts};
use crate::model::{AuthMethod, Capabilities, ConnectConfig, EntryKind, FileEntry};

const COPY_BUFFER: usize = 256 * 1024;

/// Result of the host key check, captured by the handler.
#[derive(Default)]
struct HostKeyOutcome {
    presented: Option<HostKey>,
    status: Option<&'static str>,
    previous: Option<HostKey>,
}

struct ClientHandler {
    host: String,
    port: u16,
    known: Vec<HostKey>,
    outcome: Arc<Mutex<HostKeyOutcome>>,
}

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let (algorithm, fingerprint) = match key {
            PublicKeyOrCertificate::PublicKey { key, .. } => (
                key.algorithm().to_string(),
                key.fingerprint(HashAlg::Sha256).to_string(),
            ),
            PublicKeyOrCertificate::Certificate(cert) => (
                cert.algorithm().to_string(),
                cert.public_key().fingerprint(HashAlg::Sha256).to_string(),
            ),
        };
        let presented = HostKey {
            host: self.host.clone(),
            port: self.port,
            key_type: algorithm,
            fingerprint: fingerprint.clone(),
            added_at: 0,
        };
        // evaluate against a snapshot of the trust store
        let store = KnownHosts::from_entries(self.known.clone());
        let mut outcome = self.outcome.lock().unwrap();
        outcome.presented = Some(presented);
        match store.check(&self.host, self.port, &fingerprint) {
            HostKeyStatus::Trusted => {
                outcome.status = Some("trusted");
                Ok(true)
            }
            HostKeyStatus::Unknown => {
                outcome.status = Some("unknown");
                Ok(false)
            }
            HostKeyStatus::Changed(prev) => {
                outcome.status = Some("changed");
                outcome.previous = Some(prev);
                Ok(false)
            }
        }
    }
}

pub struct SftpFs {
    handle: Handle<ClientHandler>,
    sftp: SftpSession,
}

fn ssh_err(e: russh::Error) -> AppError {
    AppError::connection(format!("SSH: {e}"))
}

fn sftp_err(e: russh_sftp::client::error::Error) -> AppError {
    use russh_sftp::client::error::Error as E;
    match e {
        E::Status(status) => {
            let code = match status.status_code {
                StatusCode::NoSuchFile => ErrorCode::NotFound,
                StatusCode::PermissionDenied => ErrorCode::PermissionDenied,
                StatusCode::OpUnsupported => ErrorCode::Unsupported,
                StatusCode::NoConnection | StatusCode::ConnectionLost => ErrorCode::Connection,
                _ => ErrorCode::Protocol,
            };
            let msg = if status.error_message.is_empty() {
                format!("{:?}", status.status_code)
            } else {
                status.error_message
            };
            AppError::new(code, msg)
        }
        E::IO(msg) => AppError::new(ErrorCode::Io, msg),
        E::Timeout => AppError::connection("SFTP request timed out"),
        other => AppError::protocol(other.to_string()),
    }
}

fn attrs_to_entry(name: String, path: String, attrs: &FileAttributes) -> FileEntry {
    FileEntry {
        name,
        path,
        kind: if attrs.is_dir() {
            EntryKind::Dir
        } else {
            EntryKind::File
        },
        size: attrs.size.unwrap_or(0),
        modified: attrs.mtime.map(|t| t as i64 * 1000),
        mode: attrs.permissions.map(|p| p & 0o7777),
        owner: attrs
            .user
            .clone()
            .or_else(|| attrs.uid.map(|u| u.to_string())),
        group: attrs
            .group
            .clone()
            .or_else(|| attrs.gid.map(|g| g.to_string())),
        is_link: attrs.is_symlink(),
        link_target: None,
    }
}

impl SftpFs {
    pub async fn connect(cfg: &ConnectConfig, known_hosts: &KnownHosts) -> AppResult<Self> {
        let site = &cfg.site;
        let host = site.host.trim().to_string();
        let port = site.port();

        let config = Arc::new(client::Config {
            inactivity_timeout: None,
            keepalive_interval: Some(Duration::from_secs(20)),
            keepalive_max: 3,
            ..Default::default()
        });
        let outcome = Arc::new(Mutex::new(HostKeyOutcome::default()));
        let handler = ClientHandler {
            host: host.clone(),
            port,
            known: known_hosts.list(),
            outcome: outcome.clone(),
        };

        let connect_result = client::connect(config, (host.as_str(), port), handler).await;
        let mut handle = match connect_result {
            Ok(h) => h,
            Err(e) => {
                let o = outcome.lock().unwrap();
                if let (Some(status), Some(presented)) = (o.status, o.presented.clone()) {
                    if status == "unknown" {
                        return Err(AppError::new(
                            ErrorCode::HostKeyUnknown,
                            "The authenticity of this host can't be established",
                        )
                        .with_details(serde_json::json!({ "presented": presented })));
                    }
                    if status == "changed" {
                        return Err(AppError::new(
                            ErrorCode::HostKeyChanged,
                            "WARNING: the host key of this server has changed",
                        )
                        .with_details(serde_json::json!({
                            "presented": presented,
                            "previous": o.previous,
                        })));
                    }
                }
                return Err(match e {
                    russh::Error::IO(io) => AppError::connection(io.to_string()),
                    other => ssh_err(other),
                });
            }
        };

        authenticate(&mut handle, cfg).await?;

        let channel = handle.channel_open_session().await.map_err(ssh_err)?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(ssh_err)?;
        let sftp = SftpSession::new(channel.into_stream())
            .await
            .map_err(sftp_err)?;
        sftp.set_timeout(60);
        Ok(Self { handle, sftp })
    }
}

async fn authenticate(handle: &mut Handle<ClientHandler>, cfg: &ConnectConfig) -> AppResult<()> {
    let site = &cfg.site;
    let user = if site.username.is_empty() {
        "root".to_string()
    } else {
        site.username.clone()
    };

    let auth_failed = || AppError::new(ErrorCode::AuthFailed, "Authentication failed");

    match site.auth {
        AuthMethod::Password | AuthMethod::Anonymous => {
            let Some(password) = cfg.password.clone() else {
                return Err(AppError::new(
                    ErrorCode::PasswordRequired,
                    "A password is required",
                ));
            };
            let res = handle
                .authenticate_password(&user, &password)
                .await
                .map_err(ssh_err)?;
            if res.success() {
                return Ok(());
            }
            // Many servers only offer keyboard-interactive for passwords.
            let mut resp = handle
                .authenticate_keyboard_interactive_start(&user, None)
                .await
                .map_err(ssh_err)?;
            for _ in 0..5 {
                match resp {
                    KeyboardInteractiveAuthResponse::Success => return Ok(()),
                    KeyboardInteractiveAuthResponse::Failure { .. } => return Err(auth_failed()),
                    KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                        let answers = prompts.iter().map(|_| password.clone()).collect();
                        resp = handle
                            .authenticate_keyboard_interactive_respond(answers)
                            .await
                            .map_err(ssh_err)?;
                    }
                }
            }
            Err(auth_failed())
        }
        AuthMethod::Key => {
            let key = if let Some(data) = cfg.key_data.as_deref().filter(|d| !d.trim().is_empty())
            {
                keys::decode_secret_key(data, cfg.passphrase.as_deref())
            } else if let Some(path) = site.key_path.as_deref().filter(|p| !p.is_empty()) {
                keys::load_secret_key(expand_tilde(path), cfg.passphrase.as_deref())
            } else {
                return Err(AppError::invalid("No private key configured"));
            };
            let key = match key {
                Ok(k) => k,
                Err(keys::Error::KeyIsEncrypted) => {
                    return Err(AppError::new(
                        ErrorCode::PassphraseRequired,
                        "The private key is protected by a passphrase",
                    ))
                }
                Err(e) if cfg.passphrase.is_some() && is_decrypt_error(&e) => {
                    return Err(AppError::new(
                        ErrorCode::PassphraseRequired,
                        "Wrong passphrase for the private key",
                    ))
                }
                Err(e) => return Err(AppError::invalid(format!("Could not load private key: {e}"))),
            };
            let hash = handle
                .best_supported_rsa_hash()
                .await
                .map_err(ssh_err)?
                .flatten();
            let res = handle
                .authenticate_publickey(&user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
                .await
                .map_err(ssh_err)?;
            if res.success() {
                Ok(())
            } else {
                Err(auth_failed())
            }
        }
        AuthMethod::Agent => agent_auth(handle, &user).await,
    }
}

fn is_decrypt_error(e: &keys::Error) -> bool {
    let s = e.to_string().to_lowercase();
    s.contains("decrypt") || s.contains("passphrase") || s.contains("crypto")
}

fn expand_tilde(path: &str) -> std::path::PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
            return std::path::PathBuf::from(home).join(rest);
        }
    }
    std::path::PathBuf::from(path)
}

#[cfg(all(unix, not(any(target_os = "android", target_os = "ios"))))]
async fn agent_auth(handle: &mut Handle<ClientHandler>, user: &str) -> AppResult<()> {
    let mut agent = keys::agent::client::AgentClient::connect_env()
        .await
        .map_err(|e| AppError::connection(format!("SSH agent not available: {e}")))?;
    run_agent_auth(handle, user, &mut agent).await
}

#[cfg(windows)]
async fn agent_auth(handle: &mut Handle<ClientHandler>, user: &str) -> AppResult<()> {
    // Prefer the OpenSSH for Windows agent, fall back to Pageant.
    match keys::agent::client::AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent").await
    {
        Ok(mut agent) => run_agent_auth(handle, user, &mut agent).await,
        Err(_) => {
            let mut agent = keys::agent::client::AgentClient::connect_pageant()
                .await
                .map_err(|e| AppError::connection(format!("SSH agent not available: {e}")))?;
            run_agent_auth(handle, user, &mut agent).await
        }
    }
}

#[cfg(any(target_os = "android", target_os = "ios"))]
async fn agent_auth(_handle: &mut Handle<ClientHandler>, _user: &str) -> AppResult<()> {
    Err(AppError::unsupported(
        "SSH agent authentication is not available on this platform",
    ))
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
async fn run_agent_auth<S>(
    handle: &mut Handle<ClientHandler>,
    user: &str,
    agent: &mut keys::agent::client::AgentClient<S>,
) -> AppResult<()>
where
    S: keys::agent::client::AgentStream + Send + Unpin + 'static,
{
    let identities = agent
        .request_identities()
        .await
        .map_err(|e| AppError::connection(format!("SSH agent: {e}")))?;
    if identities.is_empty() {
        return Err(AppError::new(
            ErrorCode::AuthFailed,
            "The SSH agent has no keys loaded",
        ));
    }
    let hash = handle
        .best_supported_rsa_hash()
        .await
        .map_err(ssh_err)?
        .flatten();
    for identity in identities {
        let keys::agent::AgentIdentity::PublicKey { key, .. } = identity else {
            continue;
        };
        let res = handle
            .authenticate_publickey_with(user, key, hash, agent)
            .await
            .map_err(|e| AppError::connection(format!("SSH agent: {e}")))?;
        if res.success() {
            return Ok(());
        }
    }
    Err(AppError::new(
        ErrorCode::AuthFailed,
        "None of the SSH agent keys were accepted",
    ))
}

#[async_trait]
impl RemoteFs for SftpFs {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            chmod: true,
            rename_dirs: true,
            symlinks: true,
            server_side_copy: false,
        }
    }

    async fn home(&self) -> AppResult<String> {
        self.sftp.canonicalize(".").await.map_err(sftp_err)
    }

    async fn list(&self, path: &str) -> AppResult<Vec<FileEntry>> {
        let dir = self.sftp.read_dir(path).await.map_err(sftp_err)?;
        let mut out = Vec::new();
        for entry in dir {
            let name = entry.file_name();
            if name == "." || name == ".." {
                continue;
            }
            let full = join(path, &name);
            let attrs = entry.metadata();
            let mut fe = attrs_to_entry(name, full.clone(), &attrs);
            if fe.is_link {
                // follow the link to find out whether it points to a directory
                if let Ok(target) = self.sftp.metadata(full.clone()).await {
                    fe.kind = if target.is_dir() {
                        EntryKind::Dir
                    } else {
                        EntryKind::File
                    };
                    if !target.is_dir() {
                        fe.size = target.size.unwrap_or(fe.size);
                    }
                }
                fe.link_target = self.sftp.read_link(full).await.ok();
            }
            out.push(fe);
        }
        Ok(out)
    }

    async fn stat(&self, path: &str) -> AppResult<Option<FileEntry>> {
        match self.sftp.metadata(path).await {
            Ok(attrs) => Ok(Some(attrs_to_entry(
                super::file_name(path),
                path.to_string(),
                &attrs,
            ))),
            Err(e) => {
                let err = sftp_err(e);
                if err.code == ErrorCode::NotFound {
                    Ok(None)
                } else {
                    Err(err)
                }
            }
        }
    }

    async fn mkdir(&self, path: &str) -> AppResult<()> {
        self.sftp.create_dir(path).await.map_err(sftp_err)
    }

    async fn remove_file(&self, path: &str) -> AppResult<()> {
        self.sftp.remove_file(path).await.map_err(sftp_err)
    }

    async fn remove_dir(&self, path: &str) -> AppResult<()> {
        self.sftp.remove_dir(path).await.map_err(sftp_err)
    }

    async fn rename(&self, from: &str, to: &str) -> AppResult<()> {
        self.sftp.rename(from, to).await.map_err(sftp_err)
    }

    async fn chmod(&self, path: &str, mode: u32) -> AppResult<()> {
        let mut attrs = FileAttributes::empty();
        attrs.permissions = Some(mode & 0o7777);
        self.sftp.set_metadata(path, attrs).await.map_err(sftp_err)
    }

    async fn set_mtime(&self, path: &str, mtime: i64) -> AppResult<()> {
        let mut attrs = FileAttributes::empty();
        attrs.mtime = Some(mtime as u32);
        attrs.atime = Some(mtime as u32);
        self.sftp.set_metadata(path, attrs).await.map_err(sftp_err)
    }

    async fn download(&self, path: &str, sink: Writer<'_>) -> AppResult<()> {
        let file = self.sftp.open(path).await.map_err(sftp_err)?;
        let mut reader = tokio::io::BufReader::with_capacity(COPY_BUFFER, file);
        tokio::io::copy_buf(&mut reader, sink).await?;
        Ok(())
    }

    async fn upload(&self, source: Reader<'_>, path: &str, _size: u64) -> AppResult<()> {
        let mut file = self
            .sftp
            .open_with_flags(
                path,
                OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::WRITE,
            )
            .await
            .map_err(sftp_err)?;
        let mut reader = tokio::io::BufReader::with_capacity(COPY_BUFFER, source);
        tokio::io::copy_buf(&mut reader, &mut file).await?;
        file.flush().await?;
        file.shutdown().await?;
        Ok(())
    }

    async fn is_alive(&self) -> bool {
        !self.handle.is_closed()
    }

    async fn close(&self) {
        let _ = self.sftp.close().await;
        let _ = self
            .handle
            .disconnect(russh::Disconnect::ByApplication, "", "en")
            .await;
    }
}
