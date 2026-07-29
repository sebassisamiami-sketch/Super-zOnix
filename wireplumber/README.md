# WirePlumber integration (Phase 2)

This directory holds the WirePlumber Lua scripts and PipeWire config fragments
that create zOnix's virtual sinks and wire them into the graph.

Planned contents:

| File | Purpose |
|------|---------|
| `scripts/51-zonix-virtual-sinks.lua` | Creates the `zonix.sink.{game,chat,media,system}` null sinks via `node.factory` (`support.null-audio-sink`). |
| `config/zonix.conf.d/10-zonix.conf` | Drop-in loaded from `~/.config/pipewire/pipewire.conf.d/` that links the mixed output to the default physical sink. |
| `scripts/52-zonix-mic-chain.lua` | Sets up the microphone capture chain node used by the ClearCast enhancement. |

The daemon (`zonixd`) can either (a) install/refresh these scripts and let
WirePlumber own the nodes, or (b) create the nodes directly via the PipeWire API
(`pipewire-rs`). Phase 2 implements the direct-API path in
`crates/zonix-daemon/src/engine/pipewire.rs`; these Lua scripts are the
declarative fallback for users who prefer config-file management.

> Nothing here is loaded yet — Phase 1 ships only the scaffold and the
> in-memory stub engine.
