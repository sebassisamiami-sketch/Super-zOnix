//! The IPC contract between clients (GUI / CLI) and the daemon.
//!
//! Transport is newline-delimited JSON over a Unix domain socket (see
//! [`crate::ipc`]). Every frame is exactly one [`RpcMessage`]. Clients send
//! [`Request`]s wrapped in [`RpcMessage::Request`]; the daemon answers with a
//! [`RpcMessage::Response`] carrying the same `id`, and may at any time push
//! unsolicited [`RpcMessage::Event`]s (e.g. a new app stream appeared).

use serde::{Deserialize, Serialize};

use crate::model::{
    AppStream, ChannelRole, Equalizer, MicSettings, Profile, SpatialSettings, SystemState,
};

/// Top-level framed message. `id` correlates a response with its request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RpcMessage {
    Request { id: u64, request: Request },
    Response { id: u64, response: Response },
    Event { event: Event },
}

/// A command from a client to the daemon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// Handshake: client announces the protocol version it was built against.
    Hello { protocol: (u16, u16) },
    /// Ask for the full current [`SystemState`].
    GetState,
    /// Set the linear volume (0.0..=1.0) of a playback lane.
    SetSinkVolume { role: ChannelRole, volume: f32 },
    /// Mute/unmute a playback lane.
    SetSinkMute { role: ChannelRole, muted: bool },
    /// Move an application stream into a different lane.
    RouteStream { node_id: u32, role: ChannelRole },
    /// Set the per-app volume of a stream.
    SetStreamVolume { node_id: u32, volume: f32 },
    /// Replace the equalizer for a lane.
    SetEqualizer { role: ChannelRole, equalizer: Equalizer },
    /// Update the microphone enhancement chain.
    SetMicSettings { settings: MicSettings },
    /// Update virtual-surround settings.
    SetSpatial { settings: SpatialSettings },
    /// List all saved profiles.
    ListProfiles,
    /// Persist a profile (create or overwrite by id).
    SaveProfile { profile: Profile },
    /// Load a profile by id and apply it to the live graph.
    LoadProfile { id: String },
    /// Delete a saved profile.
    DeleteProfile { id: String },
    /// Enable/disable automatic profile switching by foreground app.
    SetAutoSwitch { enabled: bool },
}

/// The daemon's answer to a [`Request`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum Response {
    /// Handshake accepted; includes the daemon's own protocol version.
    Hello { protocol: (u16, u16), daemon_version: String },
    /// Full state snapshot.
    State { state: Box<SystemState> },
    /// The list of saved profiles.
    Profiles { profiles: Vec<Profile> },
    /// Generic success with no payload.
    Ok,
    /// The request failed.
    Error { error: RpcError },
}

/// An unsolicited notification pushed by the daemon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// The full state changed; clients should refresh.
    StateChanged { state: Box<SystemState> },
    /// A new application stream appeared on the graph.
    StreamAdded { stream: AppStream },
    /// An application stream went away.
    StreamRemoved { node_id: u32 },
    /// Peak/RMS meter update for a lane (linear 0.0..=1.0), for VU meters.
    LevelMeter { role: ChannelRole, peak: f32, rms: f32 },
    /// A profile was auto-activated because the foreground app matched.
    ProfileAutoActivated { id: String },
    /// The PipeWire connection state changed.
    PipeWireStatus { connected: bool },
}

/// A structured error returned in [`Response::Error`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[error("{code}: {message}")]
pub struct RpcError {
    pub code: ErrorCode,
    pub message: String,
}

impl RpcError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Machine-readable error categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The request was malformed or out of range.
    BadRequest,
    /// A referenced object (stream, profile...) does not exist.
    NotFound,
    /// The daemon could not reach or command PipeWire.
    PipeWire,
    /// Protocol version mismatch.
    IncompatibleVersion,
    /// Anything unexpected.
    Internal,
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ErrorCode::BadRequest => "bad_request",
            ErrorCode::NotFound => "not_found",
            ErrorCode::PipeWire => "pipewire",
            ErrorCode::IncompatibleVersion => "incompatible_version",
            ErrorCode::Internal => "internal",
        };
        f.write_str(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrips_through_json() {
        let msg = RpcMessage::Request {
            id: 7,
            request: Request::SetSinkVolume {
                role: ChannelRole::Game,
                volume: 0.5,
            },
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: RpcMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, back);
    }

    #[test]
    fn event_tag_is_snake_case() {
        let ev = Event::StreamRemoved { node_id: 42 };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains("\"event\":\"stream_removed\""));
    }
}
