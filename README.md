<p align="center">
  <img src="src-tauri/icons/logo.svg" width="96" alt="SFTPinguin" />
</p>

<h1 align="center">SFTPinguin</h1>

<p align="center">
  Ein moderner, minimalistischer Dateitransfer-Client – wie FileZilla oder Cyberduck, nur aufgeräumt.<br/>
  <b>SFTP · FTP · FTPS · WebDAV · S3</b> – für Windows, macOS, Linux, Android und iOS.
</p>

![SFTPinguin – helles Design](docs/screenshot-light.png)

## Funktionen

**Protokolle**

| Protokoll | Details |
|---|---|
| **SFTP** (SSH) | Passwort, Keyboard-Interactive, SSH-Schlüssel (Datei oder eingefügt, auch mit Passphrase), SSH-Agent (ssh-agent / Pageant). Host-Key-Prüfung mit Fingerabdruck und Warnung bei geändertem Schlüssel. |
| **FTP** | Passiver/aktiver Modus, MLSD und klassisches `LIST` (Unix/DOS), anonymer Login, UTF-8 |
| **FTPS** | Explizit (AUTH TLS) und implizit (Port 990), TLS-Session-Reuse (z. B. vsftpd), optional selbstsignierte Zertifikate |
| **WebDAV** | HTTP und HTTPS – z. B. Nextcloud, ownCloud, Synology, Apache, nginx |
| **S3** | Amazon S3 und kompatible Dienste (MinIO, Cloudflare R2, Wasabi, Backblaze B2, Hetzner …), Bucket-Übersicht, Path-Style |

**Server-Manager**
- Server speichern mit Name, Gruppe/Ordner, Farbe, Favoriten, Notizen, Startverzeichnissen
- Passwörter sicher im Schlüsselbund des Betriebssystems (Windows Credential Manager, macOS Keychain, Secret Service/GNOME Keyring/KWallet); auf Handys im geschützten App-Speicher
- Schnellverbindung – auch mit URLs wie `sftp://user@host:2222/pfad`
- **Import aus FileZilla** (`sitemanager.xml` inkl. Ordnern und Passwörtern)

**Dateien & Übertragungen**
- Zwei Fenster (Lokal ↔ Server), mehrere Verbindungen gleichzeitig in Tabs
- Drag & Drop zwischen den Fenstern, in Unterordner, und Dateien direkt aus dem Explorer/Finder
- Übertragungswarteschlange mit Fortschritt, Geschwindigkeit, Restzeit, Abbrechen und Wiederholen
- Parallele Übertragungen (einstellbar), ganze Ordner rekursiv
- Bei vorhandenen Dateien: Überschreiben, Überspringen, nur neuere, beide behalten
- Änderungsdatum bleibt erhalten, Downloads laufen über `.part`-Dateien
- Remote-Dateien **direkt bearbeiten**: Datei öffnet sich im Standardprogramm, Änderungen werden erkannt und auf Wunsch hochgeladen
- Umbenennen, Löschen, neue Ordner/Dateien, Rechte ändern (chmod, auch rekursiv), Pfad kopieren, Filter, Sortierung, versteckte Dateien
- Tastatur: Pfeiltasten, Enter, Backspace, Entf, F2, F5, Strg+A, Strg+F, Strg+T/W/Tab, Strg+, und Tipp-Suche
- Automatisches Wiederverbinden nach Verbindungsabbruch, Protokoll-Ansicht

**Oberfläche**
- Hell/Dunkel (oder automatisch), sechs Akzentfarben
- Deutsch und Englisch (automatisch nach Systemsprache)
- Responsives Handy-Layout mit Umschalter Server/Lokal und Seitenmenü

| Dunkel | Handy |
|---|---|
| ![Dunkles Design](docs/screenshot-dark.png) | ![Handy](docs/screenshot-mobile.png) |

## Technik

- **[Tauri 2](https://tauri.app)** + **Rust** im Backend, **React + TypeScript** (Vite) im Frontend
- SFTP: [`russh`](https://crates.io/crates/russh) + [`russh-sftp`](https://crates.io/crates/russh-sftp) (reines Rust)
- FTP/FTPS: [`suppaftp`](https://crates.io/crates/suppaftp) mit rustls
- WebDAV: eigene Implementierung auf Basis von `reqwest`
- S3: [`rust-s3`](https://crates.io/crates/rust-s3)
- Kryptografie durchgehend über `ring`/rustls – keine OpenSSL-Abhängigkeit, dadurch einfache Builds für alle Plattformen inkl. Android/iOS

```
src/                 React-Oberfläche
  components/        Seitenleiste, Dateifenster, Server-Editor, Übertragungen, …
  lib/               API, Zustand (zustand), Übersetzungen, Aktionen
src-tauri/src/
  remote/            Protokolle (sftp.rs, ftp.rs, webdav.rs, s3.rs) hinter einem gemeinsamen Trait
  session.rs         Verbindungen inkl. Pool für parallele Übertragungen
  transfer.rs        Übertragungswarteschlange
  sites.rs           Server-Manager & FileZilla-Import
  secrets.rs         Passwortspeicher (Schlüsselbund / App-Sandbox)
  known_hosts.rs     Vertrauenswürdige SSH-Host-Keys
```

## Entwicklung

Voraussetzungen: [Node.js 20+](https://nodejs.org), [Rust](https://rustup.rs) und die
[Tauri-Systemvoraussetzungen](https://tauri.app/start/prerequisites/) für dein Betriebssystem.

```sh
npm install
npm run tauri dev        # App im Entwicklungsmodus starten
npm run tauri build      # Installer für das aktuelle System bauen
```

### Android

Benötigt Android Studio / SDK, NDK und Java 17 ([Anleitung](https://tauri.app/start/prerequisites/#android)).

```sh
rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android
npm run tauri android init
npm run tauri android dev        # auf Emulator oder Gerät
npm run tauri android build      # APK / AAB
```

### iOS

Nur auf macOS mit Xcode ([Anleitung](https://tauri.app/start/prerequisites/#ios)).

```sh
rustup target add aarch64-apple-ios x86_64-apple-ios aarch64-apple-ios-sim
npm run tauri ios init
npm run tauri ios dev
npm run tauri ios build          # benötigt ein Apple-Developer-Team zum Signieren
```

> Hinweis zu Handys: Die App arbeitet dort mit ihrem eigenen Dokumente-Ordner als lokale Seite.
> SSH-Agent-Anmeldung gibt es nur auf dem Desktop.

### Tests

```sh
cd src-tauri
cargo test                                   # Unit-Tests
```

Integrationstests gegen echte Server (SFTP, FTP, FTPS, WebDAV, WebDAV über HTTPS, S3) laufen mit lokalen
Testservern (Linux, als root):

```sh
sudo scripts/test-servers.sh
SFTPINGUIN_IT=1 SFTPINGUIN_IT_KEYS=/tmp/sftpinguin-test/ssh \
  cargo test --manifest-path src-tauri/Cargo.toml -- --include-ignored --test-threads=1
```

### Releases

Der Workflow `.github/workflows/release.yml` baut beim Pushen eines Tags (`v0.1.0`) Installer für
Windows (`.msi`/`.exe`), macOS (Apple Silicon & Intel, `.dmg`), Linux (`.deb`, `.rpm`, `.AppImage`)
sowie eine Android-APK und legt einen Release-Entwurf an.
