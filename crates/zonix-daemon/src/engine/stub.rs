//! In-memory stub engine used until the PipeWire backend lands in Phase 2.
//!
//! It implements the full [`AudioEngine`] contract so the daemon, IPC layer and
//! GUI can be developed and tested end-to-end without any audio hardware or
//! PipeWire session. Every call is logged at debug level.

use tracing::debug;
use zonix_core::model::{ChannelRole, Equalizer};

use super::{AudioEngine, EngineResult};

/// A no-op engine that only logs the actions it receives.
#[derive(Debug, Default)]
pub struct StubEngine;

impl StubEngine {
    pub fn new() -> Self {
        Self
    }
}

impl AudioEngine for StubEngine {
    fn initialize_graph(&self) -> EngineResult<()> {
        for role in ChannelRole::PLAYBACK {
            debug!(sink = %role.slug(), "stub: would create virtual sink");
        }
        debug!("stub: would create microphone capture chain");
        Ok(())
    }

    fn set_sink_volume(&self, role: ChannelRole, volume: f32) -> EngineResult<()> {
        debug!(role = %role.slug(), volume, "stub: set sink volume");
        Ok(())
    }

    fn set_sink_mute(&self, role: ChannelRole, muted: bool) -> EngineResult<()> {
        debug!(role = %role.slug(), muted, "stub: set sink mute");
        Ok(())
    }

    fn route_stream(&self, node_id: u32, role: ChannelRole) -> EngineResult<()> {
        debug!(node_id, role = %role.slug(), "stub: route stream");
        Ok(())
    }

    fn apply_equalizer(&self, role: ChannelRole, eq: &Equalizer) -> EngineResult<()> {
        debug!(role = %role.slug(), enabled = eq.enabled, bands = eq.bands.len(), "stub: apply EQ");
        Ok(())
    }

    fn shutdown(&self) -> EngineResult<()> {
        debug!("stub: shutdown");
        Ok(())
    }
}
