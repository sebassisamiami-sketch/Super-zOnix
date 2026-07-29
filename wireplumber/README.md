# PipeWire / WirePlumber integration

This directory holds the PipeWire config fragments that create zOnix's virtual
sinks and wire them into the graph.

## Shipped (Phase 2)

| File | Purpose |
|------|---------|
| `config/pipewire.conf.d/50-zonix-loopback.conf` | Creates the four lane sinks `zonix.sink.{game,chat,media,system}` using `module-loopback`. Each is a targetable `Audio/Sink` whose audio is mixed into the current default output device. Install it with `scripts/install-config.sh`. |

This is the **audible** creation path: apps route into these sinks, and the
`zonixd` daemon discovers them by name to control volume/mute and to move
application streams between lanes over the native PipeWire API.

Validated against PipeWire 1.2.7: after installing the drop-in, all four sinks
appear in `pw-cli ls Node` / `wpctl status`, and the daemon binds and controls
them (see `docs/ARCHITECTURE.md`, "Phase 2").

## Planned

| File | Purpose |
|------|---------|
| `config/pipewire.conf.d/51-zonix-mic.conf` | Microphone capture chain feeding the ClearCast enhancement (Phase 3). |
| `scripts/*.lua` | Optional WirePlumber Lua policy for automatic per-app lane assignment (Phase 3+). |
