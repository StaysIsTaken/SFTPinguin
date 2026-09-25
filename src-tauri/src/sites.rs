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
}

impl SiteStore {
    pub fn load(path: PathBuf) -> Self {
        let sites = storage::read_json(&path).unwrap_or_default();
        Self {
            path,
            sites: RwLock::new(sites),
        }
    }

    fn persist(&self, sites: &[Site]) -> AppResult<()> {
        storage::write_json(&self.path, sites)
    }

    pub fn list(&self) -> Vec<Site> {
        self.sites.read().unwrap().clone()
    }

    pub fn get(&self, id: &str) -> Option<Site> {
        self.sites.read().unwrap().iter().find(|s| s.id == id).cloned()
    }

    /// Inserts or replaces a site. Assigns an id for new sites.
    pub fn upsert(&self, mut site: Site) -> AppResult<Site> {
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
                        pass_base64 = e.attributes().flatten().any(|a| {
                            a.key.as_ref() == "encoding" && a.value.as_ref() == "base64"
                        })
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
                                .cloned()
                                .collect::<Vec<_>>()
                                .join(" / ");
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
            passive: f.get("PasvMode").map(|m| m != "MODE_ACTIVE").unwrap_or(true),
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

    #[test]
    fn remote_dir() {
        assert_eq!(parse_filezilla_remote_dir("1 0 4 home 4 user"), "/home/user");
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
