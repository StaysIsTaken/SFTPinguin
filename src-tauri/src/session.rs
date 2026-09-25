//! Open connections. Each session has one "browse" connection for directory listings
//! and file operations plus a small pool of extra connections used by transfers, so
//! browsing stays responsive while files are being transferred.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, RwLock};

use serde::Serialize;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::known_hosts::KnownHosts;
use crate::model::{Capabilities, ConnectConfig, FileEntry, Protocol};
use crate::remote::{self, RemoteHandle};

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
}

pub struct Session {
    pub id: String,
    pub config: ConnectConfig,
    pub info: SessionInfo,
    browse: tokio::sync::Mutex<Option<RemoteHandle>>,
    pool: tokio::sync::Mutex<Vec<RemoteHandle>>,
}

impl Session {
    pub async fn open(
        config: ConnectConfig,
        site_id: Option<String>,
        known_hosts: &KnownHosts,
    ) -> AppResult<Arc<Self>> {
        let conn = remote::connect(&config, known_hosts).await?;
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
        };
        Ok(Arc::new(Self {
            id,
            config,
            info,
            browse: tokio::sync::Mutex::new(Some(conn)),
            pool: tokio::sync::Mutex::new(Vec::new()),
        }))
    }

    async fn browse_conn(&self, known_hosts: &KnownHosts) -> AppResult<RemoteHandle> {
        let mut guard = self.browse.lock().await;
        if let Some(conn) = guard.as_ref() {
            return Ok(conn.clone());
        }
        let conn = remote::connect(&self.config, known_hosts).await?;
        *guard = Some(conn.clone());
        Ok(conn)
    }

    async fn invalidate_browse(&self, failed: &RemoteHandle) {
        let mut guard = self.browse.lock().await;
        if guard.as_ref().is_some_and(|c| Arc::ptr_eq(c, failed)) {
            if let Some(c) = guard.take() {
                tauri::async_runtime::spawn(async move { c.close().await });
            }
        }
    }

    /// Runs an operation on the browse connection. If the connection was lost
    /// (server timeout, network change, ...) it reconnects once and retries.
    pub async fn run<T, F, Fut>(&self, known_hosts: &KnownHosts, f: F) -> AppResult<T>
    where
        F: Fn(RemoteHandle) -> Fut,
        Fut: Future<Output = AppResult<T>>,
    {
        let conn = self.browse_conn(known_hosts).await?;
        match f(conn.clone()).await {
            Err(e) if matches!(e.code, ErrorCode::Connection | ErrorCode::SessionClosed) => {
                log::info!("connection lost ({e}), reconnecting");
                self.invalidate_browse(&conn).await;
                let conn = self.browse_conn(known_hosts).await?;
                f(conn).await
            }
            other => other,
        }
    }

    /// Takes a connection for a transfer (from the pool or newly opened).
    pub async fn acquire(&self, known_hosts: &KnownHosts) -> AppResult<RemoteHandle> {
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
        remote::connect(&self.config, known_hosts).await
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
        known_hosts: &KnownHosts,
        root: &str,
    ) -> AppResult<(Vec<FileEntry>, Vec<FileEntry>)> {
        let mut dirs = Vec::new();
        let mut files = Vec::new();
        let mut stack = vec![root.to_string()];
        while let Some(dir) = stack.pop() {
            let path = dir.clone();
            let entries = self
                .run(known_hosts, |c| {
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
