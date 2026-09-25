use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum Protocol {
    Sftp,
    Ftp,
    /// FTP with explicit TLS (AUTH TLS)
    Ftps,
    /// FTP with implicit TLS (legacy, port 990)
    FtpsImplicit,
    Webdav,
    Webdavs,
    S3,
}

impl Protocol {
    pub fn default_port(self) -> u16 {
        match self {
            Protocol::Sftp => 22,
            Protocol::Ftp | Protocol::Ftps => 21,
            Protocol::FtpsImplicit => 990,
            Protocol::Webdav => 80,
            Protocol::Webdavs | Protocol::S3 => 443,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Protocol::Sftp => "SFTP",
            Protocol::Ftp => "FTP",
            Protocol::Ftps => "FTPS",
            Protocol::FtpsImplicit => "FTPS (implicit)",
            Protocol::Webdav => "WebDAV",
            Protocol::Webdavs => "WebDAV (HTTPS)",
            Protocol::S3 => "S3",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum AuthMethod {
    #[default]
    Password,
    /// SSH private key (file on disk or pasted key stored in the secret store)
    Key,
    /// SSH agent (ssh-agent / Pageant)
    Agent,
    /// Anonymous FTP / no credentials
    Anonymous,
}

/// A saved server entry of the site manager.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Site {
    pub id: String,
    pub name: String,
    pub protocol: Protocol,
    pub host: String,
    pub port: Option<u16>,
    pub username: String,
    pub auth: AuthMethod,
    /// Path to a private key file (SFTP)
    pub key_path: Option<String>,
    /// Whether the password / secret should be kept in the secret store
    pub save_password: bool,
    pub remote_path: String,
    pub local_path: String,
    /// Optional folder / group name in the site manager
    pub group: String,
    pub color: String,
    pub notes: String,
    pub favorite: bool,
    /// FTP: passive mode (recommended)
    pub passive: bool,
    /// TLS: accept invalid / self-signed certificates
    pub insecure_tls: bool,
    /// S3: region (e.g. eu-central-1)
    pub region: String,
    /// S3: custom endpoint (MinIO, Wasabi, R2, Hetzner, ...). Empty = AWS
    pub endpoint: String,
    /// S3: use path style addressing
    pub path_style: bool,
    /// Connection timeout in seconds
    pub timeout: u64,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    /// Maintained by the backend: a password / secret key is in the secret store
    pub has_password: bool,
    /// Maintained by the backend: a pasted private key is in the secret store
    pub has_key_data: bool,
}

impl Default for Site {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            protocol: Protocol::Sftp,
            host: String::new(),
            port: None,
            username: String::new(),
            auth: AuthMethod::Password,
            key_path: None,
            save_password: true,
            remote_path: String::new(),
            local_path: String::new(),
            group: String::new(),
            color: String::new(),
            notes: String::new(),
            favorite: false,
            passive: true,
            insecure_tls: false,
            region: String::new(),
            endpoint: String::new(),
            path_style: false,
            timeout: 20,
            created_at: 0,
            last_used_at: None,
            has_password: false,
            has_key_data: false,
        }
    }
}

impl Site {
    pub fn port(&self) -> u16 {
        self.port.filter(|p| *p != 0).unwrap_or(self.protocol.default_port())
    }

    pub fn display_name(&self) -> String {
        if !self.name.trim().is_empty() {
            self.name.clone()
        } else if self.username.is_empty() {
            self.host.clone()
        } else {
            format!("{}@{}", self.username, self.host)
        }
    }
}

/// Everything needed to open a connection: the site plus the resolved secrets.
#[derive(Clone)]
pub struct ConnectConfig {
    pub site: Site,
    pub password: Option<String>,
    pub passphrase: Option<String>,
    /// Private key in OpenSSH / PEM format (pasted by the user)
    pub key_data: Option<String>,
}

impl std::fmt::Debug for ConnectConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectConfig")
            .field("site", &self.site.id)
            .field("host", &self.site.host)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    File,
    Dir,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub kind: EntryKind,
    pub size: u64,
    /// Unix timestamp in milliseconds
    pub modified: Option<i64>,
    /// Unix permission bits (lower 12 bits)
    pub mode: Option<u32>,
    pub owner: Option<String>,
    pub group: Option<String>,
    pub is_link: bool,
    pub link_target: Option<String>,
}

impl FileEntry {
    pub fn is_dir(&self) -> bool {
        self.kind == EntryKind::Dir
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub chmod: bool,
    pub rename_dirs: bool,
    pub symlinks: bool,
    pub server_side_copy: bool,
}
