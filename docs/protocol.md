# Listen-together protocol

Listening rooms are peer-to-peer. The host's player is the room: there is no
project-operated server to deploy or configure.

## Transport

- **iroh 1.4** over QUIC, with TLS encryption and the ALPN
  `enigmatic-player/room/2`.
- iroh uses a direct connection when NAT traversal succeeds and an encrypted
  public relay otherwise. Relay operators only forward ciphertext.
- After connecting, the guest opens one bidirectional QUIC stream. Both
  directions carry newline-delimited JSON, one message per line.

Older `EROOM1:` invites belong to the retired plain-TCP relay. They are
recognised so the player can explain the mismatch, but they cannot join a
current room.

## Invite

```text
EROOM2:<endpoint-id>:<room-code>
```

`endpoint-id` is the host's iroh endpoint public key. `room-code` is six
characters from a human-friendly alphabet. The invite is the only thing a
remote guest needs; no IP address, port mapping or account is involved.

On the same network, hosts also announce the room over UDP broadcast, so other
players can list and join it without typing an invite. Discovery packets carry
the host name, room code and endpoint id only.

## Connection

The guest sends exactly one join request:

```json
{"type":"join","code":"ABC234","name":"Alex"}
```

The host checks the room code, rejects the connection when the room is full,
and answers with the current room snapshot:

```json
{"type":"joined","code":"ABC234","token":"","host":"Sam","members":["Sam","Alex"],"state":{...}}
```

Rooms hold at most 16 members. `token` is reserved and currently empty.

## Messages from the host

```json
{"type":"sync","host":"Sam","members":["Sam","Alex"],"state":{...}}
{"type":"members","host":"Sam","members":["Sam","Alex"]}
{"type":"error","message":"that room is full"}
{"type":"pong"}
```

`sync` is broadcast whenever the host changes playback. `members` is broadcast
when someone joins or leaves.

## Messages from guests

```json
{"type":"leave"}
{"type":"ping"}
```

Only the host controls playback. Guest state messages are ignored, so guests
cannot change the track, position or play/pause state for everyone else.

## Room state

```json
{
  "label": "shared session",
  "queue": [
    {"title": "Song", "artist": "Band", "secs": 215.0, "url": null, "file": "song.mp3"}
  ],
  "index": 0,
  "pos": 12.5,
  "playing": true,
  "speed": 1.0
}
```

Only room metadata crosses the network: titles, artists, durations, playback
position and online URLs. Audio files and local file paths are never sent.
Each listener plays their own copy, matched from their local library or
streamed from the same online URL.

## Lifecycle

1. The host presses `H`; the player creates an iroh endpoint, publishes it and
   starts announcing the room on the LAN.
2. Guests join from the nearby-room list or by pasting an `EROOM2:` invite.
3. The host sends snapshots as playback changes. Guests apply every snapshot.
4. A guest leaves with `L` or by disconnecting. The host leaving closes the
   room for everyone.

The host identity key is stored in
`~/.config/enigmatic-player/identity.key`, so the endpoint id stays stable
across restarts. Rooms themselves live in memory only.
