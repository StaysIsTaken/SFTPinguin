use serde::Serialize;

/// Machine readable error codes. The frontend uses them to decide how to react
/// (e.g. ask for a password, show a host key dialog, ...).
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Connection,
    AuthFailed,
    PasswordRequired,
    PassphraseRequired,
    HostKeyUnknown,
    HostKeyChanged,
    /// TLS certificate not publicly trusted (self-signed, expired, wrong name, …)
    CertUntrusted,
    /// A pinned TLS certificate was replaced by a different untrusted one
    CertChanged,
    /// The FTP server does not support encryption (AUTH TLS)
    TlsNotSupported,
    /// Unencrypted connection that the user has not allowed
    InsecureConnection,
    NotFound,
    PermissionDenied,
    AlreadyExists,
    Unsupported,
    Cancelled,
    SessionClosed,
    InvalidInput,
    Protocol,
    SecretStore,
    Io,
}

#[derive(Debug, Clone, Serialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{message}")]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

pub type AppResult<T> = Result<T, AppError>;

impl AppError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn connection(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Connection, message)
    }
    pub fn protocol(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Protocol, message)
    }
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Unsupported, message)
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidInput, message)
    }
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, message)
    }
    pub fn cancelled() -> Self {
        Self::new(ErrorCode::Cancelled, "Cancelled")
    }

    /// Errors after which a pooled connection should not be reused.
    pub fn is_fatal_for_connection(&self) -> bool {
        matches!(
            self.code,
            ErrorCode::Connection | ErrorCode::SessionClosed | ErrorCode::Cancelled | ErrorCode::Io
        )
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        use std::io::ErrorKind as K;
        let code = match e.kind() {
            K::NotFound => ErrorCode::NotFound,
            K::PermissionDenied => ErrorCode::PermissionDenied,
            K::AlreadyExists => ErrorCode::AlreadyExists,
            K::Interrupted => ErrorCode::Cancelled,
            K::ConnectionRefused
            | K::ConnectionReset
            | K::ConnectionAborted
            | K::NotConnected
            | K::BrokenPipe
            | K::TimedOut
            | K::UnexpectedEof => ErrorCode::Connection,
            _ => ErrorCode::Io,
        };
        if code == ErrorCode::Cancelled {
            return AppError::cancelled();
        }
        AppError::new(code, e.to_string())
    }
}

impl From<tauri::Error> for AppError {
    fn from(e: tauri::Error) -> Self {
        AppError::new(ErrorCode::Io, e.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::new(ErrorCode::Io, format!("JSON: {e}"))
    }
}
