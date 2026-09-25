//! Events sent to the frontend.

use serde::Serialize;
use tauri::{AppHandle, Emitter};

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Info,
    Success,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub session_id: Option<String>,
    pub level: LogLevel,
    pub message: String,
    pub time: i64,
}

pub fn log(app: &AppHandle, session_id: Option<&str>, level: LogLevel, message: impl Into<String>) {
    let entry = LogEntry {
        session_id: session_id.map(str::to_string),
        level,
        message: message.into(),
        time: chrono::Utc::now().timestamp_millis(),
    };
    let _ = app.emit("log", entry);
}

pub fn emit<S: Serialize + Clone>(app: &AppHandle, event: &str, payload: S) {
    let _ = app.emit(event, payload);
}
