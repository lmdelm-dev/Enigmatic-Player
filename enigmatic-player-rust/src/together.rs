//! Listen-together transport.
//!
//! Rooms are hosted by the player itself over iroh, a peer-to-peer QUIC
//! network: the invite carries the host's public key, so guests can connect
//! from any network without accounts, port forwarding or a relay set up by the
//! user. When a direct connection cannot be punched through the routers, iroh
//! falls back to an encrypted public relay; only ciphertext passes through it.
//!
//! The wire protocol is newline-delimited JSON over one bidirectional QUIC
//! stream per guest. Playback state is tiny (queue metadata, position, speed);
//! audio never leaves the machine.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use iroh::endpoint::{presets, Incoming, SendStream};
use iroh::{Endpoint, EndpointAddr, PublicKey, SecretKey};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{broadcast, mpsc};

use crate::share::SharedTrack;

const ALPN: &[u8] = b"enigmatic-player/room/2";
const MAX_MEMBERS: usize = 16;
const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
/// How long to wait for the public relay before giving up on it. Dials still
/// work afterwards; this only avoids announcing a room too early.
const ONLINE_WAIT: Duration = Duration::from_secs(15);
/// Total time a guest keeps retrying while the host's address propagates.
const CONNECT_WAIT: Duration = Duration::from_secs(45);

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
        if self.code.is_empty() || self.server.is_empty() {
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

/// Hosts the current session from this process. The returned handle is live
/// immediately; the endpoint id arrives with `Event::Created` once networking
/// is ready.
pub fn create(me: &str, state: RoomState, events: Sender<Event>) -> Result<RoomHandle, String> {
    let code = new_code();
    let (tx, rx) = channel::<ClientMsg>();
    let me = me.to_string();
    let me_thread = me.clone();
    let code_thread = code.clone();
    let handle_state = state.clone();
    let ev = events.clone();

    thread::Builder::new()
        .name("room-host".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .worker_threads(2)
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = ev.send(Event::Error(format!("cannot start room networking: {e}")));
                    return;
                }
            };
            if let Err(e) = rt.block_on(host_main(me_thread, code_thread, state, rx, ev.clone())) {
                let _ = ev.send(Event::Error(e));
            }
        })
        .map_err(|e| format!("cannot start a room: {e}"))?;

    Ok(RoomHandle {
        code,
        host: true,
        server: String::new(),
        me: me.clone(),
        host_name: me,
        members: Vec::new(),
        connected: true,
        pending: true,
        state: handle_state,
        tx,
    })
}

/// Joins a room through its endpoint id (the address carried by the invite).
pub fn join(server: &str, me: &str, code: &str, events: Sender<Event>) -> Result<RoomHandle, String> {
    let code = code.trim().to_uppercase();
    if code.len() < 4 {
        return Err("that invite code is too short".into());
    }
    let server = server.trim().to_string();
    if server.is_empty() {
        return Err("that invite has no room address".into());
    }
    PublicKey::from_str(&server).map_err(|e| format!("that room address is invalid: {e}"))?;

    let (tx, rx) = channel::<ClientMsg>();
    let me = me.to_string();
    let me_thread = me.clone();
    let server_thread = server.clone();
    let code_thread = code.clone();
    let ev = events.clone();

    thread::Builder::new()
        .name("room-guest".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .worker_threads(2)
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = ev.send(Event::Error(format!("cannot start room networking: {e}")));
                    return;
                }
            };
            if let Err(e) = rt.block_on(guest_main(
                server_thread,
                me_thread,
                code_thread,
                rx,
                ev.clone(),
            )) {
                let _ = ev.send(Event::Error(e));
            }
        })
        .map_err(|e| format!("cannot join a room: {e}"))?;

    Ok(RoomHandle {
        code,
        host: false,
        server,
        me: me.clone(),
        host_name: String::new(),
        members: Vec::new(),
        connected: true,
        pending: true,
        state: RoomState::default(),
        tx,
    })
}

// ---------- host side ----------

struct RoomShared {
    code: String,
    host: String,
    state: Mutex<RoomState>,
    members: Mutex<Vec<String>>,
    bcast: broadcast::Sender<Arc<String>>,
    events: Sender<Event>,
}

impl RoomShared {
    fn snapshot(&self) -> (String, Vec<String>, RoomState) {
        let state = self.state.lock().unwrap().clone();
        let members = self.members.lock().unwrap().clone();
        (self.host.clone(), members, state)
    }

    fn add_member(&self, name: &str) {
        self.members.lock().unwrap().push(name.to_string());
    }

    fn remove_member(&self, name: &str) {
        let mut members = self.members.lock().unwrap();
        if let Some(i) = members.iter().position(|m| m == name) {
            members.remove(i);
        }
    }

    fn is_full(&self) -> bool {
        self.members.lock().unwrap().len() >= MAX_MEMBERS
    }

    fn broadcast_members(&self) {
        let (host, members, _) = self.snapshot();
        if let Some(line) = json_line(&ServerMsg::Members {
            host: host.clone(),
            members: members.clone(),
        }) {
            let _ = self.bcast.send(line);
        }
        let _ = self.events.send(Event::Members { host, members });
    }

    fn broadcast_sync(&self, state: RoomState) {
        let (host, members, _) = self.snapshot();
        if let Some(line) = json_line(&ServerMsg::Sync {
            host,
            members,
            state,
        }) {
            let _ = self.bcast.send(line);
        }
    }
}

async fn host_main(
    me: String,
    code: String,
    state: RoomState,
    rx: Receiver<ClientMsg>,
    events: Sender<Event>,
) -> Result<(), String> {
    let key = load_identity()?;
    let endpoint = Endpoint::builder(presets::N0)
        .secret_key(key)
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
        .map_err(|e| format!("cannot open a room here: {e}"))?;
    // Register with a relay before handing out the invite. This is what makes
    // the endpoint id resolvable from another network.
    let _ = tokio::time::timeout(ONLINE_WAIT, endpoint.online()).await;
    let endpoint_id = endpoint.id().to_string();

    let (bcast, _) = broadcast::channel::<Arc<String>>(64);
    let room = Arc::new(RoomShared {
        code: code.clone(),
        host: me.clone(),
        state: Mutex::new(state.clone()),
        members: Mutex::new(vec![me.clone()]),
        bcast,
        events: events.clone(),
    });

    // The UI talks over a std channel; this thread turns it into an async one
    // so the runtime is never blocked waiting for the player.
    let (ctx, mut crx) = mpsc::unbounded_channel::<ClientMsg>();
    thread::Builder::new()
        .name("room-control".into())
        .spawn(move || {
            while let Ok(msg) = rx.recv() {
                if ctx.send(msg).is_err() {
                    break;
                }
            }
        })
        .ok();

    let _ = events.send(Event::Created {
        server: endpoint_id,
        code: code.clone(),
        host: me.clone(),
        members: vec![me],
        state,
    });

    let accept_ep = endpoint.clone();
    let accept_room = room.clone();
    let accept = tokio::spawn(async move {
        while let Some(incoming) = accept_ep.accept().await {
            let room = accept_room.clone();
            tokio::spawn(async move { handle_guest(incoming, room).await });
        }
    });

    while let Some(msg) = crx.recv().await {
        match msg {
            ClientMsg::State { state } => {
                *room.state.lock().unwrap() = state.clone();
                room.broadcast_sync(state);
            }
            ClientMsg::Leave => break,
            _ => {}
        }
    }

    accept.abort();
    endpoint.close().await;
    Ok(())
}

async fn handle_guest(incoming: Incoming, room: Arc<RoomShared>) {
    let Ok(conn) = incoming.await else { return };
    let Ok((mut send, recv)) = conn.accept_bi().await else {
        conn.close(0u8.into(), b"bye");
        return;
    };
    let mut reader = BufReader::new(recv);
    let mut line = String::new();
    let first = reader.read_line(&mut line).await.unwrap_or_default();
    if first == 0 {
        conn.close(0u8.into(), b"bye");
        return;
    }

    let name = match serde_json::from_str::<ClientMsg>(&line) {
        Ok(ClientMsg::Join { code, name }) if code.trim().eq_ignore_ascii_case(&room.code) => {
            clean_name(&name)
        }
        _ => {
            reject(&mut send, "this invite does not open this room").await;
            conn.close(0u8.into(), b"bye");
            return;
        }
    };
    if room.is_full() {
        reject(&mut send, "that room is full").await;
        conn.close(0u8.into(), b"bye");
        return;
    }

    // Subscribe before the first snapshot is sent, so no update can slip
    // between the Joined message and the writer task starting.
    let mut updates = room.bcast.subscribe();
    let (host, members, state) = room.snapshot();
    let joined = ServerMsg::Joined {
        code: room.code.clone(),
        token: String::new(),
        host,
        members,
        state,
    };
    if write_msg(&mut send, &joined).await.is_err() {
        conn.close(0u8.into(), b"bye");
        return;
    }

    let writer = tokio::spawn(async move {
        loop {
            match updates.recv().await {
                Ok(line) => {
                    if send.write_all(line.as_bytes()).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
        let _ = send.finish();
    });

    room.add_member(&name);
    room.broadcast_members();

    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => break,
            Ok(_) => {
                // Only the host changes playback; guest state is ignored.
                if let Ok(ClientMsg::Leave) = serde_json::from_str::<ClientMsg>(&line) {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    writer.abort();
    room.remove_member(&name);
    room.broadcast_members();
    conn.close(0u8.into(), b"bye");
}

async fn reject(send: &mut SendStream, message: &str) {
    let _ = write_msg(
        send,
        &ServerMsg::Error {
            message: message.to_string(),
        },
    )
    .await;
    let _ = send.finish();
}

// ---------- guest side ----------

async fn guest_main(
    server: String,
    me: String,
    code: String,
    rx: Receiver<ClientMsg>,
    events: Sender<Event>,
) -> Result<(), String> {
    let id = PublicKey::from_str(&server).map_err(|e| format!("that room address is invalid: {e}"))?;
    let key = load_identity()?;
    let endpoint = Endpoint::builder(presets::N0)
        .secret_key(key)
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
        .map_err(|e| format!("cannot open the room connection: {e}"))?;

    // The address lookup may need a moment after the host published itself,
    // so keep trying until the deadline instead of failing on the first miss.
    let deadline = Instant::now() + CONNECT_WAIT;
    let conn = loop {
        match endpoint.connect(EndpointAddr::new(id), ALPN).await {
            Ok(conn) => break conn,
            Err(e) => {
                if Instant::now() >= deadline {
                    endpoint.close().await;
                    return Err(format!("cannot reach the host: {e}"));
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    };
    let (mut send, recv) = conn
        .open_bi()
        .await
        .map_err(|e| format!("cannot open a room channel: {e}"))?;

    let join = json_line(&ClientMsg::Join {
        code: code.clone(),
        name: me,
    })
    .ok_or_else(|| "cannot encode the join request".to_string())?;
    send.write_all(join.as_bytes())
        .await
        .map_err(|e| format!("cannot talk to the host: {e}"))?;

    let mut reader = BufReader::new(recv);
    let mut line = String::new();
    match reader.read_line(&mut line).await {
        Ok(0) => {
            let _ = events.send(Event::Error("the host closed the room".into()));
            endpoint.close().await;
            return Ok(());
        }
        Ok(_) => {}
        Err(e) => {
            let _ = events.send(Event::Error(format!("cannot reach the host: {e}")));
            endpoint.close().await;
            return Ok(());
        }
    }

    match serde_json::from_str::<ServerMsg>(&line) {
        Ok(ServerMsg::Joined {
            code,
            host,
            members,
            state,
            ..
        }) => {
            let _ = events.send(Event::Joined {
                server,
                code,
                host,
                members,
                state,
            });
        }
        Ok(ServerMsg::Error { message }) => {
            let _ = events.send(Event::Error(message));
            endpoint.close().await;
            return Ok(());
        }
        Ok(_) => {
            let _ = events.send(Event::Error("unexpected reply from the host".into()));
            endpoint.close().await;
            return Ok(());
        }
        Err(e) => {
            let _ = events.send(Event::Error(format!("bad reply from the host: {e}")));
            endpoint.close().await;
            return Ok(());
        }
    }

    let (ctx, mut crx) = mpsc::unbounded_channel::<ClientMsg>();
    thread::Builder::new()
        .name("room-guest-control".into())
        .spawn(move || {
            while let Ok(msg) = rx.recv() {
                if ctx.send(msg).is_err() {
                    break;
                }
            }
        })
        .ok();

    let control_conn = conn.clone();
    let writer = tokio::spawn(async move {
        while let Some(msg) = crx.recv().await {
            match msg {
                ClientMsg::Ping => {
                    let Some(line) = json_line(&ClientMsg::Ping) else { continue };
                    if send.write_all(line.as_bytes()).await.is_err() {
                        break;
                    }
                }
                ClientMsg::Leave => {
                    if let Some(line) = json_line(&ClientMsg::Leave) {
                        let _ = send.write_all(line.as_bytes()).await;
                    }
                    let _ = send.finish();
                    control_conn.close(0u8.into(), b"bye");
                    break;
                }
                _ => {}
            }
        }
        let _ = send.finish();
    });

    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => {
                let _ = events.send(Event::Closed("the host closed the room".into()));
                break;
            }
            Ok(_) => match serde_json::from_str::<ServerMsg>(&line) {
                Ok(ServerMsg::Sync {
                    host,
                    members,
                    state,
                }) => {
                    let _ = events.send(Event::Sync {
                        host,
                        members,
                        state,
                    });
                }
                Ok(ServerMsg::Members { host, members }) => {
                    let _ = events.send(Event::Members { host, members });
                }
                Ok(ServerMsg::Error { message }) => {
                    let _ = events.send(Event::Error(message));
                    break;
                }
                Ok(ServerMsg::Pong) | Ok(_) => {}
                Err(e) => {
                    let _ = events.send(Event::Error(format!("bad message from the host: {e}")));
                }
            },
            Err(e) => {
                let _ = events.send(Event::Closed(format!("room connection lost: {e}")));
                break;
            }
        }
    }

    writer.abort();
    endpoint.close().await;
    Ok(())
}

// ---------- helpers ----------

async fn write_msg(send: &mut SendStream, msg: &ServerMsg) -> Result<(), String> {
    let line = json_line(msg).ok_or_else(|| "cannot encode a room message".to_string())?;
    send.write_all(line.as_bytes())
        .await
        .map_err(|e| e.to_string())
}

fn json_line<T: Serialize>(msg: &T) -> Option<Arc<String>> {
    serde_json::to_string(msg).ok().map(|mut s| {
        s.push('\n');
        Arc::new(s)
    })
}

fn clean_name(name: &str) -> String {
    let n: String = name
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(24)
        .collect();
    if n.is_empty() {
        "listener".into()
    } else {
        n
    }
}

fn new_code() -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let mut x = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1)
        ^ (std::process::id() as u64) << 32
        ^ SEQ.fetch_add(1, Ordering::Relaxed).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let mut out = String::with_capacity(6);
    for _ in 0..6 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        out.push(CODE_ALPHABET[(x as usize) % CODE_ALPHABET.len()] as char);
    }
    out
}

fn identity_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("enigmatic-player").join("identity.key"))
}

/// The stable per-install secret key. Keeping it across runs means a player's
/// endpoint id - and therefore its reachability - survives restarts.
fn load_identity() -> Result<SecretKey, String> {
    let Some(path) = identity_path() else {
        return Ok(SecretKey::generate());
    };
    if let Ok(text) = std::fs::read_to_string(&path) {
        if let Some(key) = parse_identity(text.trim()) {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            }
            return Ok(key);
        }
    }
    let key = SecretKey::generate();
    save_identity(&path, &key)?;
    Ok(key)
}

fn parse_identity(text: &str) -> Option<SecretKey> {
    let bytes = unhex(text).ok()?;
    let bytes: [u8; 32] = bytes.try_into().ok()?;
    Some(SecretKey::from_bytes(&bytes))
}

fn save_identity(path: &Path, key: &SecretKey) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    let text = format!("{}\n", hex(&key.to_bytes()));
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| format!("cannot save the room identity: {e}"))?;
        file.write_all(text.as_bytes())
            .map_err(|e| format!("cannot save the room identity: {e}"))?;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, text).map_err(|e| format!("cannot save the room identity: {e}"))?;
    }
    Ok(())
}

// ---------- invite codec ----------

const INVITE_PREFIX: &str = "EROOM2:";
const LEGACY_PREFIX: &str = "EROOM1:";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Invite {
    /// A current invite: the host's iroh endpoint id plus the room code.
    P2p { endpoint: String, code: String },
    /// An invite from a version that used a plain TCP relay.
    Relay { server: String, code: String },
}

pub fn encode_invite(endpoint: &str, code: &str) -> String {
    format!("{INVITE_PREFIX}{}:{}", endpoint.trim(), code.to_uppercase())
}

pub fn decode_invite(text: &str) -> Result<Invite, String> {
    let text = text.trim();
    let lower = text.to_lowercase();
    if lower.starts_with("eroom2:") {
        let body = &text[INVITE_PREFIX.len()..];
        let (endpoint, code) = body
            .split_once(':')
            .ok_or_else(|| "room invite is incomplete".to_string())?;
        let endpoint = endpoint.trim().to_string();
        let code = code.trim().to_uppercase();
        if endpoint.is_empty() || code.len() < 4 {
            return Err("room invite is incomplete".into());
        }
        PublicKey::from_str(&endpoint)
            .map_err(|_| "room invite has an invalid host address".to_string())?;
        return Ok(Invite::P2p { endpoint, code });
    }
    if lower.starts_with("eroom1:") {
        let body = &text[LEGACY_PREFIX.len()..];
        let (server, code) = body
            .split_once(':')
            .ok_or_else(|| "room invite is incomplete".to_string())?;
        let server = String::from_utf8(unhex(server)?)
            .map_err(|_| "room invite has an invalid server address".to_string())?;
        let code = code.trim().to_uppercase();
        if server.is_empty() || code.len() < 4 {
            return Err("room invite is incomplete".into());
        }
        return Ok(Invite::Relay { server, code });
    }
    Err("not a room invite".into())
}

pub fn looks_like_invite(text: &str) -> bool {
    let t = text.trim().to_lowercase();
    t.starts_with("eroom2:") || t.starts_with("eroom1:")
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
    if !s.len().is_multiple_of(2) {
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

    fn sample_endpoint() -> String {
        SecretKey::generate().public().to_string()
    }

    #[test]
    fn invite_roundtrip() {
        let endpoint = sample_endpoint();
        let invite = encode_invite(&endpoint, "ABC234");
        assert!(looks_like_invite(&invite));
        assert_eq!(
            decode_invite(&invite).unwrap(),
            Invite::P2p {
                endpoint: endpoint.clone(),
                code: "ABC234".into()
            }
        );
        assert!(!looks_like_invite("ELIST1:abc"));
    }

    #[test]
    fn legacy_invite_is_recognised() {
        let server = "192.168.1.20:8080|203.0.113.7:56789";
        let invite = format!("EROOM1:{}:ABC234", hex(server.as_bytes()));
        assert!(looks_like_invite(&invite));
        assert_eq!(
            decode_invite(&invite).unwrap(),
            Invite::Relay {
                server: server.into(),
                code: "ABC234".into()
            }
        );
    }

    #[test]
    fn invite_rejects_junk() {
        assert!(decode_invite("hi").is_err());
        assert!(decode_invite("EROOM2:not-a-key:ABC").is_err());
        assert!(decode_invite("EROOM2::ABC").is_err());
        assert!(decode_invite("EROOM1:zz:ABC").is_err());
        assert!(decode_invite("EROOM1:61:ABC").is_err());
    }

    #[test]
    #[ignore = "needs internet: uses the public iroh relays"]
    fn p2p_room_roundtrip() {
        use std::time::{Duration, Instant};

        let root = std::env::temp_dir().join(format!("enigmatic-p2p-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let host_cfg = root.join("host");
        let guest_cfg = root.join("guest");
        std::fs::create_dir_all(&host_cfg).unwrap();
        std::fs::create_dir_all(&guest_cfg).unwrap();
        let old_cfg = std::env::var_os("XDG_CONFIG_HOME");

        std::env::set_var("XDG_CONFIG_HOME", &host_cfg);
        let (htx, hrx) = channel::<Event>();
        let state = RoomState {
            label: "smoke".into(),
            ..Default::default()
        };
        let host = create("host", state.clone(), htx).unwrap();

        let deadline = Instant::now() + Duration::from_secs(90);
        let (endpoint, code) = loop {
            assert!(Instant::now() < deadline, "host never became ready");
            match hrx.recv_timeout(Duration::from_millis(500)) {
                Ok(Event::Created {
                    server,
                    code,
                    members,
                    state,
                    ..
                }) => {
                    assert_eq!(members, vec!["host".to_string()]);
                    assert_eq!(state.label, "smoke");
                    break (server, code);
                }
                Ok(Event::Error(m)) => panic!("host failed: {m}"),
                Ok(_) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(e) => panic!("host event channel: {e}"),
            }
        };

        std::env::set_var("XDG_CONFIG_HOME", &guest_cfg);
        let (gtx, grx) = channel::<Event>();
        let guest = join(&endpoint, "guest", &code, gtx).unwrap();

        let deadline = Instant::now() + Duration::from_secs(90);
        let mut joined = false;
        while !joined && Instant::now() < deadline {
            match grx.recv_timeout(Duration::from_millis(500)) {
                Ok(Event::Joined {
                    code: got,
                    host: host_name,
                    ..
                }) => {
                    assert_eq!(got, code);
                    assert_eq!(host_name, "host");
                    joined = true;
                }
                Ok(Event::Error(m)) => panic!("guest failed: {m}"),
                Ok(_) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(e) => panic!("guest event channel: {e}"),
            }
        }
        assert!(joined, "guest never joined");

        let deadline = Instant::now() + Duration::from_secs(30);
        let mut saw_guest = false;
        while !saw_guest && Instant::now() < deadline {
            match hrx.recv_timeout(Duration::from_millis(500)) {
                Ok(Event::Members { members, .. }) => {
                    saw_guest = members.iter().any(|m| m == "guest");
                }
                Ok(_) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(e) => panic!("host event channel: {e}"),
            }
        }
        assert!(saw_guest, "host never saw the guest");

        let updated = RoomState {
            label: "updated".into(),
            playing: true,
            ..state
        };
        host.send_state(updated);

        let deadline = Instant::now() + Duration::from_secs(30);
        let mut synced = false;
        while !synced && Instant::now() < deadline {
            match grx.recv_timeout(Duration::from_millis(500)) {
                Ok(Event::Sync { state, .. }) => {
                    if state.label == "updated" && state.playing {
                        synced = true;
                    }
                }
                Ok(_) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(e) => panic!("guest event channel: {e}"),
            }
        }
        assert!(synced, "guest never received the host state");

        guest.leave();
        host.leave();
        std::thread::sleep(Duration::from_millis(300));
        match old_cfg {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
        let _ = std::fs::remove_dir_all(&root);
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
}
