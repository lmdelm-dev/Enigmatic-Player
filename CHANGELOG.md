# Changelog

All notable changes to Enigmatic Player are documented here.

## 1.8.0 - 2026-11-09

### Added
- Zero-setup listen-together: the host's app now embeds the room relay, so no
  separate server process is needed.
- Automatic router port opening (NAT-PMP first, then UPnP) with an honest
  reachability note in the room popup; mappings are removed again on leave.
- LAN room discovery over UDP broadcast: rooms appear in the join window and
  can be joined with the keyboard, no typing.
- Multi-address invites: one invite string carries LAN and public addresses.
- Live probe tooling for the router path, plus a wire-level discovery test.

### Changed
- Room codes are now 8 digits, easier to read out and type.
- Settings no longer has an "online" tab; hosting is fully embedded.
- Old `127.0.0.1:8080` config values are migrated to "no manual relay".

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
- NAT-PMP and UPnP port opening with public-IP discovery (`src/punch.rs`).
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
