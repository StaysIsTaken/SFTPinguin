//! Tauri commands (the API used by the frontend).

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::State;
use tauri_plugin_opener::OpenerExt;

use crate::edit::{EditManager, EditedFile};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::events::{self, LogLevel};
use crate::known_hosts::HostKey;
use crate::model::{AuthMethod, ConnectConfig, FileEntry, Site};
use crate::secrets::{key_for, SecretBackend};
use crate::session::{Session, SessionInfo};
use crate::transfer::{ConflictPolicy, Direction, TransferInfo, TransferManager, TransferRequest};
use crate::{local, sites, AppState};

type St<'a> = State<'a, Arc<AppState>>;

async fn blocking<T, F>(f: F) -> AppResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> AppResult<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| AppError::new(ErrorCode::Io, e.to_string()))?
}

// ---------------------------------------------------------------------------
// Platform
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformInfo {
    os: &'static str,
    mobile: bool,
    secret_backend: SecretBackend,
    version: &'static str,
    path_separator: char,
}

#[tauri::command]
pub fn platform_info(state: St<'_>) -> PlatformInfo {
    PlatformInfo {
        os: std::env::consts::OS,
        mobile: cfg!(mobile),
        secret_backend: state.secrets.backend(),
        version: env!("CARGO_PKG_VERSION"),
        path_separator: std::path::MAIN_SEPARATOR,
    }
}

// ---------------------------------------------------------------------------
// Site manager
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn list_sites(state: St<'_>) -> Vec<Site> {
    state.sites.list()
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SiteSecrets {
    /// Password, S3 secret key or key passphrase. `None` = keep the stored one.
    password: Option<String>,
    /// Pasted private key. `None` = keep the stored one.
    key_data: Option<String>,
    clear_key_data: bool,
}

#[tauri::command]
pub async fn save_site(state: St<'_>, site: Site, secrets: SiteSecrets) -> AppResult<Site> {
    let state = state.inner().clone();
    let mut site = site;
    if let Some(existing) = (!site.id.is_empty())
        .then(|| state.sites.get(&site.id))
        .flatten()
    {
        site.has_password = existing.has_password;
        site.has_key_data = existing.has_key_data;
        site.created_at = existing.created_at;
        site.last_used_at = existing.last_used_at;
    } else {
        site.has_password = false;
        site.has_key_data = false;
    }
    let mut site = state.sites.upsert(site)?;
    let id = site.id.clone();

    let st = state.clone();
    let save_password = site.save_password;
    let (has_password, has_key_data) = blocking(move || {
        let s = &st.secrets;
        let mut has_password = None;
        if !save_password {
            s.delete(&key_for(&id, "password"))?;
            has_password = Some(false);
        } else if let Some(pw) = secrets.password.filter(|p| !p.is_empty()) {
            s.set(&key_for(&id, "password"), &pw)?;
            has_password = Some(true);
        }
        let mut has_key = None;
        if secrets.clear_key_data {
            s.delete(&key_for(&id, "key"))?;
            has_key = Some(false);
        } else if let Some(k) = secrets.key_data.filter(|k| !k.trim().is_empty()) {
            s.set(&key_for(&id, "key"), &k)?;
            has_key = Some(true);
        }
        Ok((has_password, has_key))
    })
    .await?;

    if has_password.is_some() || has_key_data.is_some() {
        if let Some(updated) = state.sites.update(&site.id, |s| {
            if let Some(v) = has_password {
                s.has_password = v;
            }
            if let Some(v) = has_key_data {
                s.has_key_data = v;
            }
        })? {
            site = updated;
        }
    }
    Ok(site)
}

#[tauri::command]
pub async fn delete_site(state: St<'_>, id: String) -> AppResult<()> {
    let state = state.inner().clone();
    state.sites.remove(&id)?;
    blocking(move || {
        let _ = state.secrets.delete(&key_for(&id, "password"));
        let _ = state.secrets.delete(&key_for(&id, "key"));
        Ok(())
    })
    .await
}

#[tauri::command]
pub fn filezilla_default_path() -> Option<String> {
    sites::default_filezilla_path()
        .filter(|p| p.exists())
        .map(|p| p.to_string_lossy().to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    imported: usize,
    with_password: usize,
}

#[tauri::command]
pub async fn import_filezilla(state: St<'_>, path: String) -> AppResult<ImportResult> {
    let state = state.inner().clone();
    blocking(move || {
        let parsed = sites::parse_filezilla(std::path::Path::new(&path))?;
        let mut result = ImportResult {
            imported: 0,
            with_password: 0,
        };
        for item in parsed {
            let mut site = state.sites.upsert(item.site)?;
            if let Some(pw) = item.password {
                if state
                    .secrets
                    .set(&key_for(&site.id, "password"), &pw)
                    .is_ok()
                {
                    result.with_password += 1;
                    site.has_password = true;
                    state.sites.upsert(site)?;
                }
            }
            result.imported += 1;
        }
        Ok(result)
    })
    .await
}

// ---------------------------------------------------------------------------
// Host keys
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn trust_host_key(state: St<'_>, key: HostKey) -> AppResult<()> {
    state.known_hosts.trust(key)
}

#[tauri::command]
pub fn list_host_keys(state: St<'_>) -> Vec<HostKey> {
    state.known_hosts.list()
}

#[tauri::command]
pub fn remove_host_key(state: St<'_>, host: String, port: u16) -> AppResult<()> {
    state.known_hosts.remove(&host, port)
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectRequest {
    /// Connect to a saved site ...
    site_id: Option<String>,
    /// ... or to an ad-hoc site (quick connect)
    site: Option<Site>,
    /// Password / passphrase / secret key entered in a prompt
    password: Option<String>,
    /// Remember the entered password for the saved site
    #[serde(default)]
    remember: bool,
}

#[tauri::command]
pub async fn connect(state: St<'_>, req: ConnectRequest) -> AppResult<SessionInfo> {
    let state = state.inner().clone();
    let site = match (&req.site_id, req.site) {
        (Some(id), _) => state
            .sites
            .get(id)
            .ok_or_else(|| AppError::not_found("Site not found"))?,
        (None, Some(site)) => site,
        (None, None) => return Err(AppError::invalid("No site given")),
    };

    // Resolve secrets
    let (stored_password, key_data) = if req.site_id.is_some() {
        let st = state.clone();
        let id = site.id.clone();
        let want_pw = site.has_password && req.password.is_none();
        let want_key = site.has_key_data;
        blocking(move || {
            let pw = if want_pw {
                st.secrets.get(&key_for(&id, "password")).unwrap_or(None)
            } else {
                None
            };
            let key = if want_key {
                st.secrets.get(&key_for(&id, "key")).unwrap_or(None)
            } else {
                None
            };
            Ok((pw, key))
        })
        .await?
    } else {
        (None, None)
    };
    let password = req.password.clone().or(stored_password);
    let config = ConnectConfig {
        passphrase: if site.auth == AuthMethod::Key {
            password.clone()
        } else {
            None
        },
        password: if site.auth == AuthMethod::Key {
            None
        } else {
            password.clone()
        },
        key_data,
        site: site.clone(),
    };

    events::log(
        &state.app,
        None,
        LogLevel::Info,
        format!(
            "Connecting to {} ({} {}:{}) …",
            site.display_name(),
            site.protocol.label(),
            site.host,
            site.port()
        ),
    );
    let session = match Session::open(config, req.site_id.clone(), &state.known_hosts).await {
        Ok(s) => s,
        Err(e) => {
            match e.code {
                ErrorCode::HostKeyUnknown
                | ErrorCode::PasswordRequired
                | ErrorCode::PassphraseRequired => {}
                ErrorCode::HostKeyChanged => {
                    events::log(&state.app, None, LogLevel::Warn, e.message.clone())
                }
                _ => events::log(&state.app, None, LogLevel::Error, e.message.clone()),
            }
            return Err(e);
        }
    };
    state.sessions.insert(session.clone());
    events::log(
        &state.app,
        Some(&session.id),
        LogLevel::Success,
        format!("Connected to {}", session.info.title),
    );

    if let Some(id) = &req.site_id {
        // remember a newly entered password if wanted
        if req.remember && site.save_password {
            if let Some(pw) = req.password.clone() {
                let st = state.clone();
                let key = key_for(id, "password");
                if blocking(move || st.secrets.set(&key, &pw)).await.is_ok() {
                    let _ = state.sites.update(id, |s| s.has_password = true);
                }
            }
        }
        let _ = state.sites.update(id, |s| {
            s.last_used_at = Some(chrono::Utc::now().timestamp_millis())
        });
    }
    Ok(session.info.clone())
}

#[tauri::command]
pub async fn disconnect(state: St<'_>, session_id: String) -> AppResult<()> {
    state.transfers.cancel_where(|t| t.session_id == session_id);
    state.edits.stop_session(&session_id);
    if let Some(session) = state.sessions.remove(&session_id) {
        session.close().await;
        events::log(
            &state.app,
            Some(&session_id),
            LogLevel::Info,
            format!("Disconnected from {}", session.info.title),
        );
    }
    Ok(())
}

#[tauri::command]
pub async fn list_remote(
    state: St<'_>,
    session_id: String,
    path: String,
) -> AppResult<Vec<FileEntry>> {
    let session = state.sessions.get(&session_id)?;
    session
        .run(&state.known_hosts, |c| {
            let path = path.clone();
            async move { c.list(&path).await }
        })
        .await
}

#[tauri::command]
pub async fn remote_stat(
    state: St<'_>,
    session_id: String,
    path: String,
) -> AppResult<Option<FileEntry>> {
    let session = state.sessions.get(&session_id)?;
    session
        .run(&state.known_hosts, |c| {
            let path = path.clone();
            async move { c.stat(&path).await }
        })
        .await
}

#[tauri::command]
pub async fn remote_mkdir(state: St<'_>, session_id: String, path: String) -> AppResult<()> {
    let session = state.sessions.get(&session_id)?;
    session
        .run(&state.known_hosts, |c| {
            let path = path.clone();
            async move { c.mkdir(&path).await }
        })
        .await?;
    events::log(
        &state.app,
        Some(&session_id),
        LogLevel::Info,
        format!("Created folder {path}"),
    );
    Ok(())
}

#[tauri::command]
pub async fn remote_create_file(state: St<'_>, session_id: String, path: String) -> AppResult<()> {
    let session = state.sessions.get(&session_id)?;
    session
        .run(&state.known_hosts, |c| {
            let path = path.clone();
            async move {
                if c.stat(&path).await?.is_some() {
                    return Err(AppError::new(
                        ErrorCode::AlreadyExists,
                        format!("{path} already exists"),
                    ));
                }
                let mut empty: &[u8] = &[];
                c.upload(&mut empty, &path, 0).await
            }
        })
        .await
}

#[tauri::command]
pub async fn remote_rename(
    state: St<'_>,
    session_id: String,
    from: String,
    to: String,
) -> AppResult<()> {
    let session = state.sessions.get(&session_id)?;
    session
        .run(&state.known_hosts, |c| {
            let (from, to) = (from.clone(), to.clone());
            async move { c.rename(&from, &to).await }
        })
        .await?;
    events::log(
        &state.app,
        Some(&session_id),
        LogLevel::Info,
        format!("Renamed {from} → {to}"),
    );
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathItem {
    path: String,
    is_dir: bool,
}

#[tauri::command]
pub async fn remote_delete(
    state: St<'_>,
    session_id: String,
    items: Vec<PathItem>,
) -> AppResult<()> {
    let session = state.sessions.get(&session_id)?;
    let kh = &state.known_hosts;
    for item in items {
        if item.is_dir {
            let (dirs, files) = session.walk(kh, &item.path).await?;
            for f in files {
                session
                    .run(kh, |c| {
                        let p = f.path.clone();
                        async move { c.remove_file(&p).await }
                    })
                    .await?;
            }
            for d in dirs
                .iter()
                .rev()
                .map(|d| d.path.clone())
                .chain([item.path.clone()])
            {
                session
                    .run(kh, |c| {
                        let p = d.clone();
                        async move { c.remove_dir(&p).await }
                    })
                    .await?;
            }
        } else {
            session
                .run(kh, |c| {
                    let p = item.path.clone();
                    async move { c.remove_file(&p).await }
                })
                .await?;
        }
        events::log(
            &state.app,
            Some(&session_id),
            LogLevel::Info,
            format!("Deleted {}", item.path),
        );
    }
    Ok(())
}

#[tauri::command]
pub async fn remote_chmod(
    state: St<'_>,
    session_id: String,
    items: Vec<PathItem>,
    mode: u32,
    recursive: bool,
) -> AppResult<()> {
    let session = state.sessions.get(&session_id)?;
    let kh = &state.known_hosts;
    for item in items {
        let mut targets = vec![item.path.clone()];
        if recursive && item.is_dir {
            let (dirs, files) = session.walk(kh, &item.path).await?;
            targets.extend(dirs.into_iter().map(|d| d.path));
            targets.extend(files.into_iter().map(|f| f.path));
        }
        for t in targets {
            session
                .run(kh, |c| {
                    let t = t.clone();
                    async move { c.chmod(&t, mode).await }
                })
                .await?;
        }
    }
    events::log(
        &state.app,
        Some(&session_id),
        LogLevel::Info,
        format!("Changed permissions to {:o}", mode),
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Local file system
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn list_local(path: String) -> AppResult<Vec<FileEntry>> {
    blocking(move || local::list(&path)).await
}

#[tauri::command]
pub fn local_places(app: tauri::AppHandle) -> Vec<local::Place> {
    local::places(&app)
}

#[tauri::command]
pub fn local_default_dir(app: tauri::AppHandle) -> String {
    local::default_dir(&app)
}

#[tauri::command]
pub async fn local_stat(path: String) -> AppResult<Option<FileEntry>> {
    blocking(move || {
        let p = std::path::Path::new(&path);
        Ok(if p.exists() {
            Some(local::entry_for(p)?)
        } else {
            None
        })
    })
    .await
}

#[tauri::command]
pub async fn local_mkdir(path: String) -> AppResult<()> {
    blocking(move || local::mkdir(&path)).await
}

#[tauri::command]
pub async fn local_create_file(path: String) -> AppResult<()> {
    blocking(move || local::create_file(&path)).await
}

#[tauri::command]
pub async fn local_rename(from: String, to: String) -> AppResult<()> {
    blocking(move || local::rename(&from, &to)).await
}

#[tauri::command]
pub async fn local_delete(paths: Vec<String>, trash: bool) -> AppResult<()> {
    blocking(move || local::delete(&paths, trash)).await
}

#[tauri::command]
pub async fn local_chmod(paths: Vec<String>, mode: u32) -> AppResult<()> {
    blocking(move || {
        for p in paths {
            local::chmod(&p, mode)?;
        }
        Ok(())
    })
    .await
}

// ---------------------------------------------------------------------------
// Transfers
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn enqueue_transfers(
    state: St<'_>,
    session_id: String,
    direction: Direction,
    items: Vec<TransferRequest>,
    policy: ConflictPolicy,
) -> AppResult<()> {
    state.sessions.get(&session_id)?;
    TransferManager::enqueue(state.inner().clone(), session_id, direction, items, policy);
    Ok(())
}

#[tauri::command]
pub fn list_transfers(state: St<'_>) -> Vec<TransferInfo> {
    state.transfers.list()
}

#[tauri::command]
pub fn cancel_transfer(state: St<'_>, id: String) {
    state.transfers.cancel(&id);
}

#[tauri::command]
pub fn cancel_all_transfers(state: St<'_>) {
    state.transfers.cancel_where(|_| true);
}

#[tauri::command]
pub fn retry_transfer(state: St<'_>, id: String) -> AppResult<()> {
    TransferManager::retry(state.inner().clone(), &id)
}

#[tauri::command]
pub fn clear_transfers(state: St<'_>, all: bool) {
    state.transfers.clear(all);
}

#[tauri::command]
pub fn set_transfer_options(state: St<'_>, max_concurrent: usize, preserve_mtime: bool) {
    state.transfers.set_limit(max_concurrent);
    state.transfers.set_preserve_mtime(preserve_mtime);
}

// ---------------------------------------------------------------------------
// Open / edit remote files
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn open_remote_file(
    app: tauri::AppHandle,
    state: St<'_>,
    session_id: String,
    path: String,
    watch: bool,
) -> AppResult<EditedFile> {
    let state = state.inner().clone();
    let file = EditManager::fetch(&state, &session_id, &path, watch).await?;
    app.opener()
        .open_path(&file.local_path, None::<&str>)
        .map_err(|e| AppError::new(ErrorCode::Io, e.to_string()))?;
    Ok(file)
}

#[tauri::command]
pub fn upload_edited(state: St<'_>, id: String) -> AppResult<()> {
    let file = state
        .edits
        .get(&id)
        .ok_or_else(|| AppError::not_found("File is no longer watched"))?;
    state.sessions.get(&file.session_id)?;
    TransferManager::enqueue(
        state.inner().clone(),
        file.session_id.clone(),
        Direction::Upload,
        vec![TransferRequest {
            local_path: file.local_path.clone(),
            remote_path: file.remote_path.clone(),
            is_dir: false,
            size: None,
        }],
        ConflictPolicy::Overwrite,
    );
    Ok(())
}

#[tauri::command]
pub fn stop_editing(state: St<'_>, id: String) {
    state.edits.stop(&id);
}

#[tauri::command]
pub fn list_edited(state: St<'_>) -> Vec<EditedFile> {
    state.edits.list()
}

#[tauri::command]
pub fn open_local_path(app: tauri::AppHandle, path: String) -> AppResult<()> {
    app.opener()
        .open_path(&path, None::<&str>)
        .map_err(|e| AppError::new(ErrorCode::Io, e.to_string()))
}
