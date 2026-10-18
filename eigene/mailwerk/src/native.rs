//! Desktop: IMAP und SMTP über eine verschlüsselte Verbindung (rustls).
//! Jeder Befehl baut seine eigene Verbindung auf, erledigt die Arbeit und schliesst sie wieder.
//! Das ist langsamer als eine dauernd offene Verbindung, aber einfach und robust.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use crate::imap::Imap;
use crate::mime;
use crate::smtp::Smtp;
use crate::timeutil;
use crate::types::{Account, Cmd, Ev, FolderInfo, HeaderRow, MailBody, Security};

type Tls = rustls::StreamOwned<rustls::ClientConnection, TcpStream>;

fn tls_config() -> Arc<rustls::ClientConfig> {
    static CFG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    CFG.get_or_init(|| {
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let cfg = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("TLS-Versionen")
            .with_root_certificates(roots)
            .with_no_client_auth();
        Arc::new(cfg)
    })
    .clone()
}

fn connect_tcp(host: &str, port: u16) -> Result<TcpStream, String> {
    let addrs: Vec<_> = (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("Server «{host}» nicht gefunden: {e}"))?
        .collect();
    let mut last = format!("Keine Adresse für «{host}».");
    for a in addrs {
        match TcpStream::connect_timeout(&a, Duration::from_secs(15)) {
            Ok(s) => {
                let _ = s.set_read_timeout(Some(Duration::from_secs(40)));
                let _ = s.set_write_timeout(Some(Duration::from_secs(40)));
                let _ = s.set_nodelay(true);
                return Ok(s);
            }
            Err(e) => last = format!("Verbindung zu {host}:{port} fehlgeschlagen: {e}"),
        }
    }
    Err(last)
}

fn wrap_tls(host: &str, tcp: TcpStream) -> Result<Tls, String> {
    let name = rustls::pki_types::ServerName::try_from(host.to_string()).map_err(|_| format!("Ungültiger Servername: {host}"))?;
    let conn = rustls::ClientConnection::new(tls_config(), name).map_err(|e| format!("Sichere Verbindung zu {host} fehlgeschlagen: {e}"))?;
    Ok(rustls::StreamOwned::new(conn, tcp))
}

fn login_name(acc: &Account) -> &str {
    if acc.user.trim().is_empty() {
        &acc.email
    } else {
        &acc.user
    }
}

fn imap_connect(acc: &Account) -> Result<Imap<Tls>, String> {
    if acc.imap_port == 143 {
        return Err("Unverschlüsseltes IMAP (Port 143) wird nicht unterstützt. Bitte Port 993 verwenden.".into());
    }
    let tcp = connect_tcp(&acc.imap_host, acc.imap_port)?;
    let tls = wrap_tls(&acc.imap_host, tcp)?;
    let mut im = Imap::new(tls);
    im.read_greeting()?;
    im.login(login_name(acc), &acc.password)?;
    Ok(im)
}

fn seed() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1)
}

fn smtp_finish<S: Read + Write>(sm: &mut Smtp<S>, acc: &Account, from: &str, rcpts: &[String], raw: &[u8]) -> Result<(), String> {
    sm.auth(login_name(acc), &acc.password)?;
    sm.send(from, rcpts, raw)?;
    sm.quit();
    Ok(())
}

fn smtp_send(acc: &Account, from: &str, rcpts: &[String], raw: &[u8]) -> Result<(), String> {
    let host = acc.smtp_host.as_str();
    let tcp = connect_tcp(host, acc.smtp_port)?;
    match acc.smtp_security {
        Security::Tls => {
            let mut sm = Smtp::new(wrap_tls(host, tcp)?);
            sm.read_greeting()?;
            sm.ehlo(&acc.helo_name())?;
            smtp_finish(&mut sm, acc, from, rcpts, raw)
        }
        Security::StartTls => {
            let mut sm = Smtp::new(tcp);
            sm.read_greeting()?;
            sm.ehlo(&acc.helo_name())?;
            if !sm.has_cap("STARTTLS") {
                return Err("Der Server bietet keine Verschlüsselung (STARTTLS) an.".into());
            }
            sm.starttls()?;
            let tcp = sm.into_inner()?;
            let mut sm = Smtp::new(wrap_tls(host, tcp)?);
            sm.ehlo(&acc.helo_name())?;
            smtp_finish(&mut sm, acc, from, rcpts, raw)
        }
    }
}

fn uid(id: &str) -> Result<u32, String> {
    id.parse::<u32>().map_err(|_| format!("Ungültige Nachrichtennummer «{id}»."))
}

/// Führt einen Befehl aus und liefert die Ereignisse. Blockiert, darum in einem eigenen Thread aufrufen.
pub fn run(acc: &Account, cmd: Cmd) -> Vec<Ev> {
    let what = cmd.what();
    match run_inner(acc, cmd) {
        Ok(v) => v,
        Err(err) => vec![Ev::Failed { acc: acc.id, what, err }],
    }
}

fn run_inner(acc: &Account, cmd: Cmd) -> Result<Vec<Ev>, String> {
    let what = cmd.what();
    let a = acc.id;
    let done = |note: &str| Ev::Done { acc: a, what: what.clone(), note: note.to_string() };
    match cmd {
        Cmd::Folders => {
            let mut im = imap_connect(acc)?;
            let mut out: Vec<FolderInfo> = Vec::new();
            for f in im.list_folders()?.into_iter().filter(|f| f.selectable) {
                let (total, unread) = im.status(&f.raw).unwrap_or((0, 0));
                out.push(FolderInfo { id: f.raw.clone(), name: f.name.clone(), role: f.role, unread, total });
            }
            out.sort_by(|x, y| x.role.sort_key().cmp(&y.role.sort_key()).then(x.name.to_lowercase().cmp(&y.name.to_lowercase())));
            im.logout();
            Ok(vec![Ev::Folders { acc: a, folders: out }])
        }
        Cmd::Headers { folder, count } => {
            let mut im = imap_connect(acc)?;
            let info = im.select(&folder, true)?;
            let n = info.exists;
            let mut rows: Vec<HeaderRow> = Vec::new();
            if n > 0 {
                let from = n.saturating_sub(count.saturating_sub(1)).max(1);
                for it in im.fetch_headers_range(from, n)? {
                    let p = mime::parse_head(&mime::parse_headers(&it.header));
                    let f = p.from.first().cloned().unwrap_or_default();
                    rows.push(HeaderRow {
                        id: it.uid.to_string(),
                        from_name: f.name,
                        from_email: f.email,
                        subject: p.subject,
                        ts: p.date_ts.or_else(|| timeutil::parse_imap_date(&it.internal_date)).unwrap_or(0),
                        unread: !it.has_flag("\\Seen"),
                        flagged: it.has_flag("\\Flagged"),
                        has_att: p.looks_like_attachments,
                        preview: String::new(),
                    });
                }
            }
            rows.sort_by(|x, y| y.ts.cmp(&x.ts));
            im.logout();
            Ok(vec![Ev::Headers { acc: a, folder, rows, total: n }])
        }
        Cmd::Body { folder, id } => {
            let mut im = imap_connect(acc)?;
            im.select(&folder, true)?;
            let it = im.fetch_message(uid(&id)?)?;
            let p = mime::parse(&it.body);
            let body = MailBody::from_parsed(&id, &p, timeutil::now_ts());
            im.logout();
            Ok(vec![Ev::Body { acc: a, folder, body }])
        }
        Cmd::SetSeen { folder, id, seen } => {
            let mut im = imap_connect(acc)?;
            im.select(&folder, false)?;
            im.set_flag(uid(&id)?, "\\Seen", seen)?;
            im.logout();
            Ok(vec![done("")])
        }
        Cmd::SetFlagged { folder, id, flagged } => {
            let mut im = imap_connect(acc)?;
            im.select(&folder, false)?;
            im.set_flag(uid(&id)?, "\\Flagged", flagged)?;
            im.logout();
            Ok(vec![done("")])
        }
        Cmd::MoveTo { folder, id, target } => {
            let mut im = imap_connect(acc)?;
            im.select(&folder, false)?;
            im.move_to(uid(&id)?, &target)?;
            im.logout();
            Ok(vec![done("")])
        }
        Cmd::Delete { folder, id, trash } => {
            let mut im = imap_connect(acc)?;
            im.select(&folder, false)?;
            match trash {
                Some(t) if t != folder => im.move_to(uid(&id)?, &t)?,
                _ => im.delete_here(uid(&id)?)?,
            }
            im.logout();
            Ok(vec![done("")])
        }
        Cmd::Send { msg, sent_folder, .. } => {
            let rcpts = msg.envelope_recipients();
            if rcpts.is_empty() {
                return Err("Kein Empfänger angegeben.".into());
            }
            let (raw, _id) = mime::build(&msg, timeutil::now_ts(), seed());
            smtp_send(acc, &msg.from.email, &rcpts, &raw)?;
            // Gmail & Co. legen die Kopie selbst ab; sonst würde sie doppelt erscheinen.
            let server_keeps_copy = acc.smtp_host.contains("gmail.com") || acc.smtp_host.contains("googlemail.com");
            let mut note = String::new();
            if let (Some(folder), false) = (sent_folder, server_keeps_copy) {
                let res = imap_connect(acc).and_then(|mut im| {
                    let r = im.append(&folder, "\\Seen", &raw);
                    im.logout();
                    r
                });
                if let Err(e) = res {
                    note = format!("Gesendet, aber die Kopie konnte nicht im Ordner «Gesendet» abgelegt werden: {e}");
                }
            }
            Ok(vec![done(&note)])
        }
        Cmd::Attachment { .. } => Err("Anhänge von IMAP-Konten sind schon in der Nachricht enthalten.".into()),
    }
}
