//! Central, shared runtime state of the daemon.
//!
//! `SharedState` is a cheap-to-clone handle around the authoritative
//! [`SystemState`]. Everything that mutates state does so through here so we can
//! (a) serialize access with an async mutex and (b) broadcast a change event to
//! every connected client from a single place.

use std::sync::Arc;

use tokio::sync::{broadcast, Mutex};
use zonix_core::{
    model::{ChannelRole, Equalizer, MicSettings, SpatialSettings, SystemState},
    protocol::Event,
};

/// How many events we buffer per subscriber before lagging clients drop frames.
const EVENT_CHANNEL_CAPACITY: usize = 256;

/// Clonable handle to the daemon's shared state.
#[derive(Clone)]
pub struct SharedState {
    inner: Arc<Mutex<SystemState>>,
    events: broadcast::Sender<Event>,
}

impl SharedState {
    pub fn new(initial: SystemState) -> Self {
        let (events, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Self {
            inner: Arc::new(Mutex::new(initial)),
            events,
        }
    }

    /// Subscribe to the daemon-wide event stream.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    /// Take a full snapshot of the current state (for `GetState` / `Hello`).
    pub async fn snapshot(&self) -> SystemState {
        self.inner.lock().await.clone()
    }

    /// Broadcast an event to all subscribers. Errors (no receivers) are ignored.
    fn emit(&self, event: Event) {
        let _ = self.events.send(event);
    }

    /// Emit a fresh full-state snapshot as a `StateChanged` event.
    async fn emit_state_changed(&self) {
        let snap = self.inner.lock().await.clone();
        self.emit(Event::StateChanged {
            state: Box::new(snap),
        });
    }

    /// Set a lane's linear volume (clamped to 0.0..=1.0) and notify clients.
    pub async fn set_sink_volume(&self, role: ChannelRole, volume: f32) {
        {
            let mut st = self.inner.lock().await;
            if let Some(sink) = st.sinks.iter_mut().find(|s| s.role == role) {
                sink.volume = volume.clamp(0.0, 1.0);
            }
        }
        self.emit_state_changed().await;
    }

    /// Mute/unmute a lane and notify clients.
    pub async fn set_sink_mute(&self, role: ChannelRole, muted: bool) {
        {
            let mut st = self.inner.lock().await;
            if let Some(sink) = st.sinks.iter_mut().find(|s| s.role == role) {
                sink.muted = muted;
            }
        }
        self.emit_state_changed().await;
    }

    /// Replace a lane's equalizer and notify clients.
    pub async fn set_equalizer(&self, role: ChannelRole, equalizer: Equalizer) {
        {
            let mut st = self.inner.lock().await;
            if let Some(sink) = st.sinks.iter_mut().find(|s| s.role == role) {
                sink.equalizer = equalizer;
            }
        }
        self.emit_state_changed().await;
    }

    /// Update the microphone chain and notify clients.
    pub async fn set_mic(&self, settings: MicSettings) {
        self.inner.lock().await.mic = settings;
        self.emit_state_changed().await;
    }

    /// Update spatial-audio settings and notify clients.
    pub async fn set_spatial(&self, settings: SpatialSettings) {
        self.inner.lock().await.spatial = settings;
        self.emit_state_changed().await;
    }

    /// Update the reported PipeWire connectivity flag.
    pub async fn set_pipewire_connected(&self, connected: bool) {
        self.inner.lock().await.pipewire_connected = connected;
        self.emit(Event::PipeWireStatus { connected });
    }
}
