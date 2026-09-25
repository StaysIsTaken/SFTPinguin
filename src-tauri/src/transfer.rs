//! Transfer queue: uploads / downloads with progress, cancellation, retry,
//! conflict handling and a configurable number of parallel transfers.

use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::sync::Semaphore;

use crate::error::{AppError, AppResult};
use crate::events::{self, LogLevel};
use crate::remote::{self, RemoteHandle};
use crate::AppState;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Upload,
    Download,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
    Skipped,
}

impl Status {
    fn is_finished(self) -> bool {
        matches!(
            self,
            Status::Done | Status::Failed | Status::Cancelled | Status::Skipped
        )
    }
}

/// What to do when the target file already exists.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ConflictPolicy {
    #[default]
    Overwrite,
    Skip,
    /// Only transfer when the source is newer than the target
    Newer,
    /// Keep both: the new file gets a unique name
    Rename,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferRequest {
    /// Upload: source; download: target (full path incl. file name)
    pub local_path: String,
    /// Upload: target (full path incl. file name); download: source
    pub remote_path: String,
    #[serde(default)]
    pub is_dir: bool,
    #[serde(default)]
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferInfo {
    pub id: String,
    pub session_id: String,
    pub direction: Direction,
    pub name: String,
    pub local_path: String,
    pub remote_path: String,
    pub size: u64,
    pub transferred: u64,
    pub status: Status,
    pub error: Option<String>,
    pub speed: f64,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub id: String,
    pub transferred: u64,
    pub size: u64,
    pub speed: f64,
}

pub struct Job {
    info: Mutex<TransferInfo>,
    transferred: AtomicU64,
    cancel: AtomicBool,
    policy: ConflictPolicy,
    /// (time, bytes, smoothed speed)
    sample: Mutex<(Instant, u64, f64)>,
}

impl Job {
    fn snapshot(&self) -> TransferInfo {
        let mut info = self.info.lock().unwrap().clone();
        info.transferred = self.transferred.load(Ordering::Relaxed);
        info
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    fn set_status(&self, status: Status, error: Option<String>) {
        let mut info = self.info.lock().unwrap();
        info.status = status;
        info.error = error;
        let now = chrono::Utc::now().timestamp_millis();
        match status {
            Status::Running => {
                info.started_at = Some(now);
                info.finished_at = None;
            }
            s if s.is_finished() => info.finished_at = Some(now),
            _ => {}
        }
    }
}

pub struct TransferManager {
    jobs: Mutex<Vec<Arc<Job>>>,
    sem: Arc<Semaphore>,
    limit: Mutex<usize>,
    preserve_mtime: AtomicBool,
}

impl Default for TransferManager {
    fn default() -> Self {
        Self {
            jobs: Mutex::new(Vec::new()),
            sem: Arc::new(Semaphore::new(3)),
            limit: Mutex::new(3),
            preserve_mtime: AtomicBool::new(true),
        }
    }
}

const CANCELLED_MSG: &str = "transfer cancelled";

// ---------------------------------------------------------------------------
// Progress tracking reader / writer
// ---------------------------------------------------------------------------

struct Tracked<'a, T> {
    inner: T,
    job: &'a Job,
}

impl<T: AsyncRead + Unpin> AsyncRead for Tracked<'_, T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.job.cancelled() {
            return Poll::Ready(Err(io::Error::other(CANCELLED_MSG)));
        }
        let before = buf.filled().len();
        let res = Pin::new(&mut self.inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = &res {
            let n = (buf.filled().len() - before) as u64;
            self.job.transferred.fetch_add(n, Ordering::Relaxed);
        }
        res
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for Tracked<'_, T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.job.cancelled() {
            return Poll::Ready(Err(io::Error::other(CANCELLED_MSG)));
        }
        let res = Pin::new(&mut self.inner).poll_write(cx, buf);
        if let Poll::Ready(Ok(n)) = &res {
            self.job.transferred.fetch_add(*n as u64, Ordering::Relaxed);
        }
        res
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

// ---------------------------------------------------------------------------

fn unique_local(path: &Path) -> PathBuf {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    for i in 1.. {
        let candidate = path.with_file_name(format!("{stem} ({i}){ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
}

fn numbered_name(name: &str, i: u32) -> String {
    match name.rfind('.') {
        Some(dot) if dot > 0 => format!("{} ({i}){}", &name[..dot], &name[dot..]),
        _ => format!("{name} ({i})"),
    }
}

async fn unique_remote(conn: &RemoteHandle, path: &str) -> AppResult<String> {
    let dir = remote::parent(path);
    let name = remote::file_name(path);
    for i in 1..1000 {
        let candidate = remote::join(&dir, &numbered_name(&name, i));
        if conn.stat(&candidate).await?.is_none() {
            return Ok(candidate);
        }
    }
    Err(AppError::protocol("Could not find a free file name"))
}

impl TransferManager {
    pub fn set_limit(&self, limit: usize) {
        let limit = limit.clamp(1, 10);
        let mut current = self.limit.lock().unwrap();
        if limit > *current {
            self.sem.add_permits(limit - *current);
        } else if limit < *current {
            let diff = (*current - limit) as u32;
            let sem = self.sem.clone();
            tauri::async_runtime::spawn(async move {
                if let Ok(p) = sem.acquire_many_owned(diff).await {
                    p.forget();
                }
            });
        }
        *current = limit;
    }

    pub fn set_preserve_mtime(&self, v: bool) {
        self.preserve_mtime.store(v, Ordering::Relaxed);
    }

    pub fn list(&self) -> Vec<TransferInfo> {
        self.jobs
            .lock()
            .unwrap()
            .iter()
            .map(|j| j.snapshot())
            .collect()
    }

    fn find(&self, id: &str) -> Option<Arc<Job>> {
        self.jobs
            .lock()
            .unwrap()
            .iter()
            .find(|j| j.info.lock().unwrap().id == id)
            .cloned()
    }

    pub fn cancel(&self, id: &str) {
        if let Some(job) = self.find(id) {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }

    pub fn cancel_where<F: Fn(&TransferInfo) -> bool>(&self, f: F) {
        for job in self.jobs.lock().unwrap().iter() {
            let info = job.info.lock().unwrap();
            if !info.status.is_finished() && f(&info) {
                job.cancel.store(true, Ordering::Relaxed);
            }
        }
    }

    /// Removes finished transfers (or all of them if `all`, cancelling running ones).
    pub fn clear(&self, all: bool) {
        let mut jobs = self.jobs.lock().unwrap();
        if all {
            for j in jobs.iter() {
                j.cancel.store(true, Ordering::Relaxed);
            }
        }
        jobs.retain(|j| {
            let status = j.info.lock().unwrap().status;
            if all {
                status == Status::Running
            } else {
                !status.is_finished()
            }
        });
    }

    fn add_job(
        &self,
        session_id: &str,
        direction: Direction,
        local_path: String,
        remote_path: String,
        size: u64,
        policy: ConflictPolicy,
    ) -> Arc<Job> {
        let name = match direction {
            Direction::Upload => Path::new(&local_path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
            Direction::Download => remote::file_name(&remote_path),
        };
        let job = Arc::new(Job {
            info: Mutex::new(TransferInfo {
                id: uuid::Uuid::new_v4().to_string(),
                session_id: session_id.to_string(),
                direction,
                name,
                local_path,
                remote_path,
                size,
                transferred: 0,
                status: Status::Queued,
                error: None,
                speed: 0.0,
                created_at: chrono::Utc::now().timestamp_millis(),
                started_at: None,
                finished_at: None,
            }),
            transferred: AtomicU64::new(0),
            cancel: AtomicBool::new(false),
            policy,
            sample: Mutex::new((Instant::now(), 0, 0.0)),
        });
        self.jobs.lock().unwrap().push(job.clone());
        job
    }

    /// Queues transfers. Folders are expanded in the background.
    pub fn enqueue(
        state: Arc<AppState>,
        session_id: String,
        direction: Direction,
        requests: Vec<TransferRequest>,
        policy: ConflictPolicy,
    ) {
        tauri::async_runtime::spawn(async move {
            for req in requests {
                let result = if req.is_dir {
                    Self::expand_dir(&state, &session_id, direction, &req, policy).await
                } else {
                    let job = state.transfers.add_job(
                        &session_id,
                        direction,
                        req.local_path.clone(),
                        req.remote_path.clone(),
                        req.size.unwrap_or(0),
                        policy,
                    );
                    events::emit(&state.app, "transfer-update", job.snapshot());
                    Self::spawn_job(state.clone(), job);
                    Ok(())
                };
                if let Err(e) = result {
                    let what = match direction {
                        Direction::Upload => &req.local_path,
                        Direction::Download => &req.remote_path,
                    };
                    events::log(
                        &state.app,
                        Some(&session_id),
                        LogLevel::Error,
                        format!("{what}: {}", e.message),
                    );
                }
            }
        });
    }

    async fn expand_dir(
        state: &Arc<AppState>,
        session_id: &str,
        direction: Direction,
        req: &TransferRequest,
        policy: ConflictPolicy,
    ) -> AppResult<()> {
        let session = state.sessions.get(session_id)?;
        let kh = &state.known_hosts;
        let mut new_jobs = Vec::new();
        match direction {
            Direction::Upload => {
                let root = PathBuf::from(&req.local_path);
                let (dirs, files) = {
                    let root = root.clone();
                    tokio::task::spawn_blocking(move || crate::local::walk(&root))
                        .await
                        .map_err(|e| AppError::protocol(e.to_string()))??
                };
                let to_remote = |p: &Path| -> String {
                    let rel = p.strip_prefix(&root).unwrap_or(p);
                    let mut target = req.remote_path.clone();
                    for c in rel.components() {
                        target = remote::join(&target, &c.as_os_str().to_string_lossy());
                    }
                    target
                };
                let mut remote_dirs = vec![req.remote_path.clone()];
                remote_dirs.extend(dirs.iter().map(|d| to_remote(d)));
                for d in remote_dirs {
                    // ignore "already exists" style errors
                    let _ = session
                        .run(kh, |c| {
                            let d = d.clone();
                            async move {
                                if c.stat(&d).await?.is_none() {
                                    c.mkdir(&d).await?;
                                }
                                Ok(())
                            }
                        })
                        .await;
                }
                for (file, size) in files {
                    new_jobs.push(state.transfers.add_job(
                        session_id,
                        direction,
                        file.to_string_lossy().to_string(),
                        to_remote(&file),
                        size,
                        policy,
                    ));
                }
            }
            Direction::Download => {
                let (dirs, files) = session.walk(kh, &req.remote_path).await?;
                let base = req.remote_path.trim_end_matches('/').to_string();
                let to_local = |p: &str| -> PathBuf {
                    let rel = p.strip_prefix(&base).unwrap_or(p).trim_start_matches('/');
                    let mut target = PathBuf::from(&req.local_path);
                    for seg in rel.split('/').filter(|s| !s.is_empty()) {
                        target.push(sanitize_local_name(seg));
                    }
                    target
                };
                tokio::fs::create_dir_all(&req.local_path).await?;
                for d in &dirs {
                    tokio::fs::create_dir_all(to_local(&d.path)).await?;
                }
                for f in files {
                    new_jobs.push(state.transfers.add_job(
                        session_id,
                        direction,
                        to_local(&f.path).to_string_lossy().to_string(),
                        f.path.clone(),
                        f.size,
                        policy,
                    ));
                }
            }
        }
        let snapshots: Vec<_> = new_jobs.iter().map(|j| j.snapshot()).collect();
        events::emit(&state.app, "transfers-added", snapshots);
        for job in new_jobs {
            Self::spawn_job(state.clone(), job);
        }
        Ok(())
    }

    pub fn retry(state: Arc<AppState>, id: &str) -> AppResult<()> {
        let job = state
            .transfers
            .find(id)
            .ok_or_else(|| AppError::not_found("Transfer not found"))?;
        {
            let info = job.info.lock().unwrap();
            if !matches!(info.status, Status::Failed | Status::Cancelled) {
                return Ok(());
            }
        }
        job.cancel.store(false, Ordering::Relaxed);
        job.transferred.store(0, Ordering::Relaxed);
        job.set_status(Status::Queued, None);
        events::emit(&state.app, "transfer-update", job.snapshot());
        Self::spawn_job(state, job);
        Ok(())
    }

    fn spawn_job(state: Arc<AppState>, job: Arc<Job>) {
        tauri::async_runtime::spawn(async move {
            let Ok(_permit) = state.transfers.sem.clone().acquire_owned().await else {
                return;
            };
            if job.cancelled() {
                job.set_status(Status::Cancelled, None);
                events::emit(&state.app, "transfer-update", job.snapshot());
                return;
            }
            job.transferred.store(0, Ordering::Relaxed);
            *job.sample.lock().unwrap() = (Instant::now(), 0, 0.0);
            job.set_status(Status::Running, None);
            events::emit(&state.app, "transfer-update", job.snapshot());

            let result = Self::run_job(&state, &job).await;
            let session_id = job.info.lock().unwrap().session_id.clone();
            match result {
                Ok(true) => job.set_status(Status::Done, None),
                Ok(false) => job.set_status(Status::Skipped, None),
                Err(_) if job.cancelled() => job.set_status(Status::Cancelled, None),
                Err(e) => {
                    let info = job.snapshot();
                    events::log(
                        &state.app,
                        Some(&session_id),
                        LogLevel::Error,
                        format!("{}: {}", info.name, e.message),
                    );
                    job.set_status(Status::Failed, Some(e.message))
                }
            }
            events::emit(&state.app, "transfer-update", job.snapshot());
        });
    }

    /// Returns Ok(false) if the file was skipped.
    async fn run_job(state: &Arc<AppState>, job: &Arc<Job>) -> AppResult<bool> {
        let info = job.snapshot();
        let session = state.sessions.get(&info.session_id)?;
        let conn = session.acquire(&state.known_hosts).await?;
        let result = match info.direction {
            Direction::Upload => Self::upload(state, job, &conn, &info).await,
            Direction::Download => Self::download(state, job, &conn, &info).await,
        };
        let reusable = match &result {
            Ok(_) => true,
            Err(e) => !e.is_fatal_for_connection() && !job.cancelled(),
        };
        session.release(conn, reusable).await;
        result
    }

    async fn upload(
        state: &Arc<AppState>,
        job: &Job,
        conn: &RemoteHandle,
        info: &TransferInfo,
    ) -> AppResult<bool> {
        let file = tokio::fs::File::open(&info.local_path).await?;
        let meta = file.metadata().await?;
        let size = meta.len();
        let local_mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64);
        job.info.lock().unwrap().size = size;

        let mut target = info.remote_path.clone();
        if job.policy != ConflictPolicy::Overwrite {
            if let Some(existing) = conn.stat(&target).await? {
                match job.policy {
                    ConflictPolicy::Skip => return Ok(false),
                    ConflictPolicy::Newer => {
                        let remote_secs = existing.modified.map(|m| m / 1000);
                        if let (Some(l), Some(r)) = (local_mtime, remote_secs) {
                            if l <= r {
                                return Ok(false);
                            }
                        }
                    }
                    ConflictPolicy::Rename => {
                        target = unique_remote(conn, &target).await?;
                        job.info.lock().unwrap().remote_path = target.clone();
                    }
                    ConflictPolicy::Overwrite => {}
                }
            }
        }

        let mut reader = Tracked {
            inner: tokio::io::BufReader::with_capacity(256 * 1024, file),
            job,
        };
        conn.upload(&mut reader, &target, size)
            .await
            .map_err(|e| if job.cancelled() { AppError::cancelled() } else { e })?;
        if state.transfers.preserve_mtime.load(Ordering::Relaxed) {
            if let Some(m) = local_mtime {
                let _ = conn.set_mtime(&target, m).await;
            }
        }
        Ok(true)
    }

    async fn download(
        state: &Arc<AppState>,
        job: &Job,
        conn: &RemoteHandle,
        info: &TransferInfo,
    ) -> AppResult<bool> {
        let remote_meta = conn.stat(&info.remote_path).await.ok().flatten();
        if let Some(m) = &remote_meta {
            if m.size > 0 {
                job.info.lock().unwrap().size = m.size;
            }
        }
        let mut target = PathBuf::from(&info.local_path);
        if target.exists() {
            match job.policy {
                ConflictPolicy::Overwrite => {}
                ConflictPolicy::Skip => return Ok(false),
                ConflictPolicy::Newer => {
                    let local = std::fs::metadata(&target)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs() as i64);
                    let remote_secs = remote_meta.as_ref().and_then(|m| m.modified).map(|m| m / 1000);
                    if let (Some(l), Some(r)) = (local, remote_secs) {
                        if r <= l {
                            return Ok(false);
                        }
                    }
                }
                ConflictPolicy::Rename => {
                    target = unique_local(&target);
                    job.info.lock().unwrap().local_path = target.to_string_lossy().to_string();
                }
            }
        }
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let part = target.with_file_name(format!(
            "{}.part",
            target
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default()
        ));
        let file = tokio::fs::File::create(&part).await?;
        let mut writer = Tracked {
            inner: tokio::io::BufWriter::with_capacity(256 * 1024, file),
            job,
        };
        let result = async {
            conn.download(&info.remote_path, &mut writer).await?;
            writer.inner.flush().await?;
            writer.inner.get_mut().sync_all().await?;
            Ok::<_, AppError>(())
        }
        .await;
        drop(writer);
        if let Err(e) = result {
            let _ = tokio::fs::remove_file(&part).await;
            return Err(if job.cancelled() { AppError::cancelled() } else { e });
        }
        tokio::fs::rename(&part, &target).await?;
        if state.transfers.preserve_mtime.load(Ordering::Relaxed) {
            if let Some(m) = remote_meta.and_then(|m| m.modified) {
                let _ = filetime::set_file_mtime(
                    &target,
                    filetime::FileTime::from_unix_time(m / 1000, 0),
                );
            }
        }
        Ok(true)
    }

    /// Periodically publishes progress of running transfers.
    pub fn start_progress_ticker(state: Arc<AppState>) {
        tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(300));
            loop {
                interval.tick().await;
                let running: Vec<Arc<Job>> = state
                    .transfers
                    .jobs
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|j| j.info.lock().unwrap().status == Status::Running)
                    .cloned()
                    .collect();
                if running.is_empty() {
                    continue;
                }
                let progress: Vec<Progress> = running
                    .iter()
                    .map(|j| {
                        let bytes = j.transferred.load(Ordering::Relaxed);
                        let mut sample = j.sample.lock().unwrap();
                        let elapsed = sample.0.elapsed().as_secs_f64();
                        if elapsed > 0.0 {
                            let instant = (bytes.saturating_sub(sample.1)) as f64 / elapsed;
                            // exponential smoothing for a calm speed display
                            sample.2 = if sample.2 == 0.0 {
                                instant
                            } else {
                                sample.2 * 0.7 + instant * 0.3
                            };
                        }
                        *sample = (Instant::now(), bytes, sample.2);
                        let mut info = j.info.lock().unwrap();
                        info.speed = sample.2;
                        Progress {
                            id: info.id.clone(),
                            transferred: bytes,
                            size: info.size,
                            speed: sample.2,
                        }
                    })
                    .collect();
                events::emit(&state.app, "transfer-progress", progress);
            }
        });
    }
}

/// Replaces characters that are not allowed in local file names (Windows).
pub fn sanitize_local_name(name: &str) -> String {
    if cfg!(windows) {
        name.chars()
            .map(|c| match c {
                '<' | '>' | ':' | '"' | '\\' | '|' | '?' | '*' => '_',
                c if (c as u32) < 32 => '_',
                c => c,
            })
            .collect()
    } else {
        name.replace('/', "_")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;

    #[test]
    fn numbered() {
        assert_eq!(numbered_name("a.txt", 1), "a (1).txt");
        assert_eq!(numbered_name(".bashrc", 2), ".bashrc (2)");
        assert_eq!(numbered_name("archive", 3), "archive (3)");
    }

    #[test]
    fn error_code_cancel() {
        let e = AppError::cancelled();
        assert_eq!(e.code, ErrorCode::Cancelled);
    }
}
