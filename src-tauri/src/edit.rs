//! "Open / edit remote file": downloads a file to a temporary folder, opens it with the
//! default application and watches it. When the file is saved, the frontend is asked
//! whether the changes should be uploaded.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use serde::Serialize;

use crate::error::{AppError, AppResult};
use crate::events;
use crate::AppState;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditedFile {
    pub id: String,
    pub session_id: String,
    pub remote_path: String,
    pub local_path: String,
    pub name: String,
}

struct Watch {
    file: EditedFile,
    mtime: Option<SystemTime>,
}

#[derive(Default)]
pub struct EditManager {
    watches: Mutex<Vec<Watch>>,
}

fn mtime(path: &str) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

impl EditManager {
    /// Downloads the remote file into a temporary folder and returns the local path.
    pub async fn fetch(
        state: &Arc<AppState>,
        session_id: &str,
        remote_path: &str,
        watch: bool,
    ) -> AppResult<EditedFile> {
        let session = state.sessions.get(session_id)?;
        let name = crate::transfer::sanitize_local_name(&crate::remote::file_name(remote_path));
        let id = uuid::Uuid::new_v4().to_string();
        let dir: PathBuf = std::env::temp_dir().join("sftpinguin").join(&id[..8]);
        tokio::fs::create_dir_all(&dir).await?;
        // Other users of this computer must not read the downloaded copies.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o700);
            let _ = tokio::fs::set_permissions(dir.parent().unwrap_or(&dir), perms.clone()).await;
            let _ = tokio::fs::set_permissions(&dir, perms).await;
        }
        let local = dir.join(&name);

        let conn = session.acquire(&state.trust).await?;
        let result = async {
            let mut file = tokio::fs::File::create(&local).await?;
            conn.download(remote_path, &mut file).await?;
            tokio::io::AsyncWriteExt::flush(&mut file).await?;
            Ok::<_, AppError>(())
        }
        .await;
        session.release(conn, result.is_ok()).await;
        result?;

        let file = EditedFile {
            id,
            session_id: session_id.to_string(),
            remote_path: remote_path.to_string(),
            local_path: local.to_string_lossy().to_string(),
            name,
        };
        if watch {
            state.edits.watches.lock().unwrap().push(Watch {
                mtime: mtime(&file.local_path),
                file: file.clone(),
            });
        }
        Ok(file)
    }

    pub fn get(&self, id: &str) -> Option<EditedFile> {
        self.watches
            .lock()
            .unwrap()
            .iter()
            .find(|w| w.file.id == id)
            .map(|w| w.file.clone())
    }

    pub fn list(&self) -> Vec<EditedFile> {
        self.watches
            .lock()
            .unwrap()
            .iter()
            .map(|w| w.file.clone())
            .collect()
    }

    pub fn stop(&self, id: &str) {
        self.watches.lock().unwrap().retain(|w| w.file.id != id);
    }

    pub fn stop_session(&self, session_id: &str) {
        self.watches
            .lock()
            .unwrap()
            .retain(|w| w.file.session_id != session_id);
    }

    pub fn start_watcher(state: Arc<AppState>) {
        tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(1500));
            loop {
                interval.tick().await;
                let mut changed = Vec::new();
                {
                    let mut watches = state.edits.watches.lock().unwrap();
                    for w in watches.iter_mut() {
                        let now = mtime(&w.file.local_path);
                        if now.is_some() && now != w.mtime {
                            w.mtime = now;
                            changed.push(w.file.clone());
                        }
                    }
                }
                for file in changed {
                    events::emit(&state.app, "edit-changed", file);
                }
            }
        });
    }
}
