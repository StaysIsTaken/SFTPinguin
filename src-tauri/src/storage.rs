//! Small helpers for persisting JSON files in the app config directory.

use std::path::Path;

use serde::{de::DeserializeOwned, Serialize};

use crate::error::AppResult;

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let data = std::fs::read(path).ok()?;
    match serde_json::from_slice(&data) {
        Ok(v) => Some(v),
        Err(e) => {
            log::warn!("could not parse {}: {e}", path.display());
            // keep a copy of the broken file so nothing is lost
            let _ = std::fs::copy(path, path.with_extension("json.broken"));
            None
        }
    }
}

/// Writes atomically (temp file + rename) with owner-only permissions on unix.
pub fn write_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> AppResult<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let data = serde_json::to_vec_pretty(value)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, data)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}
