use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::together::{ClientMsg, RoomState, ServerMsg};

const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const MAX_MEMBERS: usize = 16;

struct Member {
    name: String,
    out: Mutex<TcpStream>,
}

struct Room {
    host_name: String,
    host_token: String,
    state: RoomState,
    members: HashMap<String, Arc<Member>>,
}

type Rooms = Arc<Mutex<HashMap<String, Room>>>;

#[derive(Clone)]
struct Session {
    code: String,
    token: String,
    host: bool,
}

/// Runs the sync server until the process is stopped. This only relays room
/// state; it never receives or stores audio files.
pub fn run(addr: &str) -> std::io::Result<()> {
    let listener = TcpListener::bind(addr)?;
    println!(
        "enigmatic player sync server listening on {}",
        listener.local_addr()?
    );
    println!("rooms are in memory only; stop the server to remove them");
    serve(listener)
}

pub fn serve(listener: TcpListener) -> std::io::Result<()> {
    let rooms: Rooms = Arc::new(Mutex::new(HashMap::new()));
    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                let rooms = rooms.clone();
                thread::spawn(move || handle(s, rooms));
            }
            Err(e) => eprintln!("sync server: accept failed: {e}"),
        }
    }
    Ok(())
}

fn handle(stream: TcpStream, rooms: Rooms) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(45)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_nodelay(true);
    let Ok(read_half) = stream.try_clone() else {
        println!("sync server: cannot clone connection for reading");
        return;
    };
    let mut reader = BufReader::new(read_half);
    let mut session: Option<Session> = None;
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                if std::env::var("ENIGMATIC_DEBUG").is_ok() {
                    println!("sync server: got line: {line}");
                }
                let parsed = serde_json::from_str::<ClientMsg>(&line);
                match parsed {
                    Ok(ClientMsg::Create { name, state }) => {
                        if session.is_some() {
                            send_error(&stream, "you are already in a room");
                            continue;
                        }
                        let Ok(out) = stream.try_clone() else { break };
                        let (code, token) = {
                            let mut map = rooms.lock().unwrap();
                            let code = new_code(&map);
                            let token = new_id();
                            let member = Arc::new(Member {
                                name: clean_name(&name),
                                out: Mutex::new(out),
                            });
                            let mut members = HashMap::new();
                            members.insert(token.clone(), member.clone());
                            map.insert(
                                code.clone(),
                                Room {
                                    host_name: clean_name(&name),
                                    host_token: token.clone(),
                                    state: state.clone(),
                                    members,
                                },
                            );
                            (code, token)
                        };
                        session = Some(Session {
                            code: code.clone(),
                            token,
                            host: true,
                        });
                        let msg = ServerMsg::Created {
                            code,
                            token: String::new(),
                            host: clean_name(&name),
                            members: vec![clean_name(&name)],
                            state,
                        };
                        send(&stream, &msg);
                    }
                    Ok(ClientMsg::Join { code, name }) => {
                        if session.is_some() {
                            send_error(&stream, "you are already in a room");
                            continue;
                        }
                        let code = code.trim().to_uppercase();
                        let name = clean_name(&name);
                        let joined = {
                            let mut map = rooms.lock().unwrap();
                            match map.get_mut(&code) {
                                None => Err("no room with that invite code"),
                                Some(room) if room.members.len() >= MAX_MEMBERS => {
                                    Err("that room is full")
                                }
                                Some(room) => {
                                    let token = new_id();
                                    let member = Arc::new(Member {
                                        name: name.clone(),
                                        out: Mutex::new(
                                            stream
                                                .try_clone()
                                                .unwrap_or_else(|_| stream.try_clone().unwrap()),
                                        ),
                                    });
                                    room.members.insert(token.clone(), member);
                                    Ok((
                                        token,
                                        room.host_name.clone(),
                                        room.state.clone(),
                                        member_names(room),
                                    ))
                                }
                            }
                        };
                        match joined {
                            Err(e) => send_error(&stream, e),
                            Ok((token, host, state, members)) => {
                                let msg = ServerMsg::Joined {
                                    code: code.clone(),
                                    token: token.clone(),
                                    host: host.clone(),
                                    members: members.clone(),
                                    state,
                                };
                                send(&stream, &msg);
                                session = Some(Session {
                                    code: code.clone(),
                                    token,
                                    host: false,
                                });
                                broadcast_members(&rooms, &code);
                            }
                        }
                    }
                    Ok(ClientMsg::State { state }) => {
                        let Some(s) = &session else {
                            send_error(&stream, "join a room first");
                            continue;
                        };
                        if !s.host {
                            send_error(&stream, "only the host controls playback");
                            continue;
                        }
                        broadcast_sync(&rooms, &s.code, &s.token, state);
                    }
                    Ok(ClientMsg::Ping) => {
                        send(&stream, &ServerMsg::Pong);
                    }
                    Ok(ClientMsg::Leave) => break,
                    Err(e) => {
                        println!("sync server: bad message: {e} - {line}");
                        send_error(&stream, "invalid message");
                    }
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(e) => {
                println!("sync server: connection error: {e}");
                break;
            }
        }
    }

    cleanup(&rooms, session);
}

fn cleanup(rooms: &Rooms, session: Option<Session>) {
    let Some(s) = session else { return };
    let mut map = rooms.lock().unwrap();
    let Some(room) = map.get_mut(&s.code) else { return };
    room.members.remove(&s.token);
    if s.host && room.host_token == s.token {
        let members: Vec<Arc<Member>> = room.members.values().cloned().collect();
        map.remove(&s.code);
        for m in members {
            send_member(&m, &ServerMsg::Error { message: "the host ended the session".into() });
        }
    } else {
        let host = room.host_name.clone();
        let members = member_names(room);
        drop(map);
        broadcast_members_named(rooms, &s.code, host, members);
    }
}

fn new_code(map: &HashMap<String, Room>) -> String {
    loop {
        let code: String = new_id()
            .bytes()
            .take(6)
            .map(|b| CODE_ALPHABET[(b as usize) % CODE_ALPHABET.len()] as char)
            .collect();
        if !map.contains_key(&code) {
            return code;
        }
    }
}

fn new_id() -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let mut x = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1)
        ^ (std::process::id() as u64) << 32
        ^ SEQ.fetch_add(1, Ordering::Relaxed).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let mut out = String::new();
    for _ in 0..16 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        out.push(CODE_ALPHABET[(x as usize) % CODE_ALPHABET.len()] as char);
    }
    out
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

fn member_names(room: &Room) -> Vec<String> {
    let mut names: Vec<String> = room.members.values().map(|m| m.name.clone()).collect();
    names.sort();
    names
}

fn send(stream: &TcpStream, msg: &ServerMsg) {
    let Ok(json) = serde_json::to_string(msg) else { return };
    let mut w = stream;
    if std::env::var("ENIGMATIC_DEBUG").is_ok() {
        println!("sync server: send {:?} ({} bytes)", json.split_at(1.max(json.len().min(30))).0, json.len());
    }
    let _ = w.write_all(json.as_bytes());
    let _ = w.write_all(b"\n");
    let _ = w.flush();
}

fn send_member(member: &Member, msg: &ServerMsg) {
    let Ok(json) = serde_json::to_string(msg) else { return };
    if let Ok(mut s) = member.out.lock() {
        if std::env::var("ENIGMATIC_DEBUG").is_ok() {
            println!("sync server: send_member {:?} ({} bytes)", json.split_at(json.len().min(30)).0, json.len());
        }
        let _ = s.write_all(json.as_bytes());
        let _ = s.write_all(b"\n");
        let _ = s.flush();
    }
}

fn send_error(stream: &TcpStream, message: &str) {
    send(
        stream,
        &ServerMsg::Error {
            message: message.to_string(),
        },
    );
}

fn broadcast_members(rooms: &Rooms, code: &str) {
    let infos = {
        let map = rooms.lock().unwrap();
        let Some(room) = map.get(code) else { return };
        Some((room.host_name.clone(), member_names(room), room.members.values().cloned().collect::<Vec<_>>()))
    };
    let Some((host, members, targets)) = infos else { return };
    let msg = ServerMsg::Members { host, members };
    for m in targets {
        send_member(&m, &msg);
    }
}

fn broadcast_members_named(rooms: &Rooms, code: &str, host: String, names: Vec<String>) {
    let targets = {
        let map = rooms.lock().unwrap();
        match map.get(code) {
            Some(room) => room.members.values().cloned().collect::<Vec<_>>(),
            None => return,
        }
    };
    let msg = ServerMsg::Members {
        host,
        members: names,
    };
    for m in targets {
        send_member(&m, &msg);
    }
}

fn broadcast_sync(rooms: &Rooms, code: &str, sender_token: &str, state: RoomState) {
    let (host, members, targets) = {
        let mut map = rooms.lock().unwrap();
        let Some(room) = map.get_mut(code) else { return };
        room.state = state.clone();
        (
            room.host_name.clone(),
            member_names(room),
            room.members
                .iter()
                .filter(|(token, _)| token.as_str() != sender_token)
                .map(|(_, m)| m.clone())
                .collect::<Vec<_>>(),
        )
    };
    let msg = ServerMsg::Sync {
        host,
        members,
        state,
    };
    for m in targets {
        send_member(&m, &msg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::share::SharedTrack;
    use std::net::SocketAddr;

    fn connect(addr: SocketAddr) -> (TcpStream, BufReader<TcpStream>) {
        let s = TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let r = BufReader::new(s.try_clone().unwrap());
        (s, r)
    }

    fn write(s: &TcpStream, msg: &ClientMsg) {
        let mut w = s;
        let mut line = serde_json::to_string(msg).unwrap();
        line.push('\n');
        w.write_all(line.as_bytes()).unwrap();
        w.flush().unwrap();
    }

    fn read_until(
        r: &mut BufReader<TcpStream>,
        kind: &str,
    ) -> serde_json::Value {
        let mut line = String::new();
        loop {
            line.clear();
            let n = r.read_line(&mut line);
            if n.is_err() || line.trim().is_empty() {
                panic!(
                    "read_until({kind}): bad read {n:?} or empty line from the server: {:?}",
                    line
                );
            }
            let v: serde_json::Value =
                serde_json::from_str(&line).unwrap_or_else(|e| panic!("read_until({kind}): json error {e} - line: {line:?}"));
            if v["type"] == kind {
                return v;
            }
        }
    }

    #[test]
    fn create_join_and_sync() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || serve(listener).unwrap());

        let state = RoomState {
            label: "test mix".into(),
            queue: vec![SharedTrack {
                title: "One".into(),
                artist: "Band".into(),
                secs: 10.0,
                url: None,
                file: Some("one.mp3".into()),
            }],
            index: 0,
            pos: 2.0,
            playing: true,
            speed: 1.0,
        };

        let (host, mut host_r) = connect(addr);
        write(
            &host,
            &ClientMsg::Create {
                name: "Host".into(),
                state: state.clone(),
            },
        );
        let created = read_until(&mut host_r, "created");
        let code = created["code"].as_str().unwrap().to_string();
        assert_eq!(code.len(), 6);

        let (guest, mut guest_r) = connect(addr);
        write(
            &guest,
            &ClientMsg::Join {
                code: code.clone(),
                name: "Guest".into(),
            },
        );
        let joined = read_until(&mut guest_r, "joined");
        assert_eq!(joined["state"]["label"], "test mix");
        assert_eq!(joined["host"], "Host");

        let mut next = state;
        next.pos = 4.5;
        next.index = 0;
        write(&host, &ClientMsg::State { state: next });
        let sync = read_until(&mut guest_r, "sync");
        assert_eq!(sync["state"]["pos"], 4.5);
    }
}
