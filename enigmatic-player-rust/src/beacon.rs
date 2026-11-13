use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// LAN room discovery, no server and no typing:
/// - a host's app announces its room every ~3 seconds on a loopback LAN UDP broadcast
/// - every app runs a quiet listener that collects those announcements
///
/// Announcements carry room code, host name and the host's iroh endpoint id;
/// nothing else leaves the machine. This is best-effort: some Wi-Fi access
/// points and the OS firewall swallow broadcasts, in which case guests can
/// still paste the invite code directly.

const PORT: u16 = 18066;
const BROADCAST: &str = "255.255.255.255";
/// An announcement older than this is treated as the host having left.
const MAX_AGE: Duration = Duration::from_secs(12);
/// Announce interval (a bit under MAX_AGE so one lost packet is survivable);
/// split into 10 stop-checkable steps.
const EVERY: Duration = Duration::from_secs(3);
/// A room is only believed after seeing it twice, to ignore stray spoof noise.
const MIN_HITS: u32 = 2;

#[derive(Serialize, Deserialize)]
struct Ad {
    v: u8,
    code: String,
    host: String,
    server: String,
}

/// A room the listener has heard about recently.
#[derive(Clone, Debug, PartialEq)]
pub struct RoomAd {
    pub code: String,
    pub host_name: String,
    pub server: String,
    pub seen: Instant,
    pub hits: u32,
}

impl RoomAd {
    fn age(&self) -> Duration {
        self.seen.elapsed()
    }
}

/// A short name for showing in the join list: host name sans the UDP noise.
// ---------- shared collected list ----------

fn rooms_cell() -> &'static Mutex<Vec<RoomAd>> {
    static CELL: OnceLock<Mutex<Vec<RoomAd>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(Vec::new()))
}

/// Records one announcement; testable without sockets.
fn record(code: &str, host: &str, server: &str, now: Instant) {
    let mut all = rooms_cell().lock().unwrap();
    let (code, host, server) = (code.to_string(), host.to_string(), server.to_string());
    all.retain(|a| now.duration_since(a.seen) < MAX_AGE);
    if code.len() < 4 || code.len() > 8 || host.is_empty() {
        return;
    }
    if let Some(ad) = all.iter_mut().find(|a| a.code == code) {
        ad.hits += 1;
        ad.seen = now;
        if !ad.host_name.is_empty() && ad.server != server && !server.is_empty() {
            ad.server = server; // the host may have refreshed its address list
        }
    } else {
        all.push(RoomAd { code, host_name: host, server, seen: now, hits: 1 });
    }
}

/// Rooms discovered on this network, oldest-ignored: only fresh ones survive
/// and each has been seen at least twice. Sorted by host name for the pop-up.
pub fn rooms() -> Vec<RoomAd> {
    let mut all = rooms_cell().lock().unwrap();
    all.retain(|a| a.age() < MAX_AGE);
    let mut fresh: Vec<RoomAd> = all
        .iter()
        .filter(|a| a.hits >= MIN_HITS)
        .cloned()
        .collect();
    fresh.sort_by(|a, b| a.host_name.cmp(&b.host_name));
    drop(all);
    fresh
}

/// Forget everything (used on stop, so a shuttered room stops offering itself).
pub fn clear() {
    rooms_cell().lock().unwrap().clear();
}

// ---------- background threads ----------

/// Started by a host; announces its room until stopped.
pub struct Announcer {
    stop: Arc<AtomicBool>,
}

impl Announcer {
    pub fn start(code: &str, host_name: &str, server: &str) -> Announcer {
        let stop = Arc::new(AtomicBool::new(false));
        let task = Arc::clone(&stop);
        let code = code.to_string();
        let host = host_name.to_string();
        let server = server.to_string();
        thread::Builder::new().name("beacon-announce".into()).spawn(move || {
            let Ok(sock) = UdpSocket::bind("0.0.0.0:0") else { return };
            let _ = sock.set_broadcast(true);
            let ad = Ad { v: 1, code: code.clone(), host: host.clone(), server: server.clone() };
            let payload = match serde_json::to_vec(&ad) {
                Ok(p) => p,
                Err(_) => return,
            };
            loop {
                let _ = sock.send_to(&payload, (BROADCAST, PORT));
                let steps = (EVERY.as_millis() / 100).max(1) as usize;
                for _ in 0..steps {
                    if task.load(Ordering::Relaxed) {
                        return;
                    }
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }).ok();
        Announcer { stop }
    }

    /// Stops announcing and forgets the advertised room.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        clear();
    }
}

impl Drop for Announcer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Starts the one-shot global listener; returns false if the port could not
/// be bound (another local app instance won it, or the OS refused).
pub fn listen_once() -> bool {
    static STARTED: OnceLock<bool> = OnceLock::new();
    *STARTED.get_or_init(|| {
        thread::Builder::new().name("beacon-listen".into()).spawn(|| {
            let Ok(sock) = UdpSocket::bind(("0.0.0.0", PORT)) else { return };
            let _ = sock.set_read_timeout(Some(Duration::from_secs(5)));
            let mut buf = [0u8; 1024];
            loop {
                match sock.recv_from(&mut buf) {
                    Ok((n, _)) => {
                        if let Ok(ad) = serde_json::from_slice::<Ad>(&buf[..n]) {
                            if ad.v == 1 {
                                record(&ad.code, &ad.host, &ad.server, Instant::now());
                            }
                        }
                    }
                    Err(_) => continue, // timeout while nothing is announced
                }
            }
        }).is_ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The room registry is process-global, so these tests must not run at
    /// the same time or they clobber each other's rooms.
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    #[test]
    fn ad_payload_roundtrip() {
        let ad = Ad { v: 1, code: "ABC234".into(), host: "Sam's room".into(), server: "192.168.1.5:40000|8.8.4.4:40000".into() };
        let bytes = serde_json::to_vec(&ad).unwrap();
        let back: Ad = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back.code, "ABC234");
        assert_eq!(back.host, "Sam's room");
        assert_eq!(back.server, "192.168.1.5:40000|8.8.4.4:40000");
    }

    #[test]
    fn junk_ads_are_ignored() {
        let _g = serial();
        rooms_cell().lock().unwrap().clear();
        record("", "host", "1.2.3.4:1", Instant::now());
        record("ab", "host", "1.2.3.4:1", Instant::now());
        record("ABCDEFGHIJKL", "host", "1.2.3.4:1", Instant::now());
        rooms_cell().lock().unwrap().retain(|a| a.age() < MAX_AGE);
        assert!(rooms().is_empty());
        rooms_cell().lock().unwrap().clear();
    }

    #[test]
    fn rooms_need_two_hits_and_freshness() {
        let _g = serial();
        rooms_cell().lock().unwrap().clear();
        let now = Instant::now();
        record("AA11BB", "Host One", "192.168.1.5:10000", now);
        // One hit only: never believed.
        assert!(
            rooms().iter().all(|a| a.code != "AA11BB"),
            "a single announcement must not show up yet"
        );
        // Two hits: believed (freshness tested by the rooms() age filter).
        record("AA11BB", "Host One", "192.168.1.5:10000", now);
        let list = rooms();
        let found = list.iter().find(|a| a.code == "AA11BB");
        assert!(found.is_some());
        assert_eq!(found.unwrap().host_name, "Host One");

        // Stale entries disappear from the visible list.
        let stale = RoomAd { code: "CC33DD".into(), host_name: "Old".into(), server: "1.1.1.1:1".into(), seen: now - MAX_AGE - Duration::from_secs(1), hits: 9 };
        rooms_cell().lock().unwrap().push(stale);
        rooms_cell().lock().unwrap().retain(|a| a.age() < MAX_AGE);
        assert!(rooms().iter().all(|a| a.code != "CC33DD"));
        rooms_cell().lock().unwrap().clear();
    }

    #[test]
    fn host_name_refresh_keeps_server_in_sync() {
        let _g = serial();
        rooms_cell().lock().unwrap().clear();
        let now = Instant::now();
        record("BB22CC", "Host Two", "192.168.1.5:10000", now);
        record("BB22CC", "Host Two", "10.0.0.2:10000", now);
        let list = rooms();
        let found = list.iter().find(|a| a.code == "BB22CC").unwrap();
        assert_eq!(found.server, "10.0.0.2:10000");
        rooms_cell().lock().unwrap().clear();
    }

    /// The real wire path: the actual listener thread must pick up actual
    /// broadcast packets sent through the OS network stack.
    #[test]
    fn listener_receives_broadcasts() {
        let _g = serial();
        rooms_cell().lock().unwrap().clear();
        assert!(listen_once(), "the listener should bind its port once");

        let tx = UdpSocket::bind("0.0.0.0:0").unwrap();
        tx.set_broadcast(true).unwrap();
        let ad = Ad { v: 1, code: "ZZ01ZZ".into(), host: "Wire Test".into(), server: "192.168.1.9:31999".into() };
        let payload = serde_json::to_vec(&ad).unwrap();
        // two announcements: the listener only believes a room after two hits
        for _ in 0..2 {
            tx.send_to(&payload, (BROADCAST, PORT)).unwrap();
            thread::sleep(Duration::from_millis(600));
        }
        let list = rooms();
        let found = list.iter().find(|a| a.code == "ZZ01ZZ");
        assert!(found.is_some(), "a real broadcast should be discovered: {list:?}");
        assert_eq!(found.unwrap().host_name, "Wire Test");
        rooms_cell().lock().unwrap().clear();
    }
}
