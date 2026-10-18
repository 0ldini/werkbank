//! Zeitplan-Texte: Aufgabe für die Windows-Aufgabenplanung (XML für `schtasks /Create /XML`),
//! crontab-Zeile und systemd-Timer für Linux. Nur std; Test:
//! `rustc --edition 2021 --test src/zeitplan.rs`.

#![allow(dead_code)]

/// Wann eine Aufgabe läuft.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Takt {
    /// Täglich um h:m.
    Taeglich { h: u8, m: u8 },
    /// Wöchentlich am Tag (0 = Montag) um h:m.
    Woechentlich { tag: u8, h: u8, m: u8 },
    /// Alle n Minuten (Anschluss-Prüfung).
    Minuten(u32),
}

const TAGE_EN: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
const TAGE_KURZ: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

pub fn zeit_text(h: u8, m: u8) -> String {
    format!("{:02}:{:02}", h.min(23), m.min(59))
}

/// Name der Aufgabe in der Aufgabenplanung (Ordner «Sicherwerk»).
pub fn aufgabenname(auftrag: &str) -> String {
    format!("Sicherwerk\\{}", auftrag.trim())
}

/// Gemeinsame Aufgabe, die regelmässig nach angeschlossenen Ziellaufwerken schaut.
pub const ANSCHLUSS_AUFGABE: &str = "Sicherwerk\\Anschluss";

fn xml_esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            c if (c as u32) < 0x20 && c != '\t' => {}
            c => o.push(c),
        }
    }
    o
}

/// Argumente für einen Auftrag: `--auftrag "Name"` (Namen enthalten nie `"`, siehe auftrag.rs).
pub fn argumente_auftrag(name: &str) -> String {
    format!("--auftrag \"{}\"", name.trim().replace('"', ""))
}

/// Aufgaben-XML (Task Scheduler 1.2). `start_datum` = «YYYY-MM-DD» (heute), `benutzer` =
/// «DOMÄNE\Name» oder leer. Verpasste Läufe holt Windows nach (StartWhenAvailable), auch
/// im Akkubetrieb; kein Zeitlimit; kein zweiter Lauf, solange einer läuft.
pub fn aufgabe_xml(beschreibung: &str, exe: &str, argumente: &str, takt: Takt, start_datum: &str, benutzer: &str) -> String {
    let trigger = match takt {
        Takt::Taeglich { h, m } => format!(
            "    <CalendarTrigger>\n      <StartBoundary>{start_datum}T{}:00</StartBoundary>\n      <Enabled>true</Enabled>\n      <ScheduleByDay>\n        <DaysInterval>1</DaysInterval>\n      </ScheduleByDay>\n    </CalendarTrigger>\n",
            zeit_text(h, m)
        ),
        Takt::Woechentlich { tag, h, m } => format!(
            "    <CalendarTrigger>\n      <StartBoundary>{start_datum}T{}:00</StartBoundary>\n      <Enabled>true</Enabled>\n      <ScheduleByWeek>\n        <DaysOfWeek>\n          <{} />\n        </DaysOfWeek>\n        <WeeksInterval>1</WeeksInterval>\n      </ScheduleByWeek>\n    </CalendarTrigger>\n",
            zeit_text(h, m),
            TAGE_EN[(tag as usize).min(6)]
        ),
        // Dazu bei der Anmeldung (nur mit Benutzer – «jeder Benutzer» bräuchte Adminrechte).
        Takt::Minuten(n) => format!(
            "    <TimeTrigger>\n      <Repetition>\n        <Interval>PT{}M</Interval>\n        <StopAtDurationEnd>false</StopAtDurationEnd>\n      </Repetition>\n      <StartBoundary>{start_datum}T00:00:00</StartBoundary>\n      <Enabled>true</Enabled>\n    </TimeTrigger>\n{}",
            n.clamp(1, 1440),
            if benutzer.is_empty() {
                String::new()
            } else {
                format!(
                    "    <LogonTrigger>\n      <Enabled>true</Enabled>\n      <UserId>{}</UserId>\n    </LogonTrigger>\n",
                    xml_esc(benutzer)
                )
            }
        ),
    };
    let user = if benutzer.is_empty() { String::new() } else { format!("      <UserId>{}</UserId>\n", xml_esc(benutzer)) };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-16\"?>\n\
<Task version=\"1.2\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\">\n\
  <RegistrationInfo>\n\
    <Description>{besch}</Description>\n\
  </RegistrationInfo>\n\
  <Triggers>\n\
{trigger}\
  </Triggers>\n\
  <Principals>\n\
    <Principal id=\"Author\">\n\
{user}\
      <LogonType>InteractiveToken</LogonType>\n\
      <RunLevel>LeastPrivilege</RunLevel>\n\
    </Principal>\n\
  </Principals>\n\
  <Settings>\n\
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>\n\
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>\n\
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>\n\
    <AllowHardTerminate>true</AllowHardTerminate>\n\
    <StartWhenAvailable>true</StartWhenAvailable>\n\
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>\n\
    <IdleSettings>\n\
      <StopOnIdleEnd>false</StopOnIdleEnd>\n\
      <RestartOnIdle>false</RestartOnIdle>\n\
    </IdleSettings>\n\
    <AllowStartOnDemand>true</AllowStartOnDemand>\n\
    <Enabled>true</Enabled>\n\
    <Hidden>false</Hidden>\n\
    <RunOnlyIfIdle>false</RunOnlyIfIdle>\n\
    <WakeToRun>false</WakeToRun>\n\
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>\n\
    <Priority>7</Priority>\n\
  </Settings>\n\
  <Actions Context=\"Author\">\n\
    <Exec>\n\
      <Command>{exe}</Command>\n\
      <Arguments>{args}</Arguments>\n\
    </Exec>\n\
  </Actions>\n\
</Task>\n",
        besch = xml_esc(beschreibung),
        exe = xml_esc(exe),
        args = xml_esc(argumente),
    )
}

/// UTF-16LE mit BOM (so liest `schtasks /XML` die Datei sicher).
pub fn utf16_bom(s: &str) -> Vec<u8> {
    let mut v = vec![0xFF, 0xFE];
    for u in s.encode_utf16() {
        v.extend_from_slice(&u.to_le_bytes());
    }
    v
}

/// `schtasks`-Argumente zum Anlegen (ersetzt eine bestehende Aufgabe gleichen Namens).
pub fn schtasks_anlegen(aufgabe: &str, xml_datei: &str) -> Vec<String> {
    ["/Create", "/F", "/TN", aufgabe, "/XML", xml_datei].iter().map(|s| s.to_string()).collect()
}

pub fn schtasks_loeschen(aufgabe: &str) -> Vec<String> {
    ["/Delete", "/F", "/TN", aufgabe].iter().map(|s| s.to_string()).collect()
}

pub fn schtasks_abfragen(aufgabe: &str) -> Vec<String> {
    ["/Query", "/TN", aufgabe].iter().map(|s| s.to_string()).collect()
}

// ------------------------------------------------------------------ Linux

/// Für die Shell in einfache Anführungszeichen setzen.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// crontab-Zeile. In crontab ist `%` ein Sonderzeichen und wird maskiert.
pub fn cron_zeile(exe: &str, argumente: &[&str], takt: Takt) -> String {
    let wann = match takt {
        Takt::Taeglich { h, m } => format!("{} {} * * *", m.min(59), h.min(23)),
        Takt::Woechentlich { tag, h, m } => format!("{} {} * * {}", m.min(59), h.min(23), (tag.min(6) + 1) % 7),
        Takt::Minuten(n) => format!("*/{} * * * *", n.clamp(1, 59)),
    };
    let mut befehl = shell_quote(exe);
    for a in argumente {
        befehl.push(' ');
        befehl.push_str(&shell_quote(a));
    }
    format!("{wann} {}", befehl.replace('%', "\\%"))
}

/// Name für systemd-Einheiten: «sicherwerk-<name>» aus Kleinbuchstaben, Ziffern und «-».
pub fn einheit_name(auftrag: &str, id: u64) -> String {
    let mut s = String::new();
    for c in auftrag.trim().chars() {
        let c = match c {
            'ä' | 'Ä' => "ae".to_string(),
            'ö' | 'Ö' => "oe".to_string(),
            'ü' | 'Ü' => "ue".to_string(),
            'ß' => "ss".to_string(),
            c if c.is_ascii_alphanumeric() => c.to_ascii_lowercase().to_string(),
            _ => "-".to_string(),
        };
        if c == "-" && (s.is_empty() || s.ends_with('-')) {
            continue;
        }
        s.push_str(&c);
    }
    let s = s.trim_end_matches('-').to_string();
    if s.is_empty() {
        format!("sicherwerk-{id}")
    } else {
        format!("sicherwerk-{s}")
    }
}

/// Wert für systemd: `"…"` mit `\`, `"`, `%`, `$` maskiert.
fn systemd_quote(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '"' => o.push_str("\\\""),
            '%' => o.push_str("%%"),
            '$' => o.push_str("$$"),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

pub fn systemd_dienst(beschreibung: &str, exe: &str, argumente: &[&str]) -> String {
    let mut befehl = systemd_quote(exe);
    for a in argumente {
        befehl.push(' ');
        befehl.push_str(&systemd_quote(a));
    }
    format!(
        "[Unit]\nDescription={}\n\n[Service]\nType=oneshot\nExecStart={befehl}\nNice=10\nIOSchedulingClass=idle\n",
        beschreibung.replace(['\n', '\r'], " ").replace('%', "%%")
    )
}

pub fn systemd_timer(beschreibung: &str, takt: Takt) -> String {
    let wann = match takt {
        Takt::Taeglich { h, m } => format!("OnCalendar=*-*-* {}:00\nPersistent=true", zeit_text(h, m)),
        Takt::Woechentlich { tag, h, m } => {
            format!("OnCalendar={} *-*-* {}:00\nPersistent=true", TAGE_KURZ[(tag as usize).min(6)], zeit_text(h, m))
        }
        Takt::Minuten(n) => format!("OnBootSec=2min\nOnUnitActiveSec={}min", n.clamp(1, 1440)),
    };
    format!(
        "[Unit]\nDescription={}\n\n[Timer]\n{wann}\n\n[Install]\nWantedBy=timers.target\n",
        beschreibung.replace(['\n', '\r'], " ").replace('%', "%%")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml() {
        let x = aufgabe_xml(
            "Sicherwerk: Fotos & Co",
            "C:\\Werkbank\\sicherwerk.exe",
            &argumente_auftrag("Fotos & Co"),
            Takt::Woechentlich { tag: 4, h: 18, m: 5 },
            "2026-10-18",
            "PC\\Oli",
        );
        assert!(x.contains("<StartBoundary>2026-10-18T18:05:00</StartBoundary>"));
        assert!(x.contains("<Friday />"));
        assert!(x.contains("<Arguments>--auftrag &quot;Fotos &amp; Co&quot;</Arguments>"));
        assert!(x.contains("<Command>C:\\Werkbank\\sicherwerk.exe</Command>"));
        assert!(x.contains("<UserId>PC\\Oli</UserId>"));
        assert!(x.contains("<StartWhenAvailable>true</StartWhenAvailable>"));
        let t = aufgabe_xml("x", "a.exe", "--anschluss", Takt::Minuten(15), "2026-10-18", "");
        assert!(t.contains("<Interval>PT15M</Interval>"));
        assert!(!t.contains("UserId"));
        let d = aufgabe_xml("x", "a.exe", "", Takt::Taeglich { h: 7, m: 0 }, "2026-01-02", "");
        assert!(d.contains("T07:00:00") && d.contains("<DaysInterval>1</DaysInterval>"));
        let b = utf16_bom("<a>");
        assert_eq!(b, vec![0xFF, 0xFE, b'<', 0, b'a', 0, b'>', 0]);
    }

    #[test]
    fn cron() {
        assert_eq!(
            cron_zeile("/opt/sicherwerk", &["--auftrag", "Fotos"], Takt::Taeglich { h: 18, m: 30 }),
            "30 18 * * * '/opt/sicherwerk' '--auftrag' 'Fotos'"
        );
        assert_eq!(
            cron_zeile("/x/s", &["--auftrag", "Olis Daten"], Takt::Woechentlich { tag: 6, h: 9, m: 0 }),
            "0 9 * * 0 '/x/s' '--auftrag' 'Olis Daten'"
        );
        assert_eq!(
            cron_zeile("/x/s", &["--auftrag", "it's 100%"], Takt::Woechentlich { tag: 0, h: 9, m: 0 }),
            "0 9 * * 1 '/x/s' '--auftrag' 'it'\\''s 100\\%'"
        );
        assert_eq!(cron_zeile("/x/s", &["--anschluss"], Takt::Minuten(15)), "*/15 * * * * '/x/s' '--anschluss'");
    }

    #[test]
    fn systemd() {
        assert_eq!(einheit_name("Fotos & Bilder", 3), "sicherwerk-fotos-bilder");
        assert_eq!(einheit_name("Grüsse", 3), "sicherwerk-gruesse");
        assert_eq!(einheit_name("???", 3), "sicherwerk-3");
        let d = systemd_dienst("Sicherwerk: A", "/home/oli/bin/sicherwerk", &["--auftrag", "A $x 5%"]);
        assert!(d.contains("ExecStart=\"/home/oli/bin/sicherwerk\" \"--auftrag\" \"A $$x 5%%\""));
        let t = systemd_timer("x", Takt::Woechentlich { tag: 0, h: 6, m: 7 });
        assert!(t.contains("OnCalendar=Mon *-*-* 06:07:00"));
        assert!(t.contains("Persistent=true"));
        let t = systemd_timer("x", Takt::Minuten(15));
        assert!(t.contains("OnUnitActiveSec=15min"));
    }
}
