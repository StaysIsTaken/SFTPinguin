//! Trust store for SSH host keys (trust on first use, like OpenSSH's known_hosts).

use std::path::PathBuf;
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

use crate::error::AppResult;
use crate::storage;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HostKey {
    pub host: String,
    pub port: u16,
    pub key_type: String,
    /// `SHA256:...` fingerprint
    pub fingerprint: String,
    #[serde(default)]
    pub added_at: i64,
}

pub enum HostKeyStatus {
    Trusted,
    Unknown,
    Changed(HostKey),
}

pub struct KnownHosts {
    path: PathBuf,
    entries: RwLock<Vec<HostKey>>,
}

impl KnownHosts {
    pub fn load(path: PathBuf) -> Self {
        let entries = storage::read_json(&path).unwrap_or_default();
        Self {
            path,
            entries: RwLock::new(entries),
        }
    }

    /// In-memory snapshot (not persisted).
    pub fn from_entries(entries: Vec<HostKey>) -> Self {
        Self {
            path: PathBuf::new(),
            entries: RwLock::new(entries),
        }
    }

    pub fn check(&self,host: &str, port: u16, fingerprint: &str) -> HostKeyStatus {
        let entries = self.entries.read().unwrap();
        let mut known = entries
            .iter()
            .filter(|e| e.host.eq_ignore_ascii_case(host) && e.port == port)
            .peekable();
        if known.peek().is_none() {
            return HostKeyStatus::Unknown;
        }
        let mut first = None;
        for e in known {
            if e.fingerprint == fingerprint {
                return HostKeyStatus::Trusted;
            }
            first.get_or_insert_with(|| e.clone());
        }
        HostKeyStatus::Changed(first.expect("non-empty"))
    }

    /// Trusts the key, replacing any previously stored key for host:port.
    pub fn trust(&self, mut key: HostKey) -> AppResult<()> {
        key.added_at = chrono::Utc::now().timestamp_millis();
        let mut entries = self.entries.write().unwrap();
        entries.retain(|e| !(e.host.eq_ignore_ascii_case(&key.host) && e.port == key.port));
        entries.push(key);
        storage::write_json(&self.path, &*entries)
    }

    pub fn list(&self) -> Vec<HostKey> {
        self.entries.read().unwrap().clone()
    }

    pub fn remove(&self, host: &str, port: u16) -> AppResult<()> {
        let mut entries = self.entries.write().unwrap();
        entries.retain(|e| !(e.host.eq_ignore_ascii_case(host) && e.port == port));
        storage::write_json(&self.path, &*entries)
    }
}
