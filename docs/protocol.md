# Listen-together protocol

The room protocol is intentionally small: newline-delimited JSON over plain
TCP. There is no TLS and no authentication beyond the room code and the
per-connection token issued by the relay.

## Connection

A client opens a TCP connection to one of the addresses in an invite and sends
exactly one of:

```json
{"type":"create","name":"Sam","state":{...}}
{"type":"join","code":"12345678","name":"Alex"}
```

`state` is the host's initial room state (label, queue, index, position,
playing, speed).

## Messages from the relay

```json
{"type":"created","code":"12345678","token":"...","host":"Sam","members":["Sam"],"state":{...}}
{"type":"joined","code":"12345678","token":"...","host":"Sam","members":["Sam","Alex"],"state":{...}}
{"type":"sync","host":"Sam","members":["Sam","Alex"],"state":{...}}
{"type":"members","host":"Sam","members":["Sam","Alex"]}
{"type":"error","message":"only the host controls playback"}
{"type":"pong"}
```

## Messages from clients

```json
{"type":"state","state":{...}}
{"type":"leave"}
{"type":"ping"}
```

Only the host may send `state`; guests receive `error` if they try. The relay
broadcasts `sync` to everyone else when the host's state changes, and
`members` when people join or leave.

## Invites

An invite is `EROOM1:` + hex(server) + `:` + code. The server field may hold
several `|`-separated addresses (LAN first, then the router-mapped public
address), which joiners try in order.

## Room state

```json
{
  "label": "shared session",
  "queue": [{"title":"Song","artist":"Band","secs":215.0,"url":null,"file":"song.mp3"}],
  "index": 0,
  "pos": 12.5,
  "playing": true,
  "speed": 1.0
}
```

`file` is a file-name hint only; local paths never leave the host's machine.
Listeners resolve each entry against their own library by tags, then file name,
or stream the original URL when present.
