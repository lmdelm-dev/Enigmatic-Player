use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, UdpSocket};
use std::process::Command;
use std::time::Duration;

/// Best-effort, user-free reachability for the host's room:
/// - asks the router to open a TCP port for this session (NAT-PMP, then UPnP)
/// - looks up the machine's public IPv4 with plain HTTP
///
/// Every attempt is allowed to fail; the caller keeps working without the
/// public address if that happens. Nothing here scans or talks to anything
/// except the local router and a public-ip echo endpoint.

/// How long the router should keep the mapping (24h; removed on leave anyway).
const LEASE_SECS: u32 = 86_400;

pub struct Reach {
    /// Address strings that can be put into an invite: `ip:port`.
    pub servers: Vec<String>,
    /// Short, honest note about how far reachability got.
    pub note: String,
    /// The port on the router's public side.
    pub pub_port: Option<u16>,
    /// The port on this machine's side (same as what was asked, normally).
    pub in_port: u16,
    /// The UPnP control URL, so the mapping can be removed again.
    pub control: Option<String>,
    /// The gateway that granted NAT-PMP, so the mapping can be removed again.
    pub pmp_gw: Option<IpAddr>,
}

impl Default for Reach {
    fn default() -> Self {
        Reach {
            servers: Vec::new(),
            note: String::new(),
            pub_port: None,
            in_port: 0,
            control: None,
            pmp_gw: None,
        }
    }
}

/// Skipped when set: speed and deterministic tests.
fn punching_disabled() -> bool {
    std::env::var("ENIGMATIC_NO_PUNCH").is_ok()
}

// ---------- local network facts ----------

/// The machine's IPv4 on its LAN, by asking the OS for the interface that
/// would be used to reach the internet. No packet is sent.
pub fn local_ip() -> Option<Ipv4Addr> {
    let ip = "1.1.1.1:80".parse::<SocketAddr>().ok()?;
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect(ip).ok()?;
    match s.local_addr().ok()? {
        SocketAddr::V4(a) => Some(*a.ip()),
        _ => None,
    }
}

/// The default gateway's IPv4. Different OSes each need their own source.
pub fn default_gateway() -> Option<IpAddr> {
    #[cfg(windows)]
    {
        let out = Command::new("route").args(["print", "-4"]).output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            let c: Vec<&str> = line.split_whitespace().collect();
            if c.len() >= 3 && c[0] == "0.0.0.0" && c[1] == "0.0.0.0" {
                if let Ok(a) = c[2].parse::<Ipv4Addr>() {
                    return Some(IpAddr::V4(a));
                }
            }
        }
        None
    }
    #[cfg(target_os = "macos")]
    {
        let out = Command::new("route").args(["-n", "get", "default"]).output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            if line.trim_start().starts_with("gateway:") {
                let gw = line.rsplit_once(':')?.1.trim();
                if let Ok(a) = gw.parse::<IpAddr>() {
                    return Some(a);
                }
            }
        }
        None
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let out = Command::new("ip").args(["route", "show", "default"]).output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        for c in text.split_whitespace() {
            if let Ok(a) = c.parse::<IpAddr>() {
                return Some(a);
            }
        }
        None
    }
}

// ---------- tiny HTTP client (no deps) ----------

/// Splits an http:// URL into `host[:port]` and path. IPv4/hostname only:
/// every router and echo service we talk to is plain http over IPv4.
fn parse_url(url: &str) -> Result<(String, String), String> {
    if !url.starts_with("http://") {
        return Err(format!("only http:// urls are supported, got {url}"));
    }
    let rest = &url["http://".len()..];
    let (host, path) = match rest.split_once('/') {
        Some((h, p)) => (h, format!("/{p}")),
        None => (rest, "/".to_string()),
    };
    if host.is_empty() {
        return Err("empty http host".into());
    }
    Ok((host.to_string(), path))
}

/// `host` may lack a port; plain http implies :80.
fn host_with_port(host: &str) -> String {
    if host.contains(':') {
        host.to_string()
    } else {
        format!("{host}:80")
    }
}

fn dns_addrs(host: &str) -> Vec<SocketAddr> {
    use std::net::ToSocketAddrs;
    host_with_port(host).to_socket_addrs().map(|it| it.collect()).unwrap_or_default()
}

fn read_http_response(stream: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                break
            }
            Err(_) => break,
        }
        if buf.len() > 1024 * 1024 {
            break;
        }
    }
    String::from_utf8_lossy(&buf).to_string()
}

fn http_verbs(
    host: &str,
    method: &str,
    path: &str,
    extra_headers: &[(&str, String)],
    body: Option<&str>,
) -> Result<String, String> {
    let mut tcp = None;
    for a in dns_addrs(host) {
        match TcpStream::connect_timeout(&a, Duration::from_secs(5)) {
            Ok(s) => {
                let _ = s.set_nodelay(true);
                let _ = s.set_read_timeout(Some(Duration::from_secs(6)));
                let _ = s.set_write_timeout(Some(Duration::from_secs(6)));
                tcp = Some(s);
                break;
            }
            Err(_) => continue,
        }
    }
    let Some(mut stream) = tcp else {
        return Err(format!("cannot reach {host}"));
    };
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    for (k, v) in extra_headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    if let Some(b) = body {
        req.push_str(&format!("Content-Length: {}\r\n", b.len()));
    }
    req.push_str("\r\n");
    stream
        .write_all(req.as_bytes())
        .or(Err("http write failed".to_string()))?;
    if let Some(b) = body {
        stream.write_all(b.as_bytes()).or(Err("http body write failed".to_string()))?;
    }
    stream.flush().ok();
    Ok(read_http_response(&mut stream))
}

/// GET content. Does not check the status line.
fn http_get(host: &str, path: &str) -> Result<String, String> {
    http_verbs(host, "GET", path, &[], None)
}

fn http_post_soap(host: &str, path: &str, action: &str, body: &str) -> Result<String, String> {
    let extra = [
        ("Content-Type", "text/xml; charset=\"utf-8\"".to_string()),
        ("SOAPACTION", format!("\"{action}\"")),
    ];
    http_verbs(host, "POST", path, &extra, Some(body))
}

fn body_of(resp: &str) -> String {
    resp.find("\r\n\r\n")
        .or_else(|| resp.find("\n\n"))
        .map(|x| resp[x..].to_string())
        .unwrap_or_default()
}

fn status_ok(resp: &str) -> bool {
    let head = resp.lines().next().unwrap_or("");
    head.contains("200") || head.contains("204")
}

// ---------- public IP ----------

/// Three echo services allow plain http and answer one ip per line.
pub fn public_ip() -> Option<Ipv4Addr> {
    for (host, path) in [
        ("checkip.amazonaws.com", "/"),
        ("api.ipify.org", "/?format=text"),
        ("ifconfig.me", "/ip"),
    ] {
        if let Ok(resp) = http_get(host, path) {
            let body = body_of(&resp);
            if let Some(ip) = body
                .split([' ', '\r', '\n', '\t'])
                .find(|p| !p.trim().is_empty())
                .and_then(|p| p.trim().parse::<Ipv4Addr>().ok())
            {
                return Some(ip);
            }
        }
    }
    None
}

// ---------- NAT-PMP (RFC 6886) ----------

const PMP_PORT: u16 = 5351;
const PMP_OP_TCP: u8 = 2;

/// The wire request for a TCP mapping: version 0, op, reserved,
/// internal port, suggested external port, lifetime (RFC 6886 layout).
fn pmp_request(internal: u16, suggested: u16, lifetime: u32) -> Vec<u8> {
    let mut req = vec![0u8, PMP_OP_TCP, 0, 0];
    req.extend_from_slice(&internal.to_be_bytes());
    req.extend_from_slice(&suggested.to_be_bytes());
    req.extend_from_slice(&lifetime.to_be_bytes());
    req
}

/// Maps an internal TCP port on a NAT-PMP gateway. Returns the external port
/// the router actually picked. A lifetime of 0 removes the mapping again.
fn pmp_map(gw: IpAddr, internal: u16, lifetime: u32) -> Result<u16, String> {
    let addr = SocketAddr::new(gw, PMP_PORT);
    let s = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
    let _ = s.set_read_timeout(Some(Duration::from_millis(800)));
    let req = pmp_request(internal, 0, lifetime);
    let mut last = String::from("router did not answer NAT-PMP");
    for _ in 0..3 {
        if s.send_to(&req, addr).is_err() {
            last = "cannot send to router".into();
            continue;
        }
        let mut buf = [0u8; 32];
        match s.recv_from(&mut buf) {
            Ok((n, _)) if n >= 12 => {
                let result = u16::from_be_bytes([buf[2], buf[3]]);
                if result != 0 {
                    return Err(format!("NAT-PMP refused (code {result})"));
                }
                if buf[1] == PMP_OP_TCP | 0x80 {
                    // reply layout: ver, op, result, epoch(4), private port,
                    // public port, lifetime - the public port is at 10..12
                    return Ok(u16::from_be_bytes([buf[10], buf[11]]));
                }
                last = "router sent a wrong reply opcode".into();
            }
            Ok(_) => continue,
            Err(_) => continue,
        }
    }
    Err(last)
}

// ---------- UPnP IGD ----------

const SSDP_PORT: u16 = 1900;
const SSDP_GROUP: &str = "239.255.255.250";

/// Asks the network for WANIPConnection:1 device descriptors (SSDP M-SEARCH).
fn ssdp_locations() -> Vec<String> {
    let Ok(s) = UdpSocket::bind("0.0.0.0:0") else { return vec![] };
    let _ = s.set_read_timeout(Some(Duration::from_secs(3)));
    let req = format!(
        "M-SEARCH * HTTP/1.1\r\nHOST: {SSDP_GROUP}:{SSDP_PORT}\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\nST: urn:schemas-upnp-org:service:WANIPConnection:1\r\n\r\n"
    );
    use std::net::ToSocketAddrs;
    if let Ok(mut it) = (SSDP_GROUP, SSDP_PORT).to_socket_addrs() {
        if let Some(dst) = it.next() {
            for _ in 0..2 {
                let _ = s.send_to(req.as_bytes(), dst);
            }
        }
    }
    let mut out = Vec::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    let mut buf = [0u8; 4096];
    while std::time::Instant::now() < deadline {
        match s.recv_from(&mut buf) {
            Ok((n, _)) => {
                let text = String::from_utf8_lossy(&buf[..n]);
                for line in text.lines() {
                    if line.to_uppercase().starts_with("LOCATION:") {
                        let loc = line.split_once(':').map(|(_, v)| v.trim()).unwrap_or("");
                        if !loc.is_empty() && !out.iter().any(|x| x == &loc.to_string()) {
                            out.push(loc.to_string());
                        }
                    }
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue
            }
            Err(_) => break,
        }
    }
    out
}

/// Locates the WANIPConnection SOAP control URL in a device descriptor.
/// Returns the absolute control URL and its host:port part.
fn control_url_from(desc: &str, desc_url: &str) -> Option<(String, String)> {
    let marker = "WANIPConnection:1";
    let pos = desc.find(marker)?;
    let rest = &desc[pos..];
    let open = rest.find("<controlURL>")?;
    let open_pos = pos + open;
    let after = &desc[open_pos..];
    let close = after.find("</controlURL>")?;
    let path = after["<controlURL>".len()..][..close - "<controlURL>".len()].to_string();
    let base: String = if let Some(b) = desc.find("<URLBase>") {
        let b_rest = &desc[b + "<URLBase>".len()..];
        let end = b_rest.find("</URLBase>")?;
        b_rest[..end].trim().to_string()
    } else {
        let slash = desc_url.rfind('/')?;
        desc_url[..slash].to_string()
    };
    let full = if path.starts_with("http://") {
        path
    } else if path.starts_with('/') {
        format!("{}{}", base.trim_end_matches('/'), path)
    } else {
        format!("{}/{path}", base.trim_end_matches('/'))
    };
    let (host, back_path) = parse_url(&full).ok()?;
    Some((full, format!("{}{}", host_with_port(&host), back_path)))
}

/// Adds a TCP port mapping on a UPnP router whose descriptor is at
/// `gw_location`. Returns the control URL, needed again to delete the mapping.
fn upnp_add(gw_location: &str, internal: u16, local_ip: Ipv4Addr) -> Result<String, String> {
    let (host, path) = parse_url(gw_location)?;
    let desc = http_get(&host, &path)?;
    let Some((full, _)) = control_url_from(&desc, gw_location) else {
        return Err("router has no WANIPConnection service".into());
    };
    let (chost, cpath) = parse_url(&full)?;
    let action = "urn:schemas-upnp-org:service:WANIPConnection:1#AddPortMapping";
    let body = format!(
        "<?xml version=\"1.0\"?>\
<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body>\
<u:AddPortMapping xmlns:u=\"urn:schemas-upnp-org:service:WANIPConnection:1\">\
<NewRemoteHost></NewRemoteHost>\
<NewExternalPort>{internal}</NewExternalPort>\
<NewProtocol>TCP</NewProtocol>\
<NewInternalPort>{internal}</NewInternalPort>\
<NewInternalClient>{local_ip}</NewInternalClient>\
<NewEnabled>1</NewEnabled>\
<NewPortMappingDescription>enigmatic player listen</NewPortMappingDescription>\
<NewLeaseDuration>{LEASE_SECS}</NewLeaseDuration>\
</u:AddPortMapping></s:Body></s:Envelope>"
    );
    let resp = http_post_soap(&chost, &cpath, action, &body)?;
    if status_ok(&resp) {
        Ok(full)
    } else {
        let why = if resp.contains("713") {
            "router refused the port".to_string()
        } else if resp.contains("\"ExternalPortConflict\"") || resp.contains("718") {
            "external port already in use".to_string()
        } else {
            body_of(&resp).chars().take(120).collect()
        };
        Err(format!("router refused the mapping: {why}"))
    }
}

/// Removes a mapping this session added, best-effort. Handles both the
/// UPnP control URL and the NAT-PMP gateway, whichever succeeded.
pub fn forget(reach: &Reach) {
    if let Some(control) = &reach.control {
        if let Ok((host, path)) = parse_url(control) {
            let action = "urn:schemas-upnp-org:service:WANIPConnection:1#DeletePortMapping";
            let body = format!(
                "<?xml version=\"1.0\"?>\
<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body>\
<u:DeletePortMapping xmlns:u=\"urn:schemas-upnp-org:service:WANIPConnection:1\">\
<NewRemoteHost></NewRemoteHost>\
<NewExternalPort>{}</NewExternalPort>\
<NewProtocol>TCP</NewProtocol>\
</u:DeletePortMapping></s:Body></s:Envelope>",
                reach.pub_port.unwrap_or(reach.in_port)
            );
            let _ = http_post_soap(&host, &path, action, &body);
        }
    }
    if let Some(gw) = reach.pmp_gw {
        let _ = pmp_map(gw, reach.in_port, 0);
    }
}

// ---------- one shared attempt ----------

fn try_pmp(internal: u16, gw: Option<IpAddr>) -> Option<Reach> {
    let gw = gw?;
    let ext = pmp_map(gw, internal, LEASE_SECS).ok()?;
    let mut reach = Reach::default();
    reach.pub_port = Some(ext);
    reach.in_port = internal;
    reach.pmp_gw = Some(gw);
    reach.note = "router opened the port (NAT-PMP)".into();
    if let Some(ip) = public_ip() {
        reach.servers.push(format!("{ip}:{ext}"));
        reach.note.push_str("; invite works on the internet");
    } else {
        reach.note.push_str("; public ip unknown");
    }
    Some(reach)
}

fn try_upnp(internal: u16, local: Option<Ipv4Addr>) -> Option<Reach> {
    let local = local?;
    for loc in ssdp_locations() {
        match upnp_add(&loc, internal, local) {
            Ok(control) => {
                let mut reach = Reach::default();
                reach.pub_port = Some(internal);
                reach.in_port = internal;
                reach.control = Some(control);
                reach.note = "router opened the port (UPnP)".into();
                if let Some(ip) = public_ip() {
                    reach.servers.push(format!("{ip}:{internal}"));
                    reach.note.push_str("; invite works on the internet");
                } else {
                    reach.note.push_str("; public ip unknown");
                }
                return Some(reach);
            }
            Err(_) => continue,
        }
    }
    None
}

/// Tries to make `internal` reachable from the internet, without the user
/// doing anything. Returns the LAN address too so a single invite covers both.
pub fn open_port(internal: u16, lan_ip: Option<Ipv4Addr>) -> Reach {
    if punching_disabled() {
        let mut r = Reach::default();
        if let Some(ip) = lan_ip {
            r.servers.push(format!("{ip}:{internal}"));
        }
        r.note = "auto port opening disabled by env".into();
        return r;
    }

    let gw = default_gateway();
    let local = local_ip();
    let mut r = try_pmp(internal, gw)
        .or_else(|| try_upnp(internal, local))
        .unwrap_or_default();

    // LAN first: guests on the same network should not detour through the router.
    if let Some(ip) = lan_ip {
        r.servers.insert(0, format!("{ip}:{internal}"));
    }
    if r.servers.is_empty() {
        r.note = "room reachable on this network only".to_string();
    } else if r.note.is_empty() {
        r.note = "LAN only (no automatic router mapping)".into();
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pmp_request_bytes() {
        let req = pmp_request(18099, 0, 86_400);
        assert_eq!(req.len(), 12);
        assert_eq!(req[0], 0); // version
        assert_eq!(req[1], 2); // TCP map op
        assert_eq!(&req[4..6], &18099u16.to_be_bytes()); // internal port
        assert_eq!(&req[6..8], &0u16.to_be_bytes()); // suggested external
        assert_eq!(&req[8..12], &86_400u32.to_be_bytes()); // lifetime
    }

    #[test]
    fn parse_url_splits_host_and_path() {
        let (h, p) = parse_url("http://192.168.1.1:54321/wanip?x=1").unwrap();
        assert_eq!(h, "192.168.1.1:54321");
        assert_eq!(p, "/wanip?x=1");
        let (h2, p2) = parse_url("http://checkip.amazonaws.com").unwrap();
        assert_eq!(h2, "checkip.amazonaws.com");
        assert_eq!(p2, "/");
        assert!(parse_url("https://example.com").is_err());
    }

    #[test]
    fn host_without_port_gets_defauled() {
        assert_eq!(host_with_port("checkip.amazonaws.com"), "checkip.amazonaws.com:80");
        assert_eq!(host_with_port("1.2.3.4:8443"), "1.2.3.4:8443");
    }

    #[test]
    fn control_url_extraction() {
        // a realistic router descriptor: relative controlURL, no URLBase
        let desc = "<?xml?><root><device><serviceList><service>\
<serviceType>urn:schemas-upnp-org:service:WANIPConnection:1</serviceType>\
<controlURL>/ctl/IPConn</controlURL></service></serviceList></device></root>";
        let (full, hp) = control_url_from(desc, "http://192.168.1.1:49152/gateway.xml").unwrap();
        assert_eq!(full, "http://192.168.1.1:49152/ctl/IPConn");
        assert_eq!(hp, "192.168.1.1:49152/ctl/IPConn");

        // absolute controlURL wins
        let abs = "<controlURL>http://router.local/soap</controlURL>";
        let d2 = format!("WANIPConnection:1 {abs}");
        let (full2, _) = control_url_from(&d2, "http://ignored:1/x.xml").unwrap();
        assert_eq!(full2, "http://router.local/soap");
    }

    #[test]
    fn reach_note_is_honest_without_mapping() {
        std::env::set_var("ENIGMATIC_NO_PUNCH", "1");
        let r = open_port(18099, None);
        assert!(r.servers.is_empty());
        assert!(r.note.contains("disabled"));
        std::env::remove_var("ENIGMATIC_NO_PUNCH");
    }

    /// Live probe against the real router: ask for a mapping, report honestly,
    /// then remove it again. `cargo test punch_live -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn punch_live() {
        let lan = local_ip();
        println!("lan ip: {lan:?}");
        println!("gateway: {:?}", default_gateway());
        let reach = open_port(18199, lan);
        println!("note: {}", reach.note);
        println!("servers: {:?}", reach.servers);
        println!("pub_port: {:?} control: {:?}", reach.pub_port, reach.control);
        forget(&reach);
        println!("mapping removed again (forget was called)");
    }
}
