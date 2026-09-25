//! Site manager persistence and FileZilla import.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::error::{AppError, AppResult};
use crate::model::{AuthMethod, Protocol, Site};
use crate::storage;

pub struct SiteStore {
    path: PathBuf,
    sites: RwLock<Vec<Site>>,
    folders_path: PathBuf,
    /// Folder paths like `Kunden/Müller` (also empty folders)
    folders: RwLock<Vec<String>>,
}

/// Normalizes a folder path: `" Kunden / Müller/"` → `"Kunden/Müller"`.
pub fn normalize_folder(path: &str) -> String {
    path.split('/')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

fn folder_parent(path: &str) -> String {
    path.rsplit_once('/')
        .map(|(p, _)| p.to_string())
        .unwrap_or_default()
}

/// `prefix` itself or anything inside it.
fn is_within(path: &str, prefix: &str) -> bool {
    path == prefix || path.starts_with(&format!("{prefix}/"))
}

impl SiteStore {
    pub fn load(path: PathBuf) -> Self {
        let mut sites: Vec<Site> = storage::read_json(&path).unwrap_or_default();
        for s in &mut sites {
            s.group = normalize_folder(&s.group);
        }
        let folders_path = path.with_file_name("folders.json");
        let folders = storage::read_json(&folders_path).unwrap_or_default();
        let store = Self {
            path,
            sites: RwLock::new(sites),
            folders_path,
            folders: RwLock::new(folders),
        };
        store.register_site_folders();
        store
    }

    /// Makes sure every folder used by a site (and all its parents) is known.
    fn register_site_folders(&self) {
        let groups: Vec<String> = self
            .sites
            .read()
            .unwrap()
            .iter()
            .map(|s| s.group.clone())
            .filter(|g| !g.is_empty())
            .collect();
        let mut folders = self.folders.write().unwrap();
        let before = folders.len();
        for g in groups {
            let mut acc = String::new();
            for seg in g.split('/') {
                acc = if acc.is_empty() {
                    seg.to_string()
                } else {
                    format!("{acc}/{seg}")
                };
                if !folders.contains(&acc) {
                    folders.push(acc.clone());
                }
            }
        }
        if folders.len() != before {
            folders.sort();
            let _ = storage::write_json(&self.folders_path, &*folders);
        }
    }

    pub fn folders(&self) -> Vec<String> {
        self.folders.read().unwrap().clone()
    }

    pub fn create_folder(&self, path: &str) -> AppResult<String> {
        let path = normalize_folder(path);
        if path.is_empty() {
            return Err(AppError::invalid("Folder name is empty"));
        }
        {
            let mut folders = self.folders.write().unwrap();
            let mut acc = String::new();
            for seg in path.split('/') {
                acc = if acc.is_empty() {
                    seg.to_string()
                } else {
                    format!("{acc}/{seg}")
                };
                if !folders.contains(&acc) {
                    folders.push(acc.clone());
                }
            }
            folders.sort();
            storage::write_json(&self.folders_path, &*folders)?;
        }
        Ok(path)
    }

    /// Renames / moves a folder incl. its sub folders and servers.
    pub fn rename_folder(&self, from: &str, to: &str) -> AppResult<()> {
        let from = normalize_folder(from);
        let to = normalize_folder(to);
        if from.is_empty() || to.is_empty() {
            return Err(AppError::invalid("Folder name is empty"));
        }
        if is_within(&to, &from) && to != from {
            return Err(AppError::invalid("A folder cannot be moved into itself"));
        }
        let remap = |p: &str| -> String {
            if is_within(p, &from) {
                format!("{to}{}", &p[from.len()..])
            } else {
                p.to_string()
            }
        };
        {
            let mut folders = self.folders.write().unwrap();
            let mut next: Vec<String> = folders.iter().map(|f| remap(f)).collect();
            next.sort();
            next.dedup();
            *folders = next;
            storage::write_json(&self.folders_path, &*folders)?;
        }
        {
            let mut sites = self.sites.write().unwrap();
            for s in sites.iter_mut() {
                s.group = remap(&s.group);
            }
            self.persist(&sites)?;
        }
        self.register_site_folders();
        Ok(())
    }

    /// Deletes a folder. Its servers and sub folders move to the parent folder.
    pub fn delete_folder(&self, path: &str) -> AppResult<()> {
        let path = normalize_folder(path);
        let parent = folder_parent(&path);
        let lift = |p: &str| -> String {
            if p == path {
                parent.clone()
            } else if is_within(p, &path) {
                let rest = &p[path.len() + 1..];
                if parent.is_empty() {
                    rest.to_string()
                } else {
                    format!("{parent}/{rest}")
                }
            } else {
                p.to_string()
            }
        };
        {
            let mut folders = self.folders.write().unwrap();
            let mut next: Vec<String> = folders
                .iter()
                .filter(|f| **f != path)
                .map(|f| lift(f))
                .filter(|f| !f.is_empty())
                .collect();
            next.sort();
            next.dedup();
            *folders = next;
            storage::write_json(&self.folders_path, &*folders)?;
        }
        let mut sites = self.sites.write().unwrap();
        for s in sites.iter_mut() {
            s.group = lift(&s.group);
        }
        self.persist(&sites)
    }

    pub fn move_sites(&self, ids: &[String], folder: &str) -> AppResult<()> {
        let folder = normalize_folder(folder);
        {
            let mut sites = self.sites.write().unwrap();
            for s in sites.iter_mut().filter(|s| ids.contains(&s.id)) {
                s.group = folder.clone();
            }
            self.persist(&sites)?;
        }
        self.register_site_folders();
        Ok(())
    }

    fn persist(&self, sites: &[Site]) -> AppResult<()> {
        storage::write_json(&self.path, sites)
    }

    pub fn list(&self) -> Vec<Site> {
        self.sites.read().unwrap().clone()
    }

    pub fn get(&self, id: &str) -> Option<Site> {
        self.sites
            .read()
            .unwrap()
            .iter()
            .find(|s| s.id == id)
            .cloned()
    }

    /// Inserts or replaces a site. Assigns an id for new sites.
    pub fn upsert(&self, mut site: Site) -> AppResult<Site> {
        site.group = normalize_folder(&site.group);
        let result = self.upsert_inner(site);
        self.register_site_folders();
        result
    }

    fn upsert_inner(&self, mut site: Site) -> AppResult<Site> {
        let mut sites = self.sites.write().unwrap();
        if site.id.is_empty() {
            site.id = uuid::Uuid::new_v4().to_string();
        }
        if site.created_at == 0 {
            site.created_at = chrono::Utc::now().timestamp_millis();
        }
        match sites.iter_mut().find(|s| s.id == site.id) {
            Some(existing) => *existing = site.clone(),
            None => sites.push(site.clone()),
        }
        self.persist(&sites)?;
        Ok(site)
    }

    pub fn update<F: FnOnce(&mut Site)>(&self, id: &str, f: F) -> AppResult<Option<Site>> {
        let mut sites = self.sites.write().unwrap();
        let Some(site) = sites.iter_mut().find(|s| s.id == id) else {
            return Ok(None);
        };
        f(site);
        let result = site.clone();
        self.persist(&sites)?;
        Ok(Some(result))
    }

    pub fn remove(&self, id: &str) -> AppResult<Option<Site>> {
        let mut sites = self.sites.write().unwrap();
        let idx = sites.iter().position(|s| s.id == id);
        let removed = idx.map(|i| sites.remove(i));
        self.persist(&sites)?;
        Ok(removed)
    }
}

// ---------------------------------------------------------------------------
// FileZilla import (sitemanager.xml)
// ---------------------------------------------------------------------------

pub struct ImportedSite {
    pub site: Site,
    pub password: Option<String>,
}

pub fn default_filezilla_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join("FileZilla/sitemanager.xml"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let home = std::env::var_os("HOME").map(PathBuf::from)?;
        let xdg = home.join(".config/filezilla/sitemanager.xml");
        if xdg.exists() {
            return Some(xdg);
        }
        Some(home.join(".filezilla/sitemanager.xml"))
    }
}

pub fn parse_filezilla(path: &Path) -> AppResult<Vec<ImportedSite>> {
    let xml = std::fs::read_to_string(path)?;
    parse_filezilla_str(&xml)
}

fn parse_filezilla_str(xml: &str) -> AppResult<Vec<ImportedSite>> {
    use base64::Engine;

    let mut reader = Reader::from_str(xml);

    let mut out = Vec::new();
    let mut folders: Vec<String> = Vec::new();
    // element name stack
    let mut stack: Vec<String> = Vec::new();
    let mut fields: Option<std::collections::HashMap<String, String>> = None;
    let mut pass_base64 = false;
    // accumulated character data of the current element
    let mut text = String::new();

    loop {
        let ev = reader
            .read_event()
            .map_err(|e| AppError::invalid(format!("Invalid sitemanager.xml: {e}")))?;
        match ev {
            Event::Start(e) => {
                let name = e.local_name().as_ref().to_string();
                // The folder name is the text before the first child element
                if stack.last().map(String::as_str) == Some("Folder") {
                    if let Some(last) = folders.last_mut() {
                        if last.is_empty() {
                            *last = text.trim().to_string();
                        }
                    }
                }
                match name.as_str() {
                    "Server" => fields = Some(Default::default()),
                    "Folder" => folders.push(String::new()),
                    "Pass" => {
                        pass_base64 = e
                            .attributes()
                            .flatten()
                            .any(|a| a.key.as_ref() == "encoding" && a.value.as_ref() == "base64")
                    }
                    _ => {}
                }
                text.clear();
                stack.push(name);
            }
            Event::End(_) => {
                let name = stack.pop().unwrap_or_default();
                match name.as_str() {
                    "Server" => {
                        if let Some(f) = fields.take() {
                            let group = folders
                                .iter()
                                .filter(|f| !f.is_empty())
                                .map(|f| f.replace('/', "-"))
                                .collect::<Vec<_>>()
                                .join("/");
                            if let Some(s) = filezilla_site(&f, group, pass_base64) {
                                out.push(s);
                            }
                        }
                    }
                    "Folder" => {
                        folders.pop();
                    }
                    field => {
                        if let Some(f) = fields.as_mut() {
                            f.insert(field.to_string(), text.trim().to_string());
                        }
                    }
                }
                text.clear();
            }
            Event::Text(t) => text.push_str(&t.xml10_content()),
            Event::GeneralRef(r) => {
                if let Ok(Some(c)) = r.resolve_char_ref() {
                    text.push(c);
                } else {
                    let name = r.xml10_content();
                    text.push_str(match name.as_ref() {
                        "amp" => "&",
                        "lt" => "<",
                        "gt" => ">",
                        "quot" => "\"",
                        "apos" => "'",
                        _ => "",
                    });
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }

    fn filezilla_site(
        f: &std::collections::HashMap<String, String>,
        group: String,
        pass_base64: bool,
    ) -> Option<ImportedSite> {
        let host = f.get("Host")?.trim().to_string();
        let protocol = match f.get("Protocol").map(String::as_str).unwrap_or("0") {
            "1" => Protocol::Sftp,
            "3" => Protocol::FtpsImplicit,
            "4" => Protocol::Ftps,
            "6" => Protocol::Ftp,
            "0" => Protocol::Ftp,
            _ => return None, // cloud protocols of FileZilla Pro are not supported
        };
        let logon = f.get("Logontype").map(String::as_str).unwrap_or("1");
        let auth = match logon {
            "0" => AuthMethod::Anonymous,
            "5" => AuthMethod::Key,
            _ => AuthMethod::Password,
        };
        let password = f.get("Pass").and_then(|p| {
            if pass_base64 {
                base64::engine::general_purpose::STANDARD
                    .decode(p.trim())
                    .ok()
                    .and_then(|b| String::from_utf8(b).ok())
            } else {
                Some(p.clone())
            }
        });
        let site = Site {
            name: f.get("Name").cloned().unwrap_or_default(),
            protocol,
            port: f.get("Port").and_then(|p| p.parse().ok()),
            username: f.get("User").cloned().unwrap_or_default(),
            auth,
            key_path: f.get("Keyfile").cloned().filter(|k| !k.is_empty()),
            remote_path: f
                .get("RemoteDir")
                .map(|r| parse_filezilla_remote_dir(r))
                .unwrap_or_default(),
            local_path: f.get("LocalDir").cloned().unwrap_or_default(),
            group,
            notes: f.get("Comments").cloned().unwrap_or_default(),
            passive: f
                .get("PasvMode")
                .map(|m| m != "MODE_ACTIVE")
                .unwrap_or(true),
            host,
            ..Site::default()
        };
        Some(ImportedSite {
            password: password.filter(|p| !p.is_empty()),
            site,
        })
    }

    Ok(out)
}

/// FileZilla stores remote dirs like `1 0 4 home 4 user` (type, prefix length, then
/// length-prefixed segments).
fn parse_filezilla_remote_dir(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut pos = 0;
    let read_num = |pos: &mut usize| -> Option<usize> {
        while *pos < chars.len() && chars[*pos] == ' ' {
            *pos += 1;
        }
        let start = *pos;
        while *pos < chars.len() && chars[*pos].is_ascii_digit() {
            *pos += 1;
        }
        chars[start..*pos].iter().collect::<String>().parse().ok()
    };
    let Some(_server_type) = read_num(&mut pos) else {
        return String::new();
    };
    let Some(prefix_len) = read_num(&mut pos) else {
        return String::new();
    };
    if prefix_len > 0 {
        pos += 1 + prefix_len;
    }
    let mut segments = Vec::new();
    while pos < chars.len() {
        let Some(len) = read_num(&mut pos) else { break };
        pos += 1; // space
        if pos + len > chars.len() {
            break;
        }
        segments.push(chars[pos..pos + len].iter().collect::<String>());
        pos += len;
    }
    if segments.is_empty() {
        String::new()
    } else {
        format!("/{}", segments.join("/"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> SiteStore {
        let dir = std::env::temp_dir().join(format!("sftpinguin-sites-{}", uuid::Uuid::new_v4()));
        SiteStore::load(dir.join("sites.json"))
    }

    fn add(store: &SiteStore, name: &str, group: &str) -> String {
        store
            .upsert(Site {
                name: name.into(),
                group: group.into(),
                ..Site::default()
            })
            .unwrap()
            .id
    }

    #[test]
    fn folders() {
        assert_eq!(normalize_folder(" Kunden / Müller/"), "Kunden/Müller");
        let store = temp_store();
        let a = add(&store, "a", "Kunden/Müller");
        let b = add(&store, "b", "Kunden");
        add(&store, "c", "");
        assert_eq!(store.folders(), vec!["Kunden", "Kunden/Müller"]);

        store.create_folder("Privat/NAS").unwrap();
        assert!(store.folders().contains(&"Privat".to_string()));

        store.rename_folder("Kunden", "Firma").unwrap();
        assert_eq!(store.get(&a).unwrap().group, "Firma/Müller");
        assert_eq!(store.get(&b).unwrap().group, "Firma");
        assert!(!store.folders().iter().any(|f| f.starts_with("Kunden")));
        assert!(store.rename_folder("Firma", "Firma/Sub").is_err());

        store
            .move_sites(std::slice::from_ref(&b), "Privat/NAS")
            .unwrap();
        assert_eq!(store.get(&b).unwrap().group, "Privat/NAS");

        store.delete_folder("Firma").unwrap();
        assert_eq!(store.get(&a).unwrap().group, "Müller");
        assert!(store.folders().contains(&"Müller".to_string()));
        assert!(!store.folders().contains(&"Firma".to_string()));

        // persisted
        let reloaded = SiteStore::load(store.path.clone());
        assert_eq!(reloaded.get(&b).unwrap().group, "Privat/NAS");
        assert!(reloaded.folders().contains(&"Privat/NAS".to_string()));
    }

    #[test]
    fn remote_dir() {
        assert_eq!(
            parse_filezilla_remote_dir("1 0 4 home 4 user"),
            "/home/user"
        );
        assert_eq!(parse_filezilla_remote_dir("1 0 6 my dir"), "/my dir");
        assert_eq!(parse_filezilla_remote_dir(""), "");
    }

    #[test]
    fn import_xml() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<FileZilla3 version="3.66">
  <Servers>
    <Server>
      <Host>example.org</Host><Port>22</Port><Protocol>1</Protocol><Type>0</Type>
      <User>alice</User><Pass encoding="base64">c2VjcmV0</Pass><Logontype>1</Logontype>
      <Name>Web &amp; Co</Name><RemoteDir>1 0 3 var 3 www</RemoteDir>
    </Server>
    <Folder expanded="1">Kunden
      <Server>
        <Host>ftp.example.com</Host><Port>21</Port><Protocol>4</Protocol>
        <User>bob</User><Logontype>2</Logontype><Name>Bob</Name>
      </Server>
    </Folder>
  </Servers>
</FileZilla3>"#;
        let sites = parse_filezilla_str(xml).unwrap();
        assert_eq!(sites.len(), 2);
        assert_eq!(sites[0].site.name, "Web & Co");
        assert_eq!(sites[0].site.protocol, Protocol::Sftp);
        assert_eq!(sites[0].password.as_deref(), Some("secret"));
        assert_eq!(sites[0].site.remote_path, "/var/www");
        assert_eq!(sites[1].site.group, "Kunden");
        assert_eq!(sites[1].site.protocol, Protocol::Ftps);
        assert!(sites[1].password.is_none());
    }
}
