//! Open connections. Each session has one "browse" connection for directory listings
//! and file operations plus a small pool of extra connections used by transfers, so
//! browsing stays responsive while files are being transferred.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::sync::Notify;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::events::{self, LogLevel};
use crate::model::{Capabilities, ConnectConfig, FileEntry, Protocol};
use crate::remote::{self, RemoteHandle};
use crate::trust::Trust;
use crate::AppState;

const MAX_POOLED: usize = 8;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub id: String,
    pub site_id: Option<String>,
    pub title: String,
    pub protocol: Protocol,
    pub host: String,
    pub username: String,
    pub home: String,
    pub local_path: String,
    pub capabilities: Capabilities,
    /// Transport is encrypted and the server identity verified (SSH / TLS)
    pub encrypted: bool,
}

pub struct Session {
    pub id: String,
    pub config: ConnectConfig,
    pub info: SessionInfo,
    browse: tokio::sync::Mutex<Option<RemoteHandle>>,
    pool: tokio::sync::Mutex<Vec<RemoteHandle>>,
    /// Set when the server / network ended the connection (reason for the user)
    closed: Mutex<Option<String>>,
    last_check: Mutex<Instant>,
    /// Wakes the session monitor as soon as the session is marked closed
    wake: Arc<Notify>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionClosed {
    pub session_id: String,
    pub title: String,
    pub reason: String,
}

fn closed_error() -> AppError {
    AppError::new(ErrorCode::SessionClosed, "The connection was closed")
}

impl Session {
    pub async fn open(
        config: ConnectConfig,
        site_id: Option<String>,
        trust: &Trust,
        wake: Arc<Notify>,
    ) -> AppResult<Arc<Self>> {
        let conn = remote::connect(&config, trust).await?;
        let mut home = if config.site.remote_path.trim().is_empty() {
            conn.home().await.unwrap_or_else(|_| "/".to_string())
        } else {
            config.site.remote_path.trim().to_string()
        };
        if home.is_empty() {
            home = "/".into();
        }
        let id = uuid::Uuid::new_v4().to_string();
        let info = SessionInfo {
            id: id.clone(),
            site_id,
            title: config.site.display_name(),
            protocol: config.site.protocol,
            host: config.site.host.clone(),
            username: config.site.username.clone(),
            home,
            local_path: config.site.local_path.clone(),
            capabilities: conn.capabilities(),
            encrypted: config.site.insecure_reason().is_none(),
        };
        Ok(Arc::new(Self {
            id,
            config,
            info,
            browse: tokio::sync::Mutex::new(Some(conn)),
            pool: tokio::sync::Mutex::new(Vec::new()),
            closed: Mutex::new(None),
            last_check: Mutex::new(Instant::now()),
            wake,
        }))
    }

    /// Marks the session as ended by the server / network. The session monitor then
    /// closes it and informs the frontend.
    pub fn mark_closed(&self, reason: impl Into<String>) {
        let mut closed = self.closed.lock().unwrap();
        if closed.is_none() {
            *closed = Some(reason.into());
            self.wake.notify_one();
        }
    }

    pub fn closed_reason(&self) -> Option<String> {
        self.closed.lock().unwrap().clone()
    }

    /// Checks the browse connection if its protocol keeps a connection open and the
    /// check is due. Marks the session closed when the connection is gone.
    async fn check_liveness(&self) {
        let Some(conn) = self.browse.lock().await.clone() else {
            return;
        };
        let Some(interval) = conn.liveness_interval() else {
            return;
        };
        {
            let mut last = self.last_check.lock().unwrap();
            if last.elapsed() < interval {
                return;
            }
            *last = Instant::now();
        }
        if !conn.is_alive().await {
            self.mark_closed("The server closed the connection");
        }
    }

    async fn browse_conn(&self) -> AppResult<RemoteHandle> {
        if self.closed_reason().is_some() {
            return Err(closed_error());
        }
        self.browse.lock().await.clone().ok_or_else(closed_error)
    }

    /// Runs an operation on the browse connection. If the connection turns out to be
    /// dead (server timeout, restart, network loss) the session is marked closed; the
    /// user is informed instead of silently reconnecting.
    pub async fn run<T, F, Fut>(&self, _trust: &Trust, f: F) -> AppResult<T>
    where
        F: Fn(RemoteHandle) -> Fut,
        Fut: Future<Output = AppResult<T>>,
    {
        let conn = self.browse_conn().await?;
        match f(conn.clone()).await {
            Err(e) if matches!(e.code, ErrorCode::Connection | ErrorCode::SessionClosed) => {
                if conn.is_alive().await {
                    // only this operation failed (e.g. a blocked FTP data connection)
                    Err(e)
                } else {
                    log::info!("connection lost: {e}");
                    self.mark_closed(e.message.clone());
                    Err(closed_error())
                }
            }
            other => other,
        }
    }

    /// Takes a connection for a transfer (from the pool or newly opened).
    pub async fn acquire(&self, trust: &Trust) -> AppResult<RemoteHandle> {
        if self.closed_reason().is_some() {
            return Err(closed_error());
        }
        loop {
            let candidate = self.pool.lock().await.pop();
            match candidate {
                Some(conn) => {
                    if conn.is_alive().await {
                        return Ok(conn);
                    }
                    conn.close().await;
                }
                None => break,
            }
        }
        remote::connect(&self.config, trust).await
    }

    /// Returns a transfer connection to the pool.
    pub async fn release(&self, conn: RemoteHandle, reusable: bool) {
        if reusable {
            let mut pool = self.pool.lock().await;
            if pool.len() < MAX_POOLED {
                pool.push(conn);
                return;
            }
        }
        tauri::async_runtime::spawn(async move { conn.close().await });
    }

    pub async fn close(&self) {
        if let Some(c) = self.browse.lock().await.take() {
            c.close().await;
        }
        let pooled: Vec<_> = self.pool.lock().await.drain(..).collect();
        for c in pooled {
            c.close().await;
        }
    }

    /// Recursively lists a remote directory (used for folder transfers and deletes).
    /// Returns (dirs, files) with dirs in top-down order.
    pub async fn walk(
        &self,
        trust: &Trust,
        root: &str,
    ) -> AppResult<(Vec<FileEntry>, Vec<FileEntry>)> {
        let mut dirs = Vec::new();
        let mut files = Vec::new();
        let mut stack = vec![root.to_string()];
        while let Some(dir) = stack.pop() {
            let path = dir.clone();
            let entries = self
                .run(trust, |c| {
                    let path = path.clone();
                    async move { c.list(&path).await }
                })
                .await?;
            for e in entries {
                if e.is_dir() {
                    // never follow directory symlinks while recursing (loops!)
                    if e.is_link {
                        continue;
                    }
                    stack.push(e.path.clone());
                    dirs.push(e);
                } else {
                    files.push(e);
                }
            }
        }
        Ok((dirs, files))
    }
}

#[derive(Default)]
pub struct SessionManager {
    sessions: RwLock<HashMap<String, Arc<Session>>>,
    pub wake: Arc<Notify>,
}

/// Ends a session: cancels its transfers, stops watching edited files and closes all
/// connections. With a `reason` (server / network ended it) the frontend is told so it
/// can inform the user and close the tab.
pub async fn end_session(state: &Arc<AppState>, id: &str, reason: Option<String>) {
    state.transfers.cancel_where(|t| t.session_id == id);
    state.edits.stop_session(id);
    let Some(session) = state.sessions.remove(id) else {
        return;
    };
    session.close().await;
    match reason {
        Some(reason) => {
            events::log(
                &state.app,
                Some(id),
                LogLevel::Warn,
                format!("Connection to {} closed: {reason}", session.info.title),
            );
            events::emit(
                &state.app,
                "session-closed",
                SessionClosed {
                    session_id: id.to_string(),
                    title: session.info.title.clone(),
                    reason,
                },
            );
        }
        None => events::log(
            &state.app,
            Some(id),
            LogLevel::Info,
            format!("Disconnected from {}", session.info.title),
        ),
    }
}

/// Watches all sessions and ends those whose connection was closed by the server.
pub fn start_monitor(state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = interval.tick() => {}
                _ = state.sessions.wake.notified() => {}
            }
            let sessions = state.sessions.all();
            // check all sessions concurrently, a hanging server must not delay others
            futures_util::future::join_all(sessions.iter().map(|s| s.check_liveness())).await;
            for s in sessions {
                if let Some(reason) = s.closed_reason() {
                    end_session(&state, &s.id, Some(reason)).await;
                }
            }
        }
    });
}

impl SessionManager {
    pub fn insert(&self, session: Arc<Session>) {
        self.sessions
            .write()
            .unwrap()
            .insert(session.id.clone(), session);
    }

    pub fn get(&self, id: &str) -> AppResult<Arc<Session>> {
        self.sessions
            .read()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::new(ErrorCode::SessionClosed, "The connection was closed"))
    }

    pub fn remove(&self, id: &str) -> Option<Arc<Session>> {
        self.sessions.write().unwrap().remove(id)
    }

    pub fn all(&self) -> Vec<Arc<Session>> {
        self.sessions.read().unwrap().values().cloned().collect()
    }
}
