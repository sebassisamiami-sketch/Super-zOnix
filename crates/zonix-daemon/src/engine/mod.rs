//! The audio engine abstraction.
//!
//! The rest of the daemon talks to the audio graph only through the
//! [`AudioEngine`] trait. In Phase 1 the only implementation is
//! [`stub::StubEngine`], which keeps state in memory and logs what it *would*
//! do. In Phase 2 we add `pipewire::PipeWireEngine`, which drives the real
//! PipeWire graph via `libpipewire-0.3`. Because the daemon only depends on the
//! trait, swapping implementations does not touch the IPC layer.

pub mod stub;

#[cfg(feature = "pipewire-backend")]
pub mod pipewire;

use zonix_core::model::{ChannelRole, Equalizer};

/// Errors the engine may return. Kept small; the IPC layer maps these onto
/// [`zonix_core::protocol::RpcError`].
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("failed to talk to PipeWire: {0}")]
    PipeWire(String),
    #[error("object not found: {0}")]
    NotFound(String),
    #[error("engine internal error: {0}")]
    Internal(String),
}

pub type EngineResult<T> = Result<T, EngineError>;

/// Commands the daemon can issue against the live audio graph.
///
/// Implementations must be `Send + Sync` because the engine handle is shared
/// across async tasks. Real PipeWire work happens on a dedicated thread; this
/// trait is the thread-safe façade in front of it.
pub trait AudioEngine: Send + Sync {
    /// Create (or ensure the existence of) the four virtual playback sinks and
    /// the microphone capture chain. Called once at startup.
    fn initialize_graph(&self) -> EngineResult<()>;

    /// Apply a lane's linear volume (0.0..=1.0) to its virtual sink.
    fn set_sink_volume(&self, role: ChannelRole, volume: f32) -> EngineResult<()>;

    /// Mute/unmute a lane's virtual sink.
    fn set_sink_mute(&self, role: ChannelRole, muted: bool) -> EngineResult<()>;

    /// Route an application stream (by PipeWire node id) into a lane's sink.
    fn route_stream(&self, node_id: u32, role: ChannelRole) -> EngineResult<()>;

    /// Load the given EQ curve into the lane's DSP filter chain.
    fn apply_equalizer(&self, role: ChannelRole, eq: &Equalizer) -> EngineResult<()>;

    /// Tear down created nodes and disconnect. Called on shutdown.
    fn shutdown(&self) -> EngineResult<()>;
}
