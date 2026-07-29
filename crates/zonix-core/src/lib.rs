//! # zonix-core
//!
//! Shared types for the zOnix audio suite. This crate is intentionally free of
//! any PipeWire / OS bindings: it only describes *what* the system looks like
//! and *how* the frontend talks to the daemon. Both the daemon (server) and the
//! GUI/CLI (clients) depend on this crate so the wire contract can never drift.
//!
//! Module layout:
//! - [`model`]    — domain objects (virtual sinks, app streams, EQ, profiles...).
//! - [`protocol`] — the request/response/event envelope exchanged over IPC.
//! - [`ipc`]      — transport helpers (socket path, framing constants).

pub mod ipc;
pub mod model;
pub mod protocol;

/// Re-export the most commonly used items at the crate root.
pub use model::{
    AppStream, ChannelRole, EqBand, EqFilterKind, Equalizer, MicSettings, NoiseGate, Profile,
    SpatialSettings, SystemState, VirtualSink,
};
pub use protocol::{Event, Request, Response, RpcError, RpcMessage};

/// Semantic version of the IPC protocol. Clients and the daemon compare this on
/// connect; a mismatch in the MAJOR component means the client must refuse to
/// operate against an incompatible daemon.
pub const PROTOCOL_VERSION: (u16, u16) = (0, 1);
