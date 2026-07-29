//! Domain model for the zOnix audio graph.
//!
//! These types are the "nouns" of the system. They map almost 1:1 onto the
//! PipeWire objects the daemon manages, but they are deliberately UI-friendly
//! (serde-serializable, no raw pointers) so the same structs can be sent to the
//! GUI without translation.

use serde::{Deserialize, Serialize};

/// A logical audio "lane" the user can mix independently.
///
/// Each role is backed by a real PipeWire virtual sink created by the daemon
/// (see the WirePlumber scripts). Applications are routed into one of these
/// sinks; the daemon then mixes them down to the physical output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelRole {
    /// Game audio.
    Game,
    /// Voice chat (Discord, Mumble, TeamSpeak...).
    Chat,
    /// Music / video / browser media.
    Media,
    /// Everything else / OS notifications.
    System,
    /// Microphone capture chain (input, not a playback sink).
    Microphone,
}

impl ChannelRole {
    /// The four playback lanes, in display order. `Microphone` is excluded
    /// because it is a capture chain, not a mix lane.
    pub const PLAYBACK: [ChannelRole; 4] = [
        ChannelRole::Game,
        ChannelRole::Chat,
        ChannelRole::Media,
        ChannelRole::System,
    ];

    /// Stable identifier used as the PipeWire node name suffix, e.g.
    /// `zonix.sink.game`.
    pub fn slug(self) -> &'static str {
        match self {
            ChannelRole::Game => "game",
            ChannelRole::Chat => "chat",
            ChannelRole::Media => "media",
            ChannelRole::System => "system",
            ChannelRole::Microphone => "microphone",
        }
    }

    /// Human-readable label for the GUI.
    pub fn label(self) -> &'static str {
        match self {
            ChannelRole::Game => "Game",
            ChannelRole::Chat => "Chat",
            ChannelRole::Media => "Media",
            ChannelRole::System => "System",
            ChannelRole::Microphone => "Microphone",
        }
    }
}

/// A virtual sink managed by the daemon. One per playback [`ChannelRole`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VirtualSink {
    /// Which lane this sink represents.
    pub role: ChannelRole,
    /// PipeWire global node id (`None` until the node has been created).
    pub node_id: Option<u32>,
    /// PipeWire node name, e.g. `zonix.sink.game`.
    pub node_name: String,
    /// Linear gain, 0.0..=1.0 (1.0 == unity). The GUI shows this as a fader.
    pub volume: f32,
    /// Whether the lane is muted.
    pub muted: bool,
    /// Per-lane equalizer.
    pub equalizer: Equalizer,
}

impl VirtualSink {
    pub fn new(role: ChannelRole) -> Self {
        Self {
            role,
            node_id: None,
            node_name: format!("zonix.sink.{}", role.slug()),
            volume: 1.0,
            muted: false,
            equalizer: Equalizer::flat(),
        }
    }
}

/// A concrete application stream currently playing into one of the sinks.
///
/// Populated from the PipeWire registry: every `Stream/Output/Audio` node the
/// daemon sees is surfaced here so the GUI can show a per-app row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppStream {
    /// PipeWire node id of the application stream.
    pub node_id: u32,
    /// `application.name` from the node properties (best effort).
    pub app_name: String,
    /// PID of the owning process, if exposed.
    pub pid: Option<u32>,
    /// Which lane this stream is currently routed into.
    pub role: ChannelRole,
    /// Per-app volume, 0.0..=1.0.
    pub volume: f32,
    pub muted: bool,
}

/// Kind of biquad filter applied by a single EQ band.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EqFilterKind {
    Peaking,
    LowShelf,
    HighShelf,
    LowPass,
    HighPass,
}

/// One parametric EQ band.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EqBand {
    pub kind: EqFilterKind,
    /// Center/cutoff frequency in Hz.
    pub frequency: f32,
    /// Gain in dB (ignored for pure pass filters).
    pub gain_db: f32,
    /// Quality factor / bandwidth.
    pub q: f32,
    /// Whether this band is active.
    pub enabled: bool,
}

impl EqBand {
    pub fn peaking(frequency: f32) -> Self {
        Self {
            kind: EqFilterKind::Peaking,
            frequency,
            gain_db: 0.0,
            q: 1.41,
            enabled: true,
        }
    }
}

/// A parametric equalizer (default is a 10-band graphic layout, but bands are
/// fully parametric so presets can reshape them freely).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Equalizer {
    pub enabled: bool,
    pub preamp_db: f32,
    pub bands: Vec<EqBand>,
}

impl Equalizer {
    /// A flat 10-band ISO graphic EQ (all gains at 0 dB).
    pub fn flat() -> Self {
        const FREQS: [f32; 10] = [
            31.0, 62.0, 125.0, 250.0, 500.0, 1_000.0, 2_000.0, 4_000.0, 8_000.0, 16_000.0,
        ];
        Self {
            enabled: false,
            preamp_db: 0.0,
            bands: FREQS.iter().map(|&f| EqBand::peaking(f)).collect(),
        }
    }
}

/// Noise gate parameters for the microphone chain.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NoiseGate {
    pub enabled: bool,
    /// Signal must exceed this level (dBFS) to open the gate.
    pub threshold_db: f32,
    /// How fast the gate opens (ms).
    pub attack_ms: f32,
    /// How long the gate stays open after signal drops (ms).
    pub release_ms: f32,
}

impl Default for NoiseGate {
    fn default() -> Self {
        Self {
            enabled: true,
            threshold_db: -45.0,
            attack_ms: 5.0,
            release_ms: 120.0,
        }
    }
}

/// Microphone enhancement chain ("ClearCast IA").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MicSettings {
    /// RNNoise-based background noise suppression.
    pub noise_suppression: bool,
    /// Suppression strength 0.0..=1.0 (blend between dry and denoised signal).
    pub suppression_strength: f32,
    pub gate: NoiseGate,
    /// Simple make-up / compression toggle.
    pub compressor_enabled: bool,
    /// Input gain in dB applied before the chain.
    pub input_gain_db: f32,
}

impl Default for MicSettings {
    fn default() -> Self {
        Self {
            noise_suppression: true,
            suppression_strength: 0.9,
            gate: NoiseGate::default(),
            compressor_enabled: true,
            input_gain_db: 0.0,
        }
    }
}

/// Virtual surround (HRTF) configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpatialSettings {
    pub enabled: bool,
    /// Identifier of the HRIR/SOFA impulse-response set to convolve with.
    pub hrtf_profile: String,
    /// Simulated speaker layout (e.g. "7.1").
    pub layout: String,
}

impl Default for SpatialSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            hrtf_profile: "default".to_string(),
            layout: "7.1".to_string(),
        }
    }
}

/// A saved audio profile. Profiles can be auto-activated when a matching
/// application/game comes to the foreground (see `match_rules`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    /// Per-lane sink configuration (volume, mute, EQ).
    pub sinks: Vec<VirtualSink>,
    pub mic: MicSettings,
    pub spatial: SpatialSettings,
    /// Substrings matched against the foreground process name to auto-activate
    /// this profile (case-insensitive). Empty == never auto-activates.
    pub match_rules: Vec<String>,
}

impl Profile {
    /// A sane default profile with all four playback lanes at unity gain.
    pub fn default_named(name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            id: slug(&name),
            name,
            sinks: ChannelRole::PLAYBACK
                .iter()
                .map(|&r| VirtualSink::new(r))
                .collect(),
            mic: MicSettings::default(),
            spatial: SpatialSettings::default(),
            match_rules: Vec::new(),
        }
    }
}

/// Full snapshot of the daemon's runtime state, sent to a client on connect and
/// pushed again (via events) whenever something changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SystemState {
    /// The four playback lanes and their live state.
    pub sinks: Vec<VirtualSink>,
    /// Application streams currently detected on the graph.
    pub streams: Vec<AppStream>,
    pub mic: MicSettings,
    pub spatial: SpatialSettings,
    /// Id of the currently active profile, if any.
    pub active_profile: Option<String>,
    /// Whether the PipeWire connection is healthy.
    pub pipewire_connected: bool,
}

impl Default for SystemState {
    fn default() -> Self {
        Self {
            sinks: ChannelRole::PLAYBACK
                .iter()
                .map(|&r| VirtualSink::new(r))
                .collect(),
            streams: Vec::new(),
            mic: MicSettings::default(),
            spatial: SpatialSettings::default(),
            active_profile: None,
            pipewire_connected: false,
        }
    }
}

/// Lowercase, hyphenated slug used for stable ids.
fn slug(s: &str) -> String {
    s.trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_eq_has_ten_bands() {
        assert_eq!(Equalizer::flat().bands.len(), 10);
    }

    #[test]
    fn sink_node_name_uses_role_slug() {
        assert_eq!(VirtualSink::new(ChannelRole::Game).node_name, "zonix.sink.game");
    }

    #[test]
    fn slug_is_stable() {
        assert_eq!(slug("FPS  Competitive!"), "fps-competitive");
    }
}
