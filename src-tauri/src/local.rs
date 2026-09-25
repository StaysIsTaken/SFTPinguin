//! Local file system access for the left pane.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::error::{AppError, AppResult};
use crate::model::{EntryKind, FileEntry};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Place {
    pub id: String,
    pub path: String,
}

fn to_millis(t: std::io::Result<std::time::SystemTime>) -> Option<i64> {
    t.ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
}

pub fn entry_for(path: &Path) -> AppResult<FileEntry> {
    let link_meta = std::fs::symlink_metadata(path)?;
    let is_link = link_meta.file_type().is_symlink();
    // follow links for kind / size; fall back to the link itself if it is dangling
    let meta = if is_link {
        std::fs::metadata(path).unwrap_or(link_meta)
    } else {
        link_meta
    };
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        Some(meta.permissions().mode() & 0o7777)
    };
    #[cfg(not(unix))]
    let mode = None;
    Ok(FileEntry {
        name: path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string()),
        path: path.to_string_lossy().to_string(),
        kind: if meta.is_dir() {
            EntryKind::Dir
        } else {
            EntryKind::File
        },
        size: if meta.is_dir() { 0 } else { meta.len() },
        modified: to_millis(meta.modified()),
        mode,
        owner: None,
        group: None,
        is_link,
        link_target: if is_link {
            std::fs::read_link(path)
                .ok()
                .map(|p| p.to_string_lossy().to_string())
        } else {
            None
        },
    })
}

pub fn list(path: &str) -> AppResult<Vec<FileEntry>> {
    #[cfg(windows)]
    if path.is_empty() || path == "/" || path == "\\" {
        return Ok(windows_drives());
    }
    let dir = PathBuf::from(path);
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let Ok(entry) = entry else { continue };
        if let Ok(fe) = entry_for(&entry.path()) {
            out.push(fe);
        }
    }
    Ok(out)
}

#[cfg(windows)]
fn windows_drives() -> Vec<FileEntry> {
    (b'A'..=b'Z')
        .filter_map(|l| {
            let root = format!("{}:\\", l as char);
            Path::new(&root).exists().then(|| FileEntry {
                name: format!("{}:", l as char),
                path: root,
                kind: EntryKind::Dir,
                size: 0,
                modified: None,
                mode: None,
                owner: None,
                group: None,
                is_link: false,
                link_target: None,
            })
        })
        .collect()
}

pub fn places(app: &AppHandle) -> Vec<Place> {
    let p = app.path();
    let mut out = Vec::new();
    let mut push = |id: &str, path: Option<PathBuf>| {
        if let Some(path) = path {
            if path.exists() || cfg!(mobile) {
                out.push(Place {
                    id: id.to_string(),
                    path: path.to_string_lossy().to_string(),
                });
            }
        }
    };
    if cfg!(mobile) {
        // App private storage is always writable on Android / iOS.
        let docs = p.document_dir().ok().or_else(|| p.app_data_dir().ok());
        if let Some(d) = &docs {
            let _ = std::fs::create_dir_all(d);
        }
        push("documents", docs);
        push("downloads", p.download_dir().ok().filter(|d| d.exists()));
    } else {
        push("home", p.home_dir().ok());
        push("desktop", p.desktop_dir().ok());
        push("documents", p.document_dir().ok());
        push("downloads", p.download_dir().ok());
        #[cfg(windows)]
        out.push(Place {
            id: "computer".into(),
            path: "/".into(),
        });
        #[cfg(not(windows))]
        push("root", Some(PathBuf::from("/")));
    }
    out
}

pub fn default_dir(app: &AppHandle) -> String {
    places(app)
        .into_iter()
        .next()
        .map(|p| p.path)
        .unwrap_or_else(|| "/".into())
}

pub fn mkdir(path: &str) -> AppResult<()> {
    std::fs::create_dir(path)?;
    Ok(())
}

pub fn create_file(path: &str) -> AppResult<()> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    Ok(())
}

pub fn rename(from: &str, to: &str) -> AppResult<()> {
    if Path::new(to).exists() {
        return Err(AppError::new(
            crate::error::ErrorCode::AlreadyExists,
            format!("{to} already exists"),
        ));
    }
    std::fs::rename(from, to)?;
    Ok(())
}

pub fn delete(paths: &[String], use_trash: bool) -> AppResult<()> {
    #[cfg(not(mobile))]
    if use_trash {
        return trash::delete_all(paths)
            .map_err(|e| AppError::new(crate::error::ErrorCode::Io, e.to_string()));
    }
    let _ = use_trash;
    for p in paths {
        let path = Path::new(p);
        let meta = std::fs::symlink_metadata(path)?;
        if meta.is_dir() {
            std::fs::remove_dir_all(path)?;
        } else {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}

pub fn chmod(path: &str, mode: u32) -> AppResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode & 0o7777))?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
        Err(AppError::unsupported("Not supported on this platform"))
    }
}

/// Directories (top-down) and files (with size) below a folder.
pub type Walk = (Vec<PathBuf>, Vec<(PathBuf, u64)>);

/// Recursively collects a local directory.
pub fn walk(root: &Path) -> AppResult<Walk> {
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let ft = entry.file_type()?;
            if ft.is_symlink() {
                // follow file links, skip directory links (loops)
                if let Ok(meta) = std::fs::metadata(entry.path()) {
                    if meta.is_file() {
                        files.push((entry.path(), meta.len()));
                    }
                }
                continue;
            }
            if ft.is_dir() {
                stack.push(entry.path());
                dirs.push(entry.path());
            } else if ft.is_file() {
                files.push((entry.path(), entry.metadata().map(|m| m.len()).unwrap_or(0)));
            }
        }
    }
    Ok((dirs, files))
}
