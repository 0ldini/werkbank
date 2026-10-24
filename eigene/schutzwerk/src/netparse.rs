//! Parser für die Netzwerk-Ansicht (defensiv): eigene lauschende Ports aus
//! `netstat -ano` und Geräte im eigenen Netz aus `arp -a`, dazu eine
//! Risiko-Einstufung bekannter Ports. Nur Standardbibliothek, gut testbar.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listen {
    pub proto: String,
    pub local: String,
    pub port: u16,
    pub pid: u32,
}

/// Liest aus `netstat -ano` nur lauschende Einträge (TCP LISTENING + UDP).
pub fn parse_netstat(out: &str) -> Vec<Listen> {
    let mut v = Vec::new();
    for line in out.lines() {
        let c: Vec<&str> = line.split_whitespace().collect();
        if c.len() < 4 {
            continue;
        }
        let proto = c[0].to_uppercase();
        if proto == "TCP" {
            // TCP  local  foreign  STATE  PID
            if c.len() >= 5 && ist_lauschend(c[3]) {
                if let (Some(port), Ok(pid)) = (port_of(c[1]), c[4].parse::<u32>()) {
                    v.push(Listen { proto, local: c[1].into(), port, pid });
                }
            }
        } else if proto == "UDP" {
            // UDP  local  foreign  PID   (kein State)
            if let (Some(port), Ok(pid)) = (port_of(c[1]), c[3].parse::<u32>()) {
                v.push(Listen { proto, local: c[1].into(), port, pid });
            }
        }
    }
    v
}

/// Zustand «lauschend» in den Sprachen von Windows (LISTENING, ABHÖREN, ÉCOUTE …).
/// Umlaute kommen je nach Codepage verstümmelt an – darum nur Wortanfänge.
fn ist_lauschend(s: &str) -> bool {
    let u = s.to_uppercase();
    u.starts_with("LISTEN") || u.starts_with("ABH") || u.contains("COUTE") || u.starts_with("ESCUCHA") || u.starts_with("IN_ASCOLTO")
}

/// Zustand «hergestellt» (ESTABLISHED, HERGESTELLT, ÉTABLIE, STABILITA …).
fn ist_hergestellt(s: &str) -> bool {
    let u = s.to_uppercase();
    u.starts_with("ESTAB") || u.starts_with("HERGESTELLT") || u.contains("TABLI") || u.starts_with("STABILIT")
}

/// Port aus "0.0.0.0:135" oder "[::]:445" herausziehen.
fn port_of(addr: &str) -> Option<u16> {
    let i = addr.rfind(':')?;
    addr[i + 1..].parse::<u16>().ok()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub ip: String,
    pub mac: String,
}

/// Liest aus `arp -a` die Geräte (IPv4 + MAC); überspringt Broadcast/Multicast.
pub fn parse_arp(out: &str) -> Vec<Device> {
    let mut v = Vec::new();
    for line in out.lines() {
        let c: Vec<&str> = line.split_whitespace().collect();
        if c.len() < 2 {
            continue;
        }
        let ip = c[0];
        let mac = c[1];
        if !is_ipv4(ip) || !is_mac(mac) {
            continue;
        }
        let m = mac.to_lowercase();
        // Broadcast / Multicast ausblenden
        if m.starts_with("ff-ff-ff") || m.starts_with("01-00-5e") || m.starts_with("33-33") {
            continue;
        }
        if ip.ends_with(".255") || ip.starts_with("224.") || ip.starts_with("239.") {
            continue;
        }
        v.push(Device { ip: ip.into(), mac: m });
    }
    v
}

fn is_ipv4(s: &str) -> bool {
    let p: Vec<&str> = s.split('.').collect();
    p.len() == 4 && p.iter().all(|x| x.parse::<u8>().is_ok())
}

fn is_mac(s: &str) -> bool {
    let p: Vec<&str> = s.split(['-', ':']).collect();
    p.len() == 6 && p.iter().all(|x| x.len() == 2 && u8::from_str_radix(x, 16).is_ok())
}

/// Einstufung bekannter Ports. None = unauffällig.
/// (Name, Hinweis, kritisch?) – kritisch = sollte nach aussen zu sein.
pub fn port_risk(port: u16) -> Option<(&'static str, &'static str, bool)> {
    Some(match port {
        21 => ("FTP", "Unverschlüsselt – besser SFTP verwenden.", true),
        23 => ("Telnet", "Unverschlüsselt und riskant – abschalten.", true),
        135 => ("RPC", "Windows-Dienst – nicht ins Internet freigeben.", true),
        137 | 138 | 139 => ("NetBIOS", "Datei-/Druckerfreigabe – nur im LAN.", true),
        445 => ("SMB", "Datei­freigabe – niemals offen ins Internet.", true),
        1433 => ("MS SQL", "Datenbank – nicht nach aussen öffnen.", true),
        3306 => ("MySQL", "Datenbank – nicht nach aussen öffnen.", true),
        3389 => ("RDP", "Fernzugriff – nur über VPN, nie offen.", true),
        5900 => ("VNC", "Fernsteuerung – nur über VPN.", true),
        5357 => ("WSD", "Geräte-Erkennung – im LAN unkritisch.", false),
        548 => ("AFP", "Dateifreigabe – nur im LAN.", true),
        _ => return None,
    })
}

/// Eine hergestellte TCP-Verbindung nach aussen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conn {
    pub remote_ip: String,
    pub remote_port: u16,
    pub pid: u32,
    /// Prozessname, wenn die Ausgabe ihn liefert (ss/lsof).
    pub name: String,
}

/// "1.2.3.4:443" / "[2a00::1]:443" / "[::ffff:1.2.3.4]:443" -> (IP, Port)
pub fn split_addr(addr: &str) -> Option<(String, u16)> {
    let i = addr.rfind(':')?;
    let port = addr.get(i + 1..)?.parse::<u16>().ok()?;
    let ip = addr.get(..i)?.trim_start_matches('[').trim_end_matches(']');
    let ip = ip.strip_prefix("::ffff:").unwrap_or(ip);
    let ip = ip.split('%').next().unwrap_or(ip); // Zonen-Index (fe80::1%12)
    if ip.is_empty() {
        return None;
    }
    Some((ip.to_string(), port))
}

/// Loopback oder unbestimmte Adresse (keine echte Verbindung nach aussen).
pub fn ist_lokal(ip: &str) -> bool {
    ip.starts_with("127.") || ip == "::1" || ip == "0.0.0.0" || ip == "::" || ip == "*"
}

/// Adresse im eigenen Netz (privat oder link-lokal).
pub fn ist_heimnetz(ip: &str) -> bool {
    if ip.starts_with("10.") || ip.starts_with("192.168.") || ip.starts_with("169.254.") {
        return true;
    }
    if let Some(rest) = ip.strip_prefix("172.") {
        if let Some(n) = rest.split('.').next().and_then(|x| x.parse::<u8>().ok()) {
            return (16..=31).contains(&n);
        }
    }
    let l = ip.to_lowercase();
    l.starts_with("fe80:") || l.starts_with("fc") || l.starts_with("fd")
}

fn conn_ok(ip: &str) -> bool {
    !ist_lokal(ip)
}

/// Windows `netstat -ano`: hergestellte TCP-Verbindungen (ohne Loopback).
pub fn parse_netstat_established(out: &str) -> Vec<Conn> {
    let mut v = Vec::new();
    for line in out.lines() {
        let c: Vec<&str> = line.split_whitespace().collect();
        if c.len() < 5 || !c[0].eq_ignore_ascii_case("TCP") || !ist_hergestellt(c[3]) {
            continue;
        }
        let (Some((ip, port)), Ok(pid)) = (split_addr(c[2]), c[c.len() - 1].parse::<u32>()) else { continue };
        if conn_ok(&ip) {
            v.push(Conn { remote_ip: ip, remote_port: port, pid, name: String::new() });
        }
    }
    v
}

/// Linux `ss -tnpH state established`: «0 0 lokal entfernt users:(("name",pid=1,fd=3))».
pub fn parse_ss(out: &str) -> Vec<Conn> {
    let mut v = Vec::new();
    for line in out.lines() {
        let c: Vec<&str> = line.split_whitespace().collect();
        if c.len() < 4 {
            continue;
        }
        // Mit oder ohne Spalte «ESTAB» am Anfang.
        let off = if c[0].parse::<u64>().is_ok() { 0 } else { 1 };
        let Some(peer) = c.get(off + 3) else { continue };
        let Some((ip, port)) = split_addr(peer) else { continue };
        if !conn_ok(&ip) {
            continue;
        }
        let rest = c.get(off + 4..).map(|r| r.join(" ")).unwrap_or_default();
        let pid = rest
            .find("pid=")
            .and_then(|p| rest.get(p + 4..))
            .map(|r| r.chars().take_while(|ch| ch.is_ascii_digit()).collect::<String>())
            .and_then(|n| n.parse::<u32>().ok())
            .unwrap_or(0);
        let name = rest
            .find("((\"")
            .and_then(|p| rest.get(p + 3..))
            .and_then(|r| r.split('"').next())
            .unwrap_or("")
            .to_string();
        v.push(Conn { remote_ip: ip, remote_port: port, pid, name });
    }
    v
}

/// macOS/Linux `lsof -nP -iTCP -sTCP:ESTABLISHED`:
/// «firefox 123 oli 50u IPv4 0x… 0t0 TCP 192.168.1.5:50000->140.82.112.25:443 (ESTABLISHED)».
pub fn parse_lsof(out: &str) -> Vec<Conn> {
    let mut v = Vec::new();
    for line in out.lines().skip_while(|l| l.starts_with("COMMAND")) {
        let c: Vec<&str> = line.split_whitespace().collect();
        if c.len() < 9 {
            continue;
        }
        let Ok(pid) = c[1].parse::<u32>() else { continue };
        let Some(name_col) = c.iter().find(|x| x.contains("->")) else { continue };
        let Some((_, remote)) = name_col.split_once("->") else { continue };
        let Some((ip, port)) = split_addr(remote) else { continue };
        if conn_ok(&ip) {
            v.push(Conn { remote_ip: ip, remote_port: port, pid, name: c[0].replace("\\x20", " ") });
        }
    }
    v
}

/// Bekannte Ziel-Ports: (Name, typischer Mining-Pool-Port?)
pub fn port_name(port: u16) -> Option<(&'static str, bool)> {
    Some(match port {
        80 => ("HTTP", false),
        443 => ("HTTPS", false),
        22 => ("SSH", false),
        21 => ("FTP", false),
        25 | 465 | 587 => ("Mail (SMTP)", false),
        993 | 143 => ("Mail (IMAP)", false),
        995 | 110 => ("Mail (POP3)", false),
        53 => ("DNS", false),
        3389 => ("RDP", false),
        5938 => ("TeamViewer", false),
        6568 | 7070 => ("AnyDesk", false),
        21115..=21119 => ("RustDesk", false),
        5900 => ("VNC", false),
        3333 | 4444 | 5555 | 7777 | 14433 | 14444 | 45700 => ("Stratum (Mining)", true),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lauschende_ports_mehrsprachig() {
        let en = "  TCP    0.0.0.0:135            0.0.0.0:0              LISTENING       1044\n  UDP    0.0.0.0:5353           *:*                                    2520\n";
        let de = "  TCP    0.0.0.0:445            0.0.0.0:0              ABH\u{fffd}REN         4\n";
        assert_eq!(parse_netstat(en).len(), 2);
        assert_eq!(parse_netstat(de)[0].port, 445);
    }

    #[test]
    fn hergestellt_windows() {
        let out = "\nAktive Verbindungen\n\n  Proto  Lokale Adresse         Remoteadresse          Status           PID\n\
  TCP    192.168.1.20:50312     140.82.112.25:443      HERGESTELLT     8124\n\
  TCP    127.0.0.1:50000        127.0.0.1:50001        HERGESTELLT     99\n\
  TCP    [2a02:1210::5]:50400   [2a00:1450:400a::200e]:443  ESTABLISHED     4410\n\
  TCP    0.0.0.0:135            0.0.0.0:0              ABH\u{fffd}REN         1044\n";
        let v = parse_netstat_established(out);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0], Conn { remote_ip: "140.82.112.25".into(), remote_port: 443, pid: 8124, name: String::new() });
        assert_eq!(v[1].remote_ip, "2a00:1450:400a::200e");
    }

    #[test]
    fn ss_linux() {
        let out = "0      0      192.168.1.5:43210   140.82.112.25:443   users:((\"firefox\",pid=1234,fd=55))\n\
0 0 [::ffff:192.168.1.5]:5000 [::ffff:8.8.8.8]:853\n\
ESTAB 0 0 127.0.0.1:1 127.0.0.1:2 users:((\"x\",pid=1,fd=1))\n";
        let v = parse_ss(out);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0], Conn { remote_ip: "140.82.112.25".into(), remote_port: 443, pid: 1234, name: "firefox".into() });
        assert_eq!(v[1].remote_ip, "8.8.8.8");
        assert_eq!(v[1].pid, 0);
    }

    #[test]
    fn lsof_mac() {
        let out = "COMMAND   PID USER   FD   TYPE             DEVICE SIZE/OFF NODE NAME\n\
firefox   123  oli   50u  IPv4 0x1234567890abcdef      0t0  TCP 192.168.1.5:50000->140.82.112.25:443 (ESTABLISHED)\n\
Mail      456  oli   12u  IPv6 0xabc      0t0  TCP [2a02::1]:50001->[2a00:1450::1]:993 (ESTABLISHED)\n";
        let v = parse_lsof(out);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].pid, 123);
        assert_eq!(v[1], Conn { remote_ip: "2a00:1450::1".into(), remote_port: 993, pid: 456, name: "Mail".into() });
    }

    #[test]
    fn adressen() {
        assert!(ist_heimnetz("192.168.1.1") && ist_heimnetz("172.20.0.1") && !ist_heimnetz("172.32.0.1"));
        assert!(ist_lokal("127.0.0.1") && !ist_lokal("8.8.8.8"));
        assert_eq!(port_name(3333), Some(("Stratum (Mining)", true)));
        assert_eq!(split_addr("[fe80::1%12]:80"), Some(("fe80::1".into(), 80)));
    }

    #[test]
    fn arp() {
        let out = "  192.168.1.1           34-81-c4-aa-bb-cc     dynamisch\n  192.168.1.255         ff-ff-ff-ff-ff-ff     statisch\n";
        assert_eq!(parse_arp(out).len(), 1);
    }
}
