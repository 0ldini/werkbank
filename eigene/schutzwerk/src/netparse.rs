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
            if c.len() >= 5 && c[3].eq_ignore_ascii_case("LISTENING") {
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
