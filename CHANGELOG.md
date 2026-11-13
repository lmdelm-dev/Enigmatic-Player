# Changelog

All notable changes to Enigmatic Player are documented here.

## 1.9.0 - 2026-11-13

### Added
- Synced lyrics popup (`y`) with an LRC parser, plain-lyrics fallback and
  automatic line following.
- Lyrics providers: sidecar `.lrc` / `.txt` files, embedded audio tags, a disk
  cache, and online lookup through [LRCLIB](https://lrclib.net).
- Online lyrics work for streamed YouTube and SoundCloud tracks, not only
  local files. `ENIGMATIC_NO_LYRICS_ONLINE=1` disables all network lookups.
- Listen-together rooms over [iroh](https://iroh.computer): encrypted QUIC
  peer-to-peer connections, LAN discovery, `EROOM2:` invites and relay
  fallback without port forwarding.

### Changed
- The repository now ships only the Rust terminal player. The browser edition,
  static web assets, Python backend and web hosting configuration were removed.
- The Windows installer and its build scripts were removed. Build from source
  with `cargo build --release`.
- README rebuilt around the lightweight, ad-free YouTube and SoundCloud
  experience, with screenshots, feature comparison and full key reference.

### Fixed
- Lyrics no longer require the track to be downloaded before lookup.
- Online playback starts directly through `yt-dlp` + `ffmpeg` without opening
  a video page or waiting through pre-roll ads.

## 1.8.1 - 2026-11-10

### Fixed
- Hosting a room failed when the embedded relay listened on the wildcard
  address. The host client now dials loopback.
- The About page shows the real build version instead of a hard-coded one.

## 1.8.0 - 2026-11-09

### Added
- Zero-setup listen-together with an embedded room relay.
- LAN room discovery over UDP broadcast and multi-address invites.

### Changed
- Room codes are easier to read out and type.
- Settings no longer has an "online" tab; hosting is fully embedded.

## 1.4.0 - 2026-10-20

### Added
- Protocol documentation and a changelog.
- End-to-end tests driving the real client threads against the real server.

## 1.2.0 - 2026-10-02

### Added
- README rewrite for the zero-setup room flow, with firewall caveats.

## 1.1.0 - 2026-09-17

### Added
- LAN room picker in the join popup.
- Reachability note in the room popup (LAN + internet, or LAN only).

## 1.0.0 - 2026-09-02

### Added
- NAT-PMP and UPnP port opening with public-IP discovery.
- Unit tests for the wire formats and UPnP descriptor parsing.

## 0.9.0 - 2026-08-18

### Added
- LAN discovery beacon (`src/beacon.rs`): hosts announce rooms, listeners
  collect them with an age-out and a two-hit confidence rule.

## 0.6.0 - 2026-05-19

### Added
- Listen-together rooms: relay server, room client, invite codec.
- Share and join popups, room popup and live badge in the title bar.

## 0.3.0 - 2026-04-14

### Added
- `ELIST1:` playlist share codes with metadata-only contents.
- Import flow that matches shared tracks against the local library.

## 0.2.0 - 2026-04-02

### Added
- Terminal lifecycle, entrypoint and release profile.
- First end-to-end run of the player UI with native audio.

## 0.1.0 - 2026-01-12

### Added
- Project skeleton: Cargo manifest, ignore rules and README.
