# zOnix — Architecture (Phase 1)

zOnix is a native audio control, mixing and enhancement suite for Linux
(targeting **Nobara**, Fedora-based, gaming-focused). It plays the role
SteelSeries Sonar / GG plays on Windows: per-application audio lanes,
parametric EQ, microphone enhancement, virtual surround, and auto-switching
profiles — but built on the Linux audio stack (**PipeWire + WirePlumber**).

---

## 1. Design principles

1. **The daemon owns the audio graph; the GUI is a thin client.**
   All audio logic lives in a long-running background process (`zonixd`). The
   GUI can be closed and reopened without disturbing audio, and multiple
   clients (GUI, CLI, scripts) can control it at once.
2. **One typed contract, shared by both ends.** The `zonix-core` crate defines
   every message and domain type once, in Rust, and is compiled into both the
   daemon and the Tauri client — no drift between frontend and backend.
3. **The OS/audio layer is behind a trait.** The daemon only depends on the
   `AudioEngine` trait, so the in-memory stub (Phase 1) and the real PipeWire
   engine (Phase 2) are interchangeable and independently testable.
4. **Low latency, low idle cost.** Rust backend, real-time-friendly PipeWire
   filter chains, and a native (non-Electron) GUI.

---

## 2. Technology stack

| Layer | Choice | Why |
|-------|--------|-----|
| Backend core | **Rust** | Memory safety without a GC, mature PipeWire bindings (`pipewire`/`libspa`), great async story (`tokio`), single static binary. |
| Audio engine | **PipeWire + WirePlumber native API** | The default on Nobara. We create null-sink virtual devices and manage links directly. |
| DSP | **PipeWire `filter-chain` + LV2 + RNNoise** | Parametric EQ via builtin biquads, noise suppression via RNNoise, all as native graph nodes (EasyEffects-style). |
| Frontend | **Tauri 2 + React + TypeScript** | Modern, fluid UI with a tiny native footprint (system webview, not Chromium). Rust side reuses `zonix-core`. |
| Frontend ↔ Backend | **Unix domain socket, newline-delimited JSON** | Per-user, off-network, language-agnostic, trivial to test with `socat`/Python. |
| Autostart | **`systemd --user` service** | Starts `zonixd` after the PipeWire user services, per the requirement. |
| Packaging | **`.rpm` (primary) / Flatpak (fallback)** | Native fit for Nobara; Flatpak for portability. |

---

## 3. Process & component topology

```
                         ┌────────────────────────────────────────────┐
                         │                 USER SPACE                   │
                         │                                              │
  ┌───────────────┐      │   ┌──────────────────────────────────────┐  │
  │  zOnix GUI     │      │   │            zonixd (daemon)           │  │
  │ (Tauri+React)  │      │   │                                      │  │
  │                │      │   │  ┌────────────┐   ┌───────────────┐  │  │
  │  React webview │      │   │  │ IPC server │   │ SharedState   │  │  │
  │      ▲  │      │      │   │  │ (unix sock)│──▶│ (SystemState  │  │  │
  │      │  ▼      │      │   │  │            │   │  + events)    │  │  │
  │  Tauri Rust    │◀─────┼──▶│  └─────┬──────┘   └──────┬────────┘  │  │
  │  (thin client) │ sock │   │        │                 │           │  │
  └───────────────┘      │   │        ▼                 ▼           │  │
        ▲                 │   │  ┌──────────────────────────────┐   │  │
        │ commands/events │   │  │      AudioEngine (trait)      │   │  │
  ┌─────┴─────┐           │   │  │  Phase1: StubEngine           │   │  │
  │ zonixctl  │───────────┼──▶│  │  Phase2: PipeWireEngine ──────┼───┼──┐
  │  (CLI)    │   sock     │   │  └──────────────────────────────┘   │  │  │
  └───────────┘           │   └──────────────────────────────────────┘  │  │
                          │                                              │  │
                          └──────────────────────────────────────────────┘  │
                                                                              │ libpipewire-0.3
                          ┌──────────────────────────────────────────────┐  │ (Phase 2)
                          │                 PipeWire graph                │◀─┘
                          │                                              │
                          │  app streams        virtual sinks   physical │
                          │  ┌────────┐         ┌───────────┐   ┌──────┐ │
                          │  │ Game   │────────▶│ zonix.sink │   │ Head-│ │
                          │  │ Chat   │────────▶│  .game     │──▶│ phones│ │
                          │  │ Browser│────────▶│  .chat ... │   │ (7.1  │ │
                          │  └────────┘         └───────────┘   │  HRTF)│ │
                          │                      + EQ / DSP      └──────┘ │
                          │                      + mic chain              │
                          └──────────────────────────────────────────────┘
```

- The **GUI** and **CLI** are peers: both are clients of the daemon over the
  same socket. Closing the GUI never stops audio processing.
- The **daemon** is the only process that touches PipeWire.

---

## 4. Request/response/event flow

Concrete example: the user drags the **Game** fader to 50%.

```
React <Fader>              Tauri Rust            zonixd IPC          AudioEngine
    │  onChange(0.5)           │                     │                   │
    │─ invoke("set_volume") ──▶│                     │                   │
    │                          │  RpcMessage::Request │                   │
    │                          │  {id, SetSinkVolume  │                   │
    │                          │   game, 0.5}         │                   │
    │                          │──── unix socket ────▶│                   │
    │                          │                      │ engine.set_sink_  │
    │                          │                      │   volume(game,.5)─▶│  (PipeWire:
    │                          │                      │                   │   set node
    │                          │                      │◀── Ok ────────────│   "volume" prop)
    │                          │                      │ state.set_sink_   │
    │                          │                      │   volume(...)     │
    │                          │◀── Response{id, Ok} ─│                   │
    │◀── resolve() ────────────│                      │                   │
    │                          │   Event::StateChanged│  (broadcast to    │
    │◀── emit "state" ─────────│◀─────────────────────│   every client)   │
```

Two things happen on every mutating command:

1. The daemon issues the real graph change through the engine, **then** updates
   its authoritative `SystemState`.
2. It broadcasts a `StateChanged` (or a finer-grained) `Event` to *all*
   connected clients, so a change made from the CLI is instantly reflected in
   an open GUI, and vice-versa.

Unsolicited events (no request needed): `StreamAdded`/`StreamRemoved` when apps
start/stop playing, `LevelMeter` for VU meters, `ProfileAutoActivated` when a
foreground game matches a profile, `PipeWireStatus` on connect/disconnect.

---

## 5. The IPC contract (`zonix-core`)

Transport: `$XDG_RUNTIME_DIR/zonix/daemon.sock`, one JSON object per line.

```jsonc
// client → daemon
{"type":"request","id":12,"request":{"op":"set_sink_volume","role":"game","volume":0.5}}
// daemon → client
{"type":"response","id":12,"response":{"result":"ok"}}
// daemon → client (unsolicited)
{"type":"event","event":{"event":"state_changed","state":{ ... }}}
```

- `Request` — every command (get state, set volume/mute, route stream, set EQ,
  mic settings, spatial, profile CRUD, auto-switch).
- `Response` — `Hello`, `State`, `Profiles`, `Ok`, or a structured `Error`
  (`RpcError { code, message }`).
- `Event` — the push notifications listed above.
- `PROTOCOL_VERSION` is exchanged in the `Hello` handshake; a MAJOR mismatch
  makes the client refuse to operate.

See `crates/zonix-core/src/protocol.rs` and `model.rs` for the full definitions.

---

## 6. Domain model (`zonix-core::model`)

- `ChannelRole` — the lanes: `Game`, `Chat`, `Media`, `System` (playback) and
  `Microphone` (capture). Each playback role maps to one PipeWire null sink
  named `zonix.sink.<slug>`.
- `VirtualSink` — a lane: node id/name, linear volume, mute, and its
  `Equalizer`.
- `AppStream` — a real application stream discovered on the graph (node id,
  app name, PID, current lane, per-app volume).
- `Equalizer` / `EqBand` — parametric EQ; the default is a 10-band ISO graphic
  layout, but bands are fully parametric so presets (FPS, Music, Movie) can
  reshape them.
- `MicSettings` / `NoiseGate` — the ClearCast chain: RNNoise suppression,
  noise gate, compressor, input gain.
- `SpatialSettings` — HRTF virtual surround (profile + speaker layout).
- `Profile` — a named snapshot of all of the above plus `match_rules` used for
  auto-switching by foreground process.
- `SystemState` — the full runtime snapshot the daemon serves and broadcasts.

---

## 7. Repository layout

```
Super-zOnix/
├── Cargo.toml                     # Rust workspace
├── crates/
│   ├── zonix-core/                # shared model + IPC contract (no OS deps)
│   │   └── src/{lib,model,protocol,ipc}.rs
│   ├── zonix-daemon/              # zonixd: engine + IPC server
│   │   └── src/{main,ipc,state}.rs
│   │       └── engine/{mod,stub}.rs   # AudioEngine trait + stub (Phase 1)
│   │                                  # + pipewire.rs (Phase 2)
│   └── zonix-cli/                 # zonixctl: CLI client
│       └── src/main.rs
├── gui/                           # Tauri + React frontend (Phase 4)
├── wireplumber/                   # Lua scripts / PipeWire config (Phase 2)
├── systemd/
│   └── zonix-daemon.service       # systemd --user unit
├── packaging/                     # rpm / flatpak (Phase 5)
└── docs/
    └── ARCHITECTURE.md            # this file
```

---

## 8. Roadmap by phase

| Phase | Scope | Status |
|-------|-------|--------|
| **1** | Architecture, workspace scaffold, IPC contract, stub engine, daemon+CLI that build & run. | ✅ done |
| **2** | `PipeWireEngine`: create the 4 virtual sinks (loopback config) + native discovery, volume/mute, stream routing. | ✅ done |
| 3 | DSP: parametric EQ + RNNoise noise suppression + gate/compressor on the lanes/mic. | next |
| 4 | Tauri + React GUI: per-app faders, EQ graph, mic panel, profile bar. | |
| 5 | Nobara deployment: `.rpm`/Flatpak, exact build & install steps. | |

### Phase 2 — PipeWire backend (implemented & validated)

The real backend is compiled with `--features pipewire-backend` (requires
`pipewire-devel` at build time). It was validated end-to-end against a live
**PipeWire 1.2.7** session:

1. The loopback config drop-in created the four lane sinks
   (`zonix.sink.{game,chat,media,system}`) — confirmed via `pw-cli ls Node`.
2. `zonixd` connected, discovered all four sinks and bound a proxy to each
   (`lane sink ready role=game node_id=40 …`).
3. `zonixctl volume game 0.40` set the sink's real `channelVolumes` to
   `[0.40, 0.40]` (confirmed via `pw-cli enum-params zonix.sink.game Props`).
4. `zonixctl mute chat on` set the real node's `Props:mute` to `true`.

**How it works:** virtual sinks are created declaratively by
`wireplumber/config/pipewire.conf.d/50-zonix-loopback.conf` (each is a
`module-loopback` instance = a targetable `Audio/Sink` + a playback stream that
mixes into the default output). The Rust `PipeWireEngine` runs the PipeWire main
loop on a dedicated thread, watches the registry to discover the lane sinks and
application streams, drives per-lane volume/mute by sending a SPA `Props` pod via
`Node::set_param`, and routes app streams by writing `target.object` into the
`default` metadata (WirePlumber then re-links them).

**Not yet exercised:** live per-app stream routing was not tested in the
headless sandbox (no real application streams were present); the code path is in
place and uses the standard metadata mechanism. EQ (`apply_equalizer`) is a
no-op until the Phase 3 filter-chain lands.

---

## 9. Build & run (Phase 1 scaffold)

```bash
# Stub build — runs anywhere, no PipeWire needed (in-memory engine)
cargo build
RUST_LOG=info ./target/debug/zonixd &
./target/debug/zonixctl state
./target/debug/zonixctl volume game 0.5
./target/debug/zonixctl mute chat on
```

### Real PipeWire backend (Phase 2)

```bash
# One-time: install the build dependency on Nobara/Fedora
sudo dnf install pipewire-devel wireplumber clang

# Install the virtual-sink config and restart the user audio services
./scripts/install-config.sh

# Build with the real backend and run the daemon
cargo build --features pipewire-backend
RUST_LOG=info ./target/debug/zonixd &

# The four zOnix lanes now exist and are controllable:
wpctl status | grep -i zonix
./target/debug/zonixctl volume game 0.4
./target/debug/zonixctl mute chat on
```

> **Sandbox note:** the default (stub) build compiles anywhere. The
> `pipewire-backend` build links `libpipewire-0.3`, so it needs the
> `pipewire-devel` package at build time.
