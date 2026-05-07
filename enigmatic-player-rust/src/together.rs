use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::sync::mpsc::{channel, Sender};
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::share::SharedTrack;

/// Playback state shared with everyone in a room. Guests follow this; the
/// audio itself is never sent over the network.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RoomState {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub queue: Vec<SharedTrack>,
    #[serde(default)]
    pub index: usize,
    #[serde(default)]
    pub pos: f64,
    #[serde(default)]
    pub playing: bool,
    #[serde(default = "default_speed")]
    pub speed: f32,
}

fn default_speed() -> f32 {
    1.0
}

impl Default for RoomState {
    fn default() -> Self {
        RoomState {
            label: String::new(),
            queue: Vec::new(),
            index: 0,
            pos: 0.0,
            playing: false,
            speed: 1.0,
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    Create { name: String, state: RoomState },
    Join { code: String, name: String },
    State { state: RoomState },
    Leave,
    Ping,
}

impl ClientMsg {
    fn line(&self) -> Option<String> {
        serde_json::to_string(self).ok().map(|mut s| {
            s.push('\n');
            s
        })
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    Created {
        code: String,
        token: String,
        host: String,
        members: Vec<String>,
        state: RoomState,
    },
    Joined {
        code: String,
        token: String,
        host: String,
        members: Vec<String>,
        state: RoomState,
    },
    Sync {
        host: String,
        members: Vec<String>,
        state: RoomState,
    },
    Members {
        host: String,
        members: Vec<String>,
    },
    Error {
        message: String,
    },
    Pong,
}

/// Events delivered to the UI thread.
#[derive(Debug)]
pub enum Event {
    Created {
        server: String,
        code: String,
        host: String,
        members: Vec<String>,
        state: RoomState,
    },
    Joined {
        server: String,
        code: String,
        host: String,
        members: Vec<String>,
        state: RoomState,
    },
    Sync {
        host: String,
        members: Vec<String>,
        state: RoomState,
    },
    Members {
        host: String,
        members: Vec<String>,
    },
    Closed(String),
    Error(String),
}

pub struct RoomHandle {
    pub code: String,
    pub host: bool,
    pub server: String,
    pub me: String,
    pub host_name: String,
    pub members: Vec<String>,
    pub connected: bool,
    pub pending: bool,
    pub state: RoomState,
    pub tx: Sender<ClientMsg>,
}

impl RoomHandle {
    pub fn invite(&self) -> String {
        if self.code.is_empty() {
            String::new()
        } else {
            encode_invite(&self.server, &self.code)
        }
    }

    pub fn send_state(&self, state: RoomState) {
        let _ = self.tx.send(ClientMsg::State { state });
    }

    pub fn ping(&self) {
        let _ = self.tx.send(ClientMsg::Ping);
    }

    pub fn leave(&self) {
        let _ = self.tx.send(ClientMsg::Leave);
    }
}

/// An invite's server field may hold several reachable addresses, separated
/// by `|` (e.g. the LAN address of the host and its public router-mapped one).
/// Returns unique, non-empty candidates in the order the host listed them.
pub fn server_options(server: &str) -> Vec<String> {
    let mut out = Vec::new();
    for s in server.split('|') {
        let s = s.trim();
        if !s.is_empty() && !out.iter().any(|x| x == s) {
            out.push(s.to_string());
        }
    }
    out
}

fn open(server: &str) -> Result<(TcpStream, String), String> {
    let mut options = server_options(server);
    if options.is_empty() {
        options.push(server.to_string());
    }
    let mut last = format!("cannot reach sync server {server}");
    for opt in &options {
        match open_one(opt) {
            Ok(s) => return Ok((s, opt.clone())),
            Err(e) => last = e,
        }
    }
    Err(last)
}

fn open_one(server: &str) -> Result<TcpStream, String> {
    let addrs = server
        .to_socket_addrs()
        .map_err(|e| format!("cannot resolve sync server {server}: {e}"))?;
    let mut last = format!("cannot reach sync server {server}");
    for a in addrs {
        match TcpStream::connect_timeout(&a, Duration::from_secs(4)) {
            Ok(s) => {
                let _ = s.set_nodelay(true);
                let _ = s.set_read_timeout(Some(Duration::from_secs(60)));
                return Ok(s);
            }
            Err(e) => last = format!("cannot reach sync server {server}: {e}"),
        }
    }
    Err(last)
}

fn spawn(
    server: &str,
    first: ClientMsg,
    me: &str,
    events: Sender<Event>,
) -> Result<Sender<ClientMsg>, String> {
    let (stream, _reached) = open(server)?;
    let mut writer = stream.try_clone().map_err(|e| e.to_string())?;
    if let Some(line) = first.line() {
        writer
            .write_all(line.as_bytes())
            .and_then(|_| writer.flush())
            .map_err(|e| format!("cannot talk to sync server: {e}"))?;
    }

    let (tx, rx) = channel::<ClientMsg>();
    let server_owned = server.to_string();
    let me = me.to_string();
    let reader_events = events.clone();

    thread::spawn(move || {
        let reader = BufReader::new(stream);
        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                    continue;
                }
                Err(e) => {
                    let _ = reader_events.send(Event::Closed(format!("room connection lost: {e}")));
                    return;
                }
            };
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<ServerMsg>(&line) {
                Ok(ServerMsg::Created { code, host, members, state, .. }) => {
                    let _ = reader_events.send(Event::Created {
                        server: server_owned.clone(),
                        code,
                        host,
                        members,
                        state,
                    });
                }
                Ok(ServerMsg::Joined { code, host, members, state, .. }) => {
                    let _ = reader_events.send(Event::Joined {
                        server: server_owned.clone(),
                        code,
                        host,
                        members,
                        state,
                    });
                }
                Ok(ServerMsg::Sync { host, members, state }) => {
                    let _ = reader_events.send(Event::Sync { host, members, state });
                }
                Ok(ServerMsg::Members { host, members }) => {
                    let _ = reader_events.send(Event::Members { host, members });
                }
                Ok(ServerMsg::Error { message }) => {
                    let _ = reader_events.send(Event::Error(message));
                }
                Ok(ServerMsg::Pong) => {}
                Err(e) => {
                    let _ = reader_events.send(Event::Error(format!("bad message from sync server: {e}")));
                }
            }
        }
        let _ = reader_events.send(Event::Closed("room closed by the sync server".into()));
    });

    thread::spawn(move || {
        while let Ok(msg) = rx.recv() {
            let leave = matches!(msg, ClientMsg::Leave);
            let Some(line) = msg.line() else { continue };
            if writer.write_all(line.as_bytes()).and_then(|_| writer.flush()).is_err() {
                break;
            }
            if leave {
                let _ = writer.shutdown(Shutdown::Both);
                break;
            }
        }
    });

    let _ = me;
    Ok(tx)
}

pub fn create(
    server: &str,
    me: &str,
    state: RoomState,
    events: Sender<Event>,
) -> Result<RoomHandle, String> {
    let tx = spawn(
        server,
        ClientMsg::Create {
            name: me.to_string(),
            state: state.clone(),
        },
        me,
        events,
    )?;
    Ok(RoomHandle {
        code: String::new(),
        host: true,
        server: server.to_string(),
        me: me.to_string(),
        host_name: me.to_string(),
        members: Vec::new(),
        connected: true,
        pending: true,
        state,
        tx,
    })
}

pub fn join(
    server: &str,
    me: &str,
    code: &str,
    events: Sender<Event>,
) -> Result<RoomHandle, String> {
    let code = code.trim().to_uppercase();
    if code.len() < 4 {
        return Err("that invite code is too short".into());
    }
    let tx = spawn(
        server,
        ClientMsg::Join {
            code: code.clone(),
            name: me.to_string(),
        },
        me,
        events,
    )?;
    Ok(RoomHandle {
        code,
        host: false,
        server: server.to_string(),
        me: me.to_string(),
        host_name: String::new(),
        members: Vec::new(),
        connected: true,
        pending: true,
        state: RoomState::default(),
        tx,
    })
}

const INVITE_PREFIX: &str = "EROOM1:";

pub fn encode_invite(server: &str, code: &str) -> String {
    format!(
        "{INVITE_PREFIX}{}:{}",
        hex(server.as_bytes()),
        code.to_uppercase()
    )
}

pub fn decode_invite(text: &str) -> Result<(String, String), String> {
    let text = text.trim();
    let body = text
        .strip_prefix(INVITE_PREFIX)
        .or_else(|| text.strip_prefix("eroom1:"))
        .ok_or_else(|| "not a room invite".to_string())?;
    let (server, code) = body
        .split_once(':')
        .ok_or_else(|| "room invite is incomplete".to_string())?;
    let server = String::from_utf8(unhex(server)?)
        .map_err(|_| "room invite has an invalid server address".to_string())?;
    let code = code.trim().to_uppercase();
    if server.is_empty() || code.len() < 4 {
        return Err("room invite is incomplete".into());
    }
    Ok((server, code))
}

pub fn looks_like_invite(text: &str) -> bool {
    let t = text.trim();
    t.starts_with(INVITE_PREFIX) || t.to_lowercase().starts_with("eroom1:")
}

fn hex(data: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(data.len() * 2);
    for b in data {
        let _ = write!(out, "{b:02x}");
    }
    out
}

fn unhex(s: &str) -> Result<Vec<u8>, String> {
    if s.len() % 2 != 0 {
        return Err("room invite is cut off".into());
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let b = s.as_bytes();
    for i in (0..b.len()).step_by(2) {
        let hi = (b[i] as char)
            .to_digit(16)
            .ok_or_else(|| "room invite has an invalid character".to_string())?;
        let lo = (b[i + 1] as char)
            .to_digit(16)
            .ok_or_else(|| "room invite has an invalid character".to_string())?;
        out.push((hi * 16 + lo) as u8);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::share::SharedTrack;
    use std::net::TcpListener;
    use std::sync::mpsc::Receiver;

    #[test]
    fn invite_roundtrip() {
        let invite = encode_invite("192.168.1.20:8080", "ABC234");
        let (server, code) = decode_invite(&invite).unwrap();
        assert_eq!(server, "192.168.1.20:8080");
        assert_eq!(code, "ABC234");
        assert!(looks_like_invite(&invite));
        assert!(!looks_like_invite("ELIST1:abc"));
    }

    #[test]
    fn invite_rejects_junk() {
        assert!(decode_invite("hi").is_err());
        assert!(decode_invite("EROOM1:zz:ABC").is_err());
        assert!(decode_invite("EROOM1:61:ABC").is_err());
    }

    #[test]
    fn invite_keeps_multi_server_addresses() {
        let multi = "192.168.1.20:8080|203.0.113.7:56789";
        let invite = encode_invite(multi, "ABC234");
        assert!(looks_like_invite(&invite));
        let (server, code) = decode_invite(&invite).unwrap();
        assert_eq!(server, multi);
        assert_eq!(code, "ABC234");

        let opts = server_options(&server);
        assert_eq!(opts.len(), 2);
        assert_eq!(opts[0], "192.168.1.20:8080");
        assert_eq!(opts[1], "203.0.113.7:56789");
    }

    #[test]
    fn server_options_dedupes_and_skips_blanks() {
        let opts = server_options(" a:1 |  | a:1 |b:2|");
        assert_eq!(opts, vec!["a:1".to_string(), "b:2".to_string()]);
        assert!(server_options("").is_empty());
    }

    #[test]
    fn room_state_roundtrip() {
        let state = RoomState {
            label: "mix".into(),
            queue: vec![SharedTrack {
                title: "Song".into(),
                artist: "Band".into(),
                secs: 3.0,
                url: None,
                file: Some("song.mp3".into()),
            }],
            index: 0,
            pos: 1.25,
            playing: true,
            speed: 1.0,
        };
        let json = serde_json::to_string(&state).unwrap();
        let back: RoomState = serde_json::from_str(&json).unwrap();
        assert_eq!(back.queue[0].title, "Song");
        assert!(back.playing);
    }

    /// The real client threads (spawn/create/join/leave) against the real
    /// server: create, follow the invite string, join, sync, leave.
    #[test]
    fn client_speaks_to_server() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            let _ = crate::server::serve(listener);
        });
        let server = format!("127.0.0.1:{}", addr.port());
        let st = RoomState {
            label: "client smoke".into(),
            queue: vec![SharedTrack {
                title: "One".into(),
                artist: "Band".into(),
                secs: 10.0,
                url: None,
                file: Some("one.mp3".into()),
            }],
            index: 0,
            pos: 0.0,
            playing: false,
            speed: 1.0,
        };

        let (etx, erx) = channel::<Event>();
        let mut host = create(&server, "Host", st.clone(), etx.clone()).unwrap();
        assert!(host.pending);
        match erx.recv_timeout(Duration::from_secs(3)).unwrap() {
            Event::Created { code, .. } => {
                assert_eq!(code.len(), 8);
                assert!(code.chars().all(|c| c.is_ascii_digit()));
                // the app fills these from the event (App::on_room_event)
                host.code = code;
                host.pending = false;
            }
            other => panic!("host expected Created, got {other:?}"),
        }
        assert!(!host.pending);
        // the invite the user would copy must decode and re-join correctly
        let (srv, code) = decode_invite(&host.invite()).unwrap();
        assert_eq!(srv, server);

        let _guest = join(&srv, "Guest", &code, etx).unwrap();
        let joined = |rx: &Receiver<Event>| -> RoomState {
            loop {
                match rx.recv_timeout(Duration::from_secs(3)).unwrap() {
                    Event::Joined { state, .. } => break state,
                    Event::Members { .. } => continue,
                    other => panic!("guest expected Joined, got {other:?}"),
                }
            }
        };
        let joined_state = joined(&erx);
        assert_eq!(joined_state.label, "client smoke");

        let mut next = st;
        next.pos = 7.5;
        next.playing = true;
        host.send_state(next);
        loop {
            match erx.recv_timeout(Duration::from_secs(3)).unwrap() {
                Event::Sync { state, .. } => {
                    assert_eq!(state.pos, 7.5);
                    assert!(state.playing);
                    break;
                }
                Event::Members { .. } => continue,
                other => panic!("guest expected Sync, got {other:?}"),
            }
        }

        host.leave();
        loop {
            match erx.recv_timeout(Duration::from_secs(5)).unwrap() {
                Event::Error { .. } => { /* host ended the session */ }
                Event::Closed(_) => break,
                Event::Members { .. } => continue,
                other => panic!("guest expected closure, got {other:?}"),
            }
        }
    }
}
