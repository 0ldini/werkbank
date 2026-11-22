//! SFTP über SSH – mit russh (SSH) und russh-sftp (Dateiübertragung), beides reines Rust.
//! Die Bibliotheken arbeiten asynchron; hier sitzt eine synchrone Hülle um sie, damit SFTP
//! genau wie FTP über die Schnittstelle `Remote` aus eigenen Threads benutzt werden kann.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use russh::client;
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh_sftp::client::fs::Metadata;
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::OpenFlags;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::model::{perms_from_mode, rjoin, trust_check, Entry, RErr, Res, Site, TrustCheck, MAX_LIST_ENTRIES};
use crate::remote::{log_line, Prog, Remote, Tick};
use crate::sprache::{t, tf};

const CHUNK: usize = 128 * 1024;

/// Höchstdauer für Verbindungsaufbau samt Anmeldung.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(45);
/// Höchstdauer einer einzelnen Anfrage (Liste, Umbenennen, ein Datenblock …).
const OP_TIMEOUT: Duration = Duration::from_secs(60);
/// Höchstdauer für eine ganze Ordnerliste (grosse Ordner mit vielen Verknüpfungen).
const LIST_TIMEOUT: Duration = Duration::from_secs(300);
/// Antwortzeit pro SFTP-Paket (russh-sftp), in Sekunden.
const SFTP_REQ_TIMEOUT: u64 = 30;

/// Gemeinsame Laufzeit für alle SSH-Verbindungen (zwei Hilfsthreads reichen). Lässt sie sich
/// nicht starten (sehr selten: keine Threads mehr), gibt es eine Fehlermeldung statt eines Absturzes.
fn rt() -> Res<&'static tokio::runtime::Runtime> {
    static RT: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("ftpwerk-ssh")
            .enable_all()
            .build()
            .map_err(|e| e.to_string())
    })
    .as_ref()
    .map_err(|e| RErr::Msg(tf("SSH konnte nicht gestartet werden: {e}", &[("e", e)])))
}

/// Ein Schritt einer Übertragung (ein Datenblock, Öffnen, Schliessen …), höchstens `OP_TIMEOUT`.
async fn step<T, E: std::fmt::Display>(f: impl std::future::Future<Output = Result<T, E>>) -> Res<T> {
    match tokio::time::timeout(OP_TIMEOUT, f).await {
        Ok(r) => r.map_err(|e| serr("", e)),
        Err(_) => Err(RErr::Disconnected(t("Der Server antwortet nicht (Zeitüberschreitung).").to_string())),
    }
}

/// Führt `f` auf der SSH-Laufzeit aus, höchstens `limit` lang.
fn run<T>(limit: Duration, f: impl std::future::Future<Output = Res<T>>) -> Res<T> {
    rt()?.block_on(async move {
        match tokio::time::timeout(limit, f).await {
            Ok(r) => r,
            Err(_) => Err(RErr::Disconnected(t("Der Server antwortet nicht (Zeitüberschreitung).").to_string())),
        }
    })
}

/// Server-Schlüssel: Art und Fingerabdruck («SHA256:…»).
fn describe(k: &PublicKeyOrCertificate) -> (String, String) {
    match k {
        PublicKeyOrCertificate::PublicKey { key, .. } => (key.algorithm().to_string(), key.fingerprint(HashAlg::Sha256).to_string()),
        PublicKeyOrCertificate::Certificate(c) => (c.algorithm().to_string(), c.public_key().fingerprint(HashAlg::Sha256).to_string()),
    }
}

struct Handler {
    trusted: Vec<String>,
    /// Hat der Server einen unbekannten Schlüssel gezeigt, steht er hier für die Rückfrage
    /// (Art, Fingerabdruck, geändert).
    found: Arc<Mutex<Option<(String, String, bool)>>>,
}

impl client::Handler for Handler {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let (algo, fp) = describe(key);
        match trust_check(&self.trusted, &fp, false) {
            TrustCheck::Accept => Ok(true),
            TrustCheck::Ask { changed } => {
                // Nie stillschweigend annehmen: abbrechen und den Benutzer fragen.
                *self.found.lock().unwrap_or_else(|e| e.into_inner()) = Some((algo, fp, changed));
                Ok(false)
            }
        }
    }
}

/// Fehlertext prüfen: sieht es nach abgerissener Verbindung aus?
fn serr(what: &str, e: impl std::fmt::Display) -> RErr {
    let msg = e.to_string();
    let low = msg.to_lowercase();
    let lost = ["disconnect", "closed", "send error", "senderror", "timeout", "timed out", "broken pipe", "reset", "recverror", "eof", "channel"]
        .iter()
        .any(|k| low.contains(k));
    let text = if what.is_empty() { msg } else { format!("{what}: {msg}") };
    if lost {
        RErr::Disconnected(text)
    } else {
        RErr::Msg(text)
    }
}

pub struct Sftp {
    session: client::Handle<Handler>,
    sftp: SftpSession,
}

impl Sftp {
    pub fn connect(site: &Site, trusted: &[String], log: Option<Sender<String>>) -> Res<Sftp> {
        run(CONNECT_TIMEOUT, Self::connect_async(site, trusted, log))
    }

    async fn connect_async(site: &Site, trusted: &[String], log: Option<Sender<String>>) -> Res<Sftp> {
        let host = site.host.trim().to_string();
        let found = Arc::new(Mutex::new(None));
        let handler = Handler { trusted: trusted.to_vec(), found: found.clone() };
        let config = client::Config {
            inactivity_timeout: Some(Duration::from_secs(1800)),
            keepalive_interval: Some(Duration::from_secs(30)),
            keepalive_max: 4,
            ..Default::default()
        };
        log_line(&log, tf("Verbinde mit {host}:{port} (SSH) …", &[("host", &host), ("port", &site.port)]));

        let connecting = client::connect(Arc::new(config), (host.as_str(), site.port), handler);
        let mut session = match tokio::time::timeout(Duration::from_secs(20), connecting).await {
            Err(_) => return Err(RErr::Msg(tf("Verbindung zu {host}:{port} dauert zu lange.", &[("host", &host), ("port", &site.port)]))),
            Ok(Ok(s)) => s,
            Ok(Err(e)) => {
                let seen = found.lock().unwrap_or_else(|e| e.into_inner()).clone();
                if let Some((algo, fp, changed)) = seen {
                    log_line(&log, tf("Server-Schlüssel {algo}: {fp}", &[("algo", &algo), ("fp", &fp)]));
                    return Err(RErr::HostKey { algo, fp, changed });
                }
                return Err(RErr::Msg(tf("Verbindung zu {host}:{port} fehlgeschlagen: {e}", &[("host", &host), ("port", &site.port), ("e", &e)])));
            }
        };
        log_line(&log, t("SSH-Verbindung steht, melde an …").to_string());

        let user = if site.user.trim().is_empty() { "anonymous".to_string() } else { site.user.trim().to_string() };
        let ok = if !site.key_file.trim().is_empty() {
            let pass = if site.password.is_empty() { None } else { Some(site.password.as_str()) };
            let key = russh::keys::load_secret_key(site.key_file.trim(), pass)
                .map_err(|e| RErr::Msg(tf("Schlüsseldatei nicht lesbar: {e}", &[("e", &e)])))?;
            let hash = session.best_supported_rsa_hash().await.map_err(|e| serr("", e))?.flatten();
            session
                .authenticate_publickey(user.clone(), PrivateKeyWithHashAlg::new(Arc::new(key), hash))
                .await
                .map_err(|e| serr("", e))?
                .success()
        } else {
            let mut ok = session.authenticate_password(user.clone(), site.password.clone()).await.map_err(|e| serr("", e))?.success();
            if !ok {
                ok = keyboard_interactive(&mut session, &user, &site.password).await?;
            }
            ok
        };
        if !ok {
            return Err(RErr::Msg(t("Anmeldung fehlgeschlagen: Benutzer oder Passwort stimmen nicht.").to_string()));
        }
        log_line(&log, t("Angemeldet, öffne SFTP …").to_string());

        let channel = session.channel_open_session().await.map_err(|e| serr("", e))?;
        channel.request_subsystem(true, "sftp").await.map_err(|e| serr(t("SFTP wird vom Server nicht angeboten"), e))?;
        let sftp = SftpSession::new(channel.into_stream()).await.map_err(|e| serr("SFTP", e))?;
        sftp.set_timeout(SFTP_REQ_TIMEOUT);
        Ok(Sftp { session, sftp })
    }
}

/// Manche Server nehmen das Passwort nur über «keyboard-interactive» an.
async fn keyboard_interactive(session: &mut client::Handle<Handler>, user: &str, password: &str) -> Res<bool> {
    use client::KeyboardInteractiveAuthResponse as R;
    let mut resp = session.authenticate_keyboard_interactive_start(user.to_string(), None).await.map_err(|e| serr("", e))?;
    for _ in 0..6 {
        match resp {
            R::Success => return Ok(true),
            R::Failure { .. } => return Ok(false),
            R::InfoRequest { prompts, .. } => {
                let answers: Vec<String> = prompts.iter().map(|_| password.to_string()).collect();
                resp = session.authenticate_keyboard_interactive_respond(answers).await.map_err(|e| serr("", e))?;
            }
        }
    }
    Ok(false)
}

fn entry_from(name: String, md: &Metadata, is_dir: bool, is_link: bool) -> Entry {
    let mode = md.permissions.map(|m| m & 0o7777);
    Entry {
        name,
        is_dir,
        is_link,
        size: md.size.unwrap_or(0),
        mtime: md.mtime.map(i64::from),
        perms: mode.map(perms_from_mode).unwrap_or_default(),
        mode,
    }
}

impl Remote for Sftp {
    fn pwd(&mut self) -> Res<String> {
        run(OP_TIMEOUT, async { self.sftp.canonicalize(".").await.map_err(|e| serr("", e)) })
    }

    fn list(&mut self, path: &str) -> Res<Vec<Entry>> {
        run(LIST_TIMEOUT, async {
            let dir = self.sftp.read_dir(path.to_string()).await.map_err(|e| serr(&tf("Ordner «{p}» lässt sich nicht öffnen", &[("p", &path)]), e))?;
            let mut out = Vec::new();
            for ent in dir.take(MAX_LIST_ENTRIES) {
                let name = ent.file_name();
                if name == "." || name == ".." {
                    continue;
                }
                let md = ent.metadata();
                let ft = ent.file_type();
                let mut e = entry_from(name, &md, ft.is_dir(), ft.is_symlink());
                if e.is_link {
                    // Verknüpfung: das Ziel entscheidet, ob es wie ein Ordner geöffnet wird
                    if let Ok(target) = self.sftp.metadata(rjoin(path, &e.name)).await {
                        e.is_dir = target.file_type().is_dir();
                        if let Some(s) = target.size {
                            e.size = s;
                        }
                    }
                }
                out.push(e);
            }
            Ok(out)
        })
    }

    fn mkdir(&mut self, path: &str) -> Res<()> {
        run(OP_TIMEOUT, async { self.sftp.create_dir(path.to_string()).await.map_err(|e| serr(t("Ordner anlegen"), e)) })
    }

    fn rename(&mut self, from: &str, to: &str) -> Res<()> {
        run(OP_TIMEOUT, async { self.sftp.rename(from.to_string(), to.to_string()).await.map_err(|e| serr(t("Umbenennen"), e)) })
    }

    fn remove_file(&mut self, path: &str) -> Res<()> {
        run(OP_TIMEOUT, async { self.sftp.remove_file(path.to_string()).await.map_err(|e| serr(t("Löschen"), e)) })
    }

    fn remove_dir(&mut self, path: &str) -> Res<()> {
        run(OP_TIMEOUT, async { self.sftp.remove_dir(path.to_string()).await.map_err(|e| serr(t("Ordner löschen"), e)) })
    }

    fn chmod(&mut self, path: &str, mode: u32) -> Res<()> {
        run(OP_TIMEOUT, async {
            let mut md = Metadata::empty();
            md.permissions = Some(mode & 0o7777);
            self.sftp.set_metadata(path.to_string(), md).await.map_err(|e| serr(t("Rechte ändern"), e))
        })
    }

    fn size(&mut self, path: &str) -> Res<Option<u64>> {
        run(OP_TIMEOUT, async {
            match self.sftp.metadata(path.to_string()).await {
                Ok(m) => Ok(m.size),
                Err(_) => Ok(None),
            }
        })
    }

    fn download(&mut self, remote: &str, local: &Path, resume: bool, prog: Prog) -> Res<u64> {
        // Keine Gesamtfrist (grosse Dateien dauern), aber jeder einzelne Schritt hat eine.
        rt()?.block_on(async {
            let total = step(self.sftp.metadata(remote.to_string())).await.ok().and_then(|m| m.size);
            let mut offset = 0u64;
            if resume {
                if let Ok(m) = std::fs::metadata(local) {
                    offset = m.len();
                    if let Some(tot) = total {
                        if offset == tot {
                            prog(Tick::Start { offset, total: tot });
                            return Ok(0);
                        }
                        if offset > tot {
                            offset = 0;
                        }
                    }
                }
            }
            let mut rf = match tokio::time::timeout(OP_TIMEOUT, self.sftp.open(remote.to_string())).await {
                Ok(r) => r.map_err(|e| serr(t("Herunterladen nicht möglich"), e))?,
                Err(_) => return Err(RErr::Disconnected(t("Der Server antwortet nicht (Zeitüberschreitung).").to_string())),
            };
            if offset > 0 {
                step(rf.seek(SeekFrom::Start(offset))).await?;
            }
            let mut file = if offset > 0 { OpenOptions::new().append(true).open(local)? } else { File::create(local)? };
            if !prog(Tick::Start { offset, total: total.unwrap_or(0) }) {
                return Err(RErr::Cancelled);
            }
            let mut got = 0u64;
            let mut buf = vec![0u8; CHUNK];
            loop {
                let n = step(rf.read(&mut buf)).await?;
                if n == 0 {
                    break;
                }
                file.write_all(buf.get(..n).unwrap_or(&[]))?;
                got = got.saturating_add(n as u64);
                if !prog(Tick::Bytes(n as u64)) {
                    let _ = step(rf.shutdown()).await;
                    return Err(RErr::Cancelled);
                }
            }
            file.flush()?;
            let _ = step(rf.shutdown()).await;
            if let Some(tot) = total {
                let have = offset.saturating_add(got);
                if have != tot {
                    return Err(RErr::Msg(tf("Datei unvollständig: {got} von {tot} Bytes", &[("got", &have), ("tot", &tot)])));
                }
            }
            Ok(got)
        })
    }

    fn upload(&mut self, local: &Path, remote: &str, resume: bool, prog: Prog) -> Res<u64> {
        rt()?.block_on(async {
            let mut file = File::open(local)?;
            let total = file.metadata()?.len();
            let mut offset = 0u64;
            if resume {
                if let Some(rs) = step(self.sftp.metadata(remote.to_string())).await.ok().and_then(|m| m.size) {
                    if rs == total && total > 0 {
                        prog(Tick::Start { offset: rs, total });
                        return Ok(0);
                    }
                    if rs > 0 && rs < total {
                        offset = rs;
                    }
                }
            }
            let mut flags = OpenFlags::CREATE | OpenFlags::WRITE;
            if offset == 0 {
                flags |= OpenFlags::TRUNCATE;
            }
            let mut wf = match tokio::time::timeout(OP_TIMEOUT, self.sftp.open_with_flags(remote.to_string(), flags)).await {
                Ok(r) => r.map_err(|e| serr(t("Hochladen nicht möglich"), e))?,
                Err(_) => return Err(RErr::Disconnected(t("Der Server antwortet nicht (Zeitüberschreitung).").to_string())),
            };
            if offset > 0 {
                step(wf.seek(SeekFrom::Start(offset))).await?;
                file.seek(SeekFrom::Start(offset))?;
            }
            if !prog(Tick::Start { offset, total }) {
                return Err(RErr::Cancelled);
            }
            let mut sent = 0u64;
            let mut buf = vec![0u8; CHUNK];
            loop {
                let n = file.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                step(wf.write_all(buf.get(..n).unwrap_or(&[]))).await?;
                sent = sent.saturating_add(n as u64);
                if !prog(Tick::Bytes(n as u64)) {
                    let _ = step(wf.shutdown()).await;
                    return Err(RErr::Cancelled);
                }
            }
            step(wf.flush()).await?;
            step(wf.shutdown()).await?;
            Ok(sent)
        })
    }

    fn keepalive(&mut self) -> Res<()> {
        run(OP_TIMEOUT, async { self.sftp.canonicalize(".").await.map(|_| ()).map_err(|e| serr("", e)) })
    }

    fn close(&mut self) {
        let _ = run(Duration::from_secs(5), async {
            let _ = self.sftp.close().await;
            let _ = self.session.disconnect(russh::Disconnect::ByApplication, "", "en").await;
            Ok(())
        });
    }
}
