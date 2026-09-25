//! Integration tests against real servers. They are ignored by default; run them with
//! local test servers (see `scripts/test-servers.sh`):
//!
//! ```sh
//! SFTPINGUIN_IT=1 cargo test -- --ignored --test-threads=1
//! ```

use crate::error::ErrorCode;
use crate::known_hosts::KnownHosts;
use crate::model::{AuthMethod, ConnectConfig, EntryKind, Protocol, Site};

use super::{connect, join, RemoteHandle};

fn env(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn site(protocol: Protocol, port: u16) -> Site {
    Site {
        id: "test".into(),
        name: "test".into(),
        protocol,
        host: env("SFTPINGUIN_IT_HOST", "127.0.0.1"),
        port: Some(port),
        username: env("SFTPINGUIN_IT_USER", "tester"),
        timeout: 10,
        ..Site::default()
    }
}

fn cfg(site: Site) -> ConnectConfig {
    ConnectConfig {
        site,
        password: Some(env("SFTPINGUIN_IT_PASS", "secret")),
        passphrase: None,
        key_data: None,
    }
}

fn payload(len: usize) -> Vec<u8> {
    // deterministic pseudo random data
    let mut x: u32 = 0x1234_5678;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x & 0xff) as u8
        })
        .collect()
}

/// Full round trip: mkdir, upload, list, stat, download, rename, chmod, delete.
async fn exercise(fs: RemoteHandle, base: &str) {
    let dir = join(base, &format!("it-{}", uuid::Uuid::new_v4().simple()));
    fs.mkdir(&dir).await.expect("mkdir");

    let data = payload(3 * 1024 * 1024 + 17);
    let file = join(&dir, "Datei mit Ümlaut.bin");
    let mut src: &[u8] = &data;
    fs.upload(&mut src, &file, data.len() as u64)
        .await
        .expect("upload");

    let list = fs.list(&dir).await.expect("list");
    let entry = list
        .iter()
        .find(|e| e.name == "Datei mit Ümlaut.bin")
        .unwrap_or_else(|| panic!("uploaded file missing in listing: {list:?}"));
    assert_eq!(entry.kind, EntryKind::File);
    assert_eq!(entry.size, data.len() as u64);

    let st = fs.stat(&file).await.expect("stat").expect("stat some");
    assert_eq!(st.size, data.len() as u64);
    assert!(fs
        .stat(&join(&dir, "missing.txt"))
        .await
        .expect("stat missing")
        .is_none());

    let mut out = Vec::new();
    fs.download(&file, &mut out).await.expect("download");
    assert!(out == data, "downloaded data differs");

    let renamed = join(&dir, "renamed.bin");
    fs.rename(&file, &renamed).await.expect("rename");
    let list = fs.list(&dir).await.expect("list after rename");
    assert!(list.iter().any(|e| e.name == "renamed.bin"));
    assert!(!list.iter().any(|e| e.name == "Datei mit Ümlaut.bin"));

    if fs.capabilities().chmod {
        fs.chmod(&renamed, 0o640).await.expect("chmod");
        let st = fs.stat(&renamed).await.unwrap().unwrap();
        if let Some(mode) = st.mode {
            assert_eq!(mode & 0o777, 0o640);
        }
    }

    let sub = join(&dir, "sub");
    fs.mkdir(&sub).await.expect("mkdir sub");
    let list = fs.list(&dir).await.unwrap();
    assert!(
        list.iter().any(|e| e.name == "sub" && e.is_dir()),
        "{list:?}"
    );

    fs.remove_file(&renamed).await.expect("remove file");
    fs.remove_dir(&sub).await.expect("remove sub");
    fs.remove_dir(&dir).await.expect("remove dir");
    let parent = fs.list(base).await.expect("list base");
    assert!(!parent.iter().any(|e| e.path == dir), "dir not deleted");
    fs.close().await;
}

fn temp_known_hosts() -> KnownHosts {
    let dir = std::env::temp_dir().join(format!("sftpinguin-kh-{}", uuid::Uuid::new_v4()));
    KnownHosts::load(dir.join("known_hosts.json"))
}

fn enabled() -> bool {
    let _ = rustls::crypto::ring::default_provider().install_default();
    std::env::var("SFTPINGUIN_IT").is_ok()
}

fn trust_all(kh: &KnownHosts, err: &crate::error::AppError) {
    let presented = err.details.as_ref().unwrap()["presented"].clone();
    kh.trust(serde_json::from_value(presented).unwrap())
        .unwrap();
}

#[tokio::test]
#[ignore]
async fn sftp_password_and_host_key() {
    if !enabled() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("sftpinguin-kh-{}", uuid::Uuid::new_v4()));
    let kh = KnownHosts::load(dir.join("known_hosts.json"));
    let c = cfg(site(Protocol::Sftp, 2222));

    // first connection: unknown host key
    let err = connect(&c, &kh).await.err().expect("must ask for host key");
    assert_eq!(err.code, ErrorCode::HostKeyUnknown);
    trust_all(&kh, &err);

    // wrong password
    let mut bad = c.clone();
    bad.password = Some("wrong".into());
    let err = connect(&bad, &kh)
        .await
        .err()
        .expect("wrong password must fail");
    assert_eq!(err.code, ErrorCode::AuthFailed);

    // missing password
    let mut none = c.clone();
    none.password = None;
    let err = connect(&none, &kh).await.err().unwrap();
    assert_eq!(err.code, ErrorCode::PasswordRequired);

    let fs = connect(&c, &kh).await.expect("connect");
    let home = fs.home().await.unwrap();
    assert!(home.starts_with('/'));
    exercise(fs, &home).await;

    // changed host key detection
    let entry = kh.list().remove(0);
    kh.trust(crate::known_hosts::HostKey {
        fingerprint: "SHA256:bogus".into(),
        ..entry
    })
    .unwrap();
    let err = connect(&c, &kh).await.err().unwrap();
    assert_eq!(err.code, ErrorCode::HostKeyChanged);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
#[ignore]
async fn sftp_key_auth() {
    if !enabled() {
        return;
    }
    let keys = env("SFTPINGUIN_IT_KEYS", "/tmp/sftpinguin-test/ssh");
    let kh = temp_known_hosts();
    let mut s = site(Protocol::Sftp, 2222);
    s.auth = AuthMethod::Key;
    s.key_path = Some(format!("{keys}/client_key"));
    let mut c = cfg(s.clone());
    c.password = None;
    let err = connect(&c, &kh).await.err().unwrap();
    trust_all(&kh, &err);
    let fs = connect(&c, &kh).await.expect("key auth");
    fs.close().await;

    // pasted key content instead of a path
    let mut pasted = c.clone();
    pasted.site.key_path = None;
    pasted.key_data = Some(std::fs::read_to_string(format!("{keys}/client_key")).unwrap());
    connect(&pasted, &kh)
        .await
        .expect("pasted key auth")
        .close()
        .await;

    // encrypted key
    let mut enc = c.clone();
    enc.site.key_path = Some(format!("{keys}/client_key_enc"));
    let err = connect(&enc, &kh).await.err().unwrap();
    assert_eq!(err.code, ErrorCode::PassphraseRequired);
    enc.passphrase = Some("wrong".into());
    let err = connect(&enc, &kh).await.err().unwrap();
    assert_eq!(err.code, ErrorCode::PassphraseRequired, "{err:?}");
    enc.passphrase = Some("keypass".into());
    connect(&enc, &kh)
        .await
        .expect("encrypted key")
        .close()
        .await;
}

#[tokio::test]
#[ignore]
async fn ftp_plain() {
    if !enabled() {
        return;
    }
    let kh = KnownHosts::from_entries(vec![]);
    let c = cfg(site(Protocol::Ftp, 2121));
    let fs = connect(&c, &kh).await.expect("connect");
    let home = fs.home().await.unwrap();
    exercise(fs, &home).await;

    let mut bad = c.clone();
    bad.password = Some("wrong".into());
    let err = connect(&bad, &kh).await.err().unwrap();
    assert_eq!(err.code, ErrorCode::AuthFailed, "{err:?}");

    let mut anon = site(Protocol::Ftp, 2121);
    anon.auth = AuthMethod::Anonymous;
    let fs = connect(&cfg(anon), &kh).await.expect("anonymous");
    fs.list("/").await.expect("anonymous list");
}

#[tokio::test]
#[ignore]
async fn ftps_explicit() {
    if !enabled() {
        return;
    }
    let kh = KnownHosts::from_entries(vec![]);
    let mut s = site(Protocol::Ftps, 2990);
    s.host = "localhost".into();
    // self-signed test certificate must be rejected by default ...
    let err = connect(&cfg(s.clone()), &kh)
        .await
        .err()
        .expect("self-signed must fail");
    assert_eq!(err.code, ErrorCode::Connection, "{err:?}");
    // ... and accepted when explicitly allowed
    s.insecure_tls = true;
    let fs = connect(&cfg(s), &kh).await.expect("connect ftps");
    let home = fs.home().await.unwrap();
    exercise(fs, &home).await;
}

#[tokio::test]
#[ignore]
async fn webdav() {
    if !enabled() {
        return;
    }
    let kh = KnownHosts::from_entries(vec![]);
    let mut s = site(Protocol::Webdav, 8080);
    s.remote_path = "/dav".into();
    let fs = connect(&cfg(s.clone()), &kh).await.expect("connect webdav");
    let home = fs.home().await.unwrap();
    assert_eq!(home, "/dav");
    exercise(fs, &home).await;

    let mut bad = cfg(s);
    bad.password = Some("wrong".into());
    let err = connect(&bad, &kh).await.err().unwrap();
    assert_eq!(err.code, ErrorCode::AuthFailed);
}

#[tokio::test]
#[ignore]
async fn s3() {
    if !enabled() {
        return;
    }
    let kh = KnownHosts::from_entries(vec![]);
    let mut s = site(Protocol::S3, 0);
    s.port = None;
    s.host = String::new();
    s.endpoint = env("SFTPINGUIN_IT_S3", "http://127.0.0.1:5000");
    s.region = "us-east-1".into();
    s.username = "testing".into();
    let mut c = cfg(s);
    c.password = Some("testing".into());
    let fs = connect(&c, &kh).await.expect("connect s3");
    let buckets = fs.list("/").await.expect("list buckets");
    assert!(
        buckets.iter().any(|b| b.name == "testbucket"),
        "{buckets:?}"
    );
    exercise(fs, "/testbucket").await;
}

#[tokio::test]
#[ignore]
async fn webdav_https() {
    if !enabled() {
        return;
    }
    let kh = KnownHosts::from_entries(vec![]);
    let mut s = site(Protocol::Webdavs, 8443);
    s.host = "localhost".into();
    s.remote_path = "/dav".into();
    let err = connect(&cfg(s.clone()), &kh)
        .await
        .err()
        .expect("self-signed must fail");
    assert_eq!(err.code, ErrorCode::Connection, "{err:?}");
    assert!(err.message.contains("certificate"), "{}", err.message);
    s.insecure_tls = true;
    let fs = connect(&cfg(s), &kh).await.expect("connect webdavs");
    exercise(fs, "/dav").await;
}
