# zOnix

**Native audio control, mixing & enhancement suite for Linux — built for Nobara.**

zOnix is an open-source alternative to SteelSeries Sonar / GG for the Linux
audio stack (**PipeWire + WirePlumber**). It gives gamers and creators:

- 🎚️ **Per-application mixer** — independent volume for Game, Chat, Media and
  System lanes via virtual PipeWire sinks.
- 🎛️ **Parametric equalizer** — 10-band / parametric EQ per lane, with presets
  (FPS, Music, Movie).
- 🎙️ **ClearCast mic enhancement** — RNNoise background-noise suppression, noise
  gate and compressor.
- 🎧 **Virtual surround** — HRTF-based 7.1 simulation on stereo headphones.
- 💾 **Profiles** — save/load audio setups and auto-switch by foreground game.

## Status

🚧 **Early development.** Phase 1 (architecture + scaffold) is in place: a Rust
workspace with the shared IPC contract, an always-on daemon (`zonixd`), and a
CLI client (`zonixctl`). The audio engine currently runs as an in-memory stub;
the real PipeWire backend lands in Phase 2. See
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for the full design and roadmap.

## Components

| Binary | Crate | Role |
|--------|-------|------|
| `zonixd` | `zonix-daemon` | Always-on audio engine + IPC server. |
| `zonixctl` | `zonix-cli` | Command-line client (scripting/testing). |
| — | `zonix-core` | Shared domain model + IPC protocol. |
| zOnix (GUI) | `gui/` | Tauri + React desktop app (Phase 4). |

## Quick start (Phase 1 scaffold)

```bash
cargo build
RUST_LOG=info ./target/debug/zonixd &
./target/debug/zonixctl state
./target/debug/zonixctl volume game 0.5
```

Run it as a background service with the provided `systemd --user` unit — see
[`systemd/zonix-daemon.service`](systemd/zonix-daemon.service).

## License

GPL-3.0-or-later.
