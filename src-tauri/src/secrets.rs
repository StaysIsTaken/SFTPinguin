//! Storage for passwords, passphrases and pasted private keys.
//!
//! * Desktop (Windows, macOS, Linux): the operating system's secure credential store
//!   (Windows Credential Manager, macOS Keychain, Secret Service / GNOME Keyring / KWallet).
//! * Mobile (Android, iOS): a file inside the app's private sandbox directory, which
//!   no other app can read.

use std::path::PathBuf;

use serde::Serialize;

use crate::error::{AppError, AppResult, ErrorCode};

#[cfg(not(mobile))]
const SERVICE: &str = "SFTPinguin";

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SecretBackend {
    SystemKeychain,
    AppSandbox,
}

pub struct SecretStore {
    #[cfg_attr(not(mobile), allow(dead_code))]
    file: PathBuf,
    #[cfg(mobile)]
    lock: std::sync::Mutex<()>,
}

pub fn key_for(site_id: &str, kind: &str) -> String {
    format!("site:{site_id}:{kind}")
}

fn store_error(e: impl std::fmt::Display) -> AppError {
    AppError::new(
        ErrorCode::SecretStore,
        format!("Secret store unavailable: {e}"),
    )
}

impl SecretStore {
    pub fn new(file: PathBuf) -> Self {
        Self {
            file,
            #[cfg(mobile)]
            lock: std::sync::Mutex::new(()),
        }
    }

    pub fn backend(&self) -> SecretBackend {
        if cfg!(mobile) {
            SecretBackend::AppSandbox
        } else {
            SecretBackend::SystemKeychain
        }
    }

    #[cfg(not(mobile))]
    pub fn get(&self, key: &str) -> AppResult<Option<String>> {
        let entry = keyring::Entry::new(SERVICE, key).map_err(store_error)?;
        match entry.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(store_error(e)),
        }
    }

    #[cfg(not(mobile))]
    pub fn set(&self, key: &str, value: &str) -> AppResult<()> {
        let entry = keyring::Entry::new(SERVICE, key).map_err(store_error)?;
        entry.set_password(value).map_err(store_error)
    }

    #[cfg(not(mobile))]
    pub fn delete(&self, key: &str) -> AppResult<()> {
        let entry = keyring::Entry::new(SERVICE, key).map_err(store_error)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(store_error(e)),
        }
    }

    #[cfg(mobile)]
    fn read_all(&self) -> std::collections::HashMap<String, String> {
        crate::storage::read_json(&self.file).unwrap_or_default()
    }

    #[cfg(mobile)]
    pub fn get(&self, key: &str) -> AppResult<Option<String>> {
        let _g = self.lock.lock().unwrap();
        Ok(self.read_all().get(key).cloned())
    }

    #[cfg(mobile)]
    pub fn set(&self, key: &str, value: &str) -> AppResult<()> {
        let _g = self.lock.lock().unwrap();
        let mut all = self.read_all();
        all.insert(key.to_string(), value.to_string());
        crate::storage::write_json(&self.file, &all)
    }

    #[cfg(mobile)]
    pub fn delete(&self, key: &str) -> AppResult<()> {
        let _g = self.lock.lock().unwrap();
        let mut all = self.read_all();
        if all.remove(key).is_some() {
            crate::storage::write_json(&self.file, &all)?;
        }
        Ok(())
    }
}
