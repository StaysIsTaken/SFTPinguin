//! TLS for FTPS and WebDAV (HTTPS).
//!
//! Certificates are validated against the Mozilla root store. Certificates that are not
//! publicly trusted (self-signed NAS / router certificates are common in local networks)
//! are never accepted blindly: the user is shown the certificate and its SHA-256
//! fingerprint and can *pin* exactly this certificate for host:port. A different,
//! untrusted certificate for a pinned host is reported as a possible man-in-the-middle
//! attack.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, DigitallySignedStruct, SignatureScheme};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult, ErrorCode};
use crate::storage;

static TLS12_ONLY: &[&rustls::SupportedProtocolVersion] = &[&rustls::version::TLS12];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CertInfo {
    pub host: String,
    pub port: u16,
    /// SHA-256 of the DER encoded certificate, `AB:CD:…`
    pub fingerprint: String,
    pub subject: String,
    pub issuer: String,
    /// Unix timestamps in milliseconds
    pub not_before: Option<i64>,
    pub not_after: Option<i64>,
    #[serde(default)]
    pub added_at: i64,
}

#[derive(Debug, Clone)]
struct CertProblem {
    presented: CertInfo,
    /// machine readable reason: unknown_issuer, expired, not_yet_valid, name_mismatch, other
    reason: String,
    detail: String,
    previous: Option<CertInfo>,
}

pub fn fingerprint(der: &[u8]) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, der);
    digest
        .as_ref()
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

fn cert_info(host: &str, port: u16, der: &CertificateDer<'_>) -> CertInfo {
    let mut info = CertInfo {
        host: host.to_string(),
        port,
        fingerprint: fingerprint(der.as_ref()),
        subject: String::new(),
        issuer: String::new(),
        not_before: None,
        not_after: None,
        added_at: 0,
    };
    if let Ok((_, cert)) = x509_parser::parse_x509_certificate(der.as_ref()) {
        info.subject = cert.subject().to_string();
        info.issuer = cert.issuer().to_string();
        let validity = cert.validity();
        info.not_before = Some(validity.not_before.timestamp() * 1000);
        info.not_after = Some(validity.not_after.timestamp() * 1000);
    }
    info
}

fn reason_of(e: &rustls::Error) -> &'static str {
    match e {
        rustls::Error::InvalidCertificate(c) => match c {
            CertificateError::UnknownIssuer => "unknown_issuer",
            CertificateError::Expired | CertificateError::ExpiredContext { .. } => "expired",
            CertificateError::NotValidYet | CertificateError::NotValidYetContext { .. } => {
                "not_yet_valid"
            }
            CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. } => {
                "name_mismatch"
            }
            CertificateError::Revoked => "revoked",
            _ => "other",
        },
        _ => "other",
    }
}

// ---------------------------------------------------------------------------
// Pinned certificate store
// ---------------------------------------------------------------------------

pub struct CertStore {
    path: PathBuf,
    entries: RwLock<Vec<CertInfo>>,
}

impl CertStore {
    pub fn load(path: PathBuf) -> Self {
        let entries = storage::read_json(&path).unwrap_or_default();
        Self {
            path,
            entries: RwLock::new(entries),
        }
    }

    pub fn pins_for(&self, host: &str, port: u16) -> Vec<CertInfo> {
        self.entries
            .read()
            .unwrap()
            .iter()
            .filter(|c| c.host.eq_ignore_ascii_case(host) && c.port == port)
            .cloned()
            .collect()
    }

    /// Pins the certificate, replacing earlier pins for host:port.
    pub fn trust(&self, mut cert: CertInfo) -> AppResult<()> {
        cert.added_at = chrono::Utc::now().timestamp_millis();
        let mut entries = self.entries.write().unwrap();
        entries.retain(|c| !(c.host.eq_ignore_ascii_case(&cert.host) && c.port == cert.port));
        entries.push(cert);
        storage::write_json(&self.path, &*entries)
    }

    pub fn list(&self) -> Vec<CertInfo> {
        self.entries.read().unwrap().clone()
    }

    pub fn remove(&self, host: &str, port: u16) -> AppResult<()> {
        let mut entries = self.entries.write().unwrap();
        entries.retain(|c| !(c.host.eq_ignore_ascii_case(host) && c.port == port));
        storage::write_json(&self.path, &*entries)
    }
}

// ---------------------------------------------------------------------------
// Verifier
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct PinningVerifier {
    inner: Arc<WebPkiServerVerifier>,
    provider: Arc<CryptoProvider>,
    host: String,
    port: u16,
    pins: Vec<CertInfo>,
    problem: Arc<Mutex<Option<CertProblem>>>,
}

impl ServerCertVerifier for PinningVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let fp = fingerprint(end_entity.as_ref());
        // Exactly this certificate was confirmed by the user before.
        if self.pins.iter().any(|p| p.fingerprint == fp) {
            return Ok(ServerCertVerified::assertion());
        }
        match self.inner.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        ) {
            Ok(v) => Ok(v),
            Err(e) => {
                let presented = cert_info(&self.host, self.port, end_entity);
                // typical for NAS / router certificates in local networks
                let self_signed =
                    !presented.subject.is_empty() && presented.subject == presented.issuer;
                let reason = match reason_of(&e) {
                    "unknown_issuer" | "other" if self_signed => "self_signed",
                    r => r,
                };
                *self.problem.lock().unwrap() = Some(CertProblem {
                    presented,
                    reason: reason.to_string(),
                    detail: e.to_string(),
                    previous: self.pins.first().cloned(),
                });
                Err(e)
            }
        }
    }

    // The handshake signatures are always verified, so a pinned certificate is only
    // accepted from a server that really owns its private key.
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// TLS settings for one connection (host:port) incl. its pinned certificates.
pub struct TlsContext {
    host: String,
    port: u16,
    pins: Vec<CertInfo>,
    problem: Arc<Mutex<Option<CertProblem>>>,
}

impl TlsContext {
    pub fn new(host: &str, port: u16, store: &CertStore) -> Self {
        Self {
            host: host.to_string(),
            port,
            pins: store.pins_for(host, port),
            problem: Arc::new(Mutex::new(None)),
        }
    }

    /// `tls12_only`: FTPS servers such as vsftpd abort TLS 1.3 data connections in the
    /// middle of an upload, so FTPS first tries TLS 1.2 (see `remote::ftp`).
    pub fn client_config(&self, tls12_only: bool) -> AppResult<Arc<rustls::ClientConfig>> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let inner = WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider.clone())
            .build()
            .map_err(|e| AppError::protocol(format!("TLS: {e}")))?;
        let verifier = PinningVerifier {
            inner,
            provider: provider.clone(),
            host: self.host.clone(),
            port: self.port,
            pins: self.pins.clone(),
            problem: self.problem.clone(),
        };
        let config = rustls::ClientConfig::builder_with_provider(provider)
            .with_protocol_versions(if tls12_only {
                TLS12_ONLY
            } else {
                rustls::DEFAULT_VERSIONS
            })
            .map_err(|e| AppError::protocol(format!("TLS: {e}")))?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(verifier))
            .with_no_client_auth();
        Ok(Arc::new(config))
    }

    /// Converts a recorded certificate problem into an error for the frontend.
    pub fn certificate_error(&self) -> Option<AppError> {
        let problem = self.problem.lock().unwrap().clone()?;
        let changed = problem
            .previous
            .as_ref()
            .is_some_and(|p| p.fingerprint != problem.presented.fingerprint);
        let (code, message) = if changed {
            (
                ErrorCode::CertChanged,
                "WARNING: the certificate of this server has changed",
            )
        } else {
            (
                ErrorCode::CertUntrusted,
                "The certificate of this server is not trusted",
            )
        };
        Some(
            AppError::new(code, format!("{message} ({})", problem.detail)).with_details(
                serde_json::json!({
                    "presented": problem.presented,
                    "previous": problem.previous,
                    "reason": problem.reason,
                }),
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_format() {
        let fp = fingerprint(b"abc");
        assert_eq!(fp.len(), 32 * 3 - 1);
        assert!(fp.starts_with("BA:78:16:BF"));
    }
}
