//! Real PipeWire backend (compiled only with `--features pipewire-backend`).
//!
//! # How it fits together
//!
//! The virtual sinks themselves (`zonix.sink.{game,chat,media,system}`) are
//! created declaratively by the loopback config drop-in
//! (`wireplumber/config/pipewire.conf.d/50-zonix-loopback.conf`), so they are
//! audible — each lane mixes into the current default output. This engine is
//! the **control plane**: it connects to PipeWire and
//!
//!   * discovers the lane sinks (by `node.name`) and binds a proxy to each so
//!     it can drive volume/mute,
//!   * discovers application playback streams and reports them to the daemon,
//!   * moves a stream into a lane by writing `target.object` into the default
//!     metadata (WirePlumber then re-links it),
//!   * (opt-in) can create bare null-sinks via the native factory API as a
//!     fallback when the loopback config is not installed.
//!
//! # Threading
//!
//! All PipeWire objects are `!Send`, so the whole PipeWire main loop lives on
//! one dedicated OS thread. The async daemon talks *to* that thread through a
//! `pipewire::channel` (which wakes the loop), and receives updates *from* it
//! through a Tokio `UnboundedSender` (whose `send` is callable off-runtime).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::mpsc as std_mpsc;
use std::sync::Mutex;
use std::thread::{self, JoinHandle};

use pipewire as pw;
use pw::proxy::ProxyT;
use pw::spa;
use tokio::sync::mpsc::UnboundedSender;
use tracing::{debug, error, info, warn};

use zonix_core::model::{AppStream, ChannelRole};

use super::{AudioEngine, EngineError, EngineResult};

/// Prefix shared by all zOnix lane sinks.
const SINK_PREFIX: &str = "zonix.sink.";

/// Updates pushed from the PipeWire thread up to the async daemon.
#[derive(Debug, Clone)]
pub enum PwUpdate {
    /// Connection to PipeWire came up (`true`) or went down (`false`).
    Connected(bool),
    /// A lane sink was discovered and is now controllable.
    SinkReady { role: ChannelRole, node_id: u32 },
    /// A new application playback stream appeared.
    StreamAdded(AppStream),
    /// An application playback stream disappeared.
    StreamRemoved(u32),
}

/// Commands sent from the async world down to the PipeWire thread.
enum Command {
    SetVolume { role: ChannelRole, volume: f32 },
    SetMute { role: ChannelRole, muted: bool },
    RouteStream { node_id: u32, role: ChannelRole },
    Shutdown,
}

/// The public engine handle held by the daemon (behind `Arc<dyn AudioEngine>`).
pub struct PipeWireEngine {
    /// Command channel into the PipeWire loop. Wrapped in a `Mutex` so the
    /// handle is `Sync` (the trait requires `Send + Sync`).
    tx: Mutex<pw::channel::Sender<Command>>,
    /// Join handle for the PipeWire thread, taken on shutdown.
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl PipeWireEngine {
    /// Spawn the PipeWire thread and connect. Blocks until the initial
    /// connection either succeeds or fails, so the caller can fall back to the
    /// stub engine on error.
    pub fn new(events: UnboundedSender<PwUpdate>) -> EngineResult<Self> {
        let (cmd_tx, cmd_rx) = pw::channel::channel::<Command>();
        // One-shot init handshake: the thread reports whether connect() worked.
        let (init_tx, init_rx) = std_mpsc::channel::<Result<(), String>>();

        let thread = thread::Builder::new()
            .name("zonix-pipewire".to_string())
            .spawn(move || {
                if let Err(err) = run_loop(events, cmd_rx, init_tx) {
                    error!(%err, "PipeWire thread exited with error");
                }
            })
            .map_err(|e| EngineError::Internal(format!("spawning PipeWire thread: {e}")))?;

        match init_rx.recv() {
            Ok(Ok(())) => {
                info!("PipeWire backend connected");
                Ok(Self {
                    tx: Mutex::new(cmd_tx),
                    thread: Mutex::new(Some(thread)),
                })
            }
            Ok(Err(msg)) => Err(EngineError::PipeWire(msg)),
            Err(_) => Err(EngineError::PipeWire(
                "PipeWire thread died during startup".to_string(),
            )),
        }
    }

    /// Send a command into the PipeWire loop.
    fn send(&self, cmd: Command) -> EngineResult<()> {
        self.tx
            .lock()
            .map_err(|_| EngineError::Internal("command channel poisoned".into()))?
            .send(cmd)
            .map_err(|_| EngineError::PipeWire("PipeWire loop is not running".into()))
    }
}

impl AudioEngine for PipeWireEngine {
    fn initialize_graph(&self) -> EngineResult<()> {
        // Sinks are created by the loopback config and discovered asynchronously
        // by the registry listener; nothing to do synchronously here.
        Ok(())
    }

    fn set_sink_volume(&self, role: ChannelRole, volume: f32) -> EngineResult<()> {
        self.send(Command::SetVolume {
            role,
            volume: volume.clamp(0.0, 1.0),
        })
    }

    fn set_sink_mute(&self, role: ChannelRole, muted: bool) -> EngineResult<()> {
        self.send(Command::SetMute { role, muted })
    }

    fn route_stream(&self, node_id: u32, role: ChannelRole) -> EngineResult<()> {
        self.send(Command::RouteStream { node_id, role })
    }

    fn apply_equalizer(&self, role: ChannelRole, _eq: &zonix_core::model::Equalizer) -> EngineResult<()> {
        // The parametric EQ filter-chain is wired up in Phase 3.
        debug!(role = %role.slug(), "apply_equalizer: deferred to Phase 3 (filter-chain)");
        Ok(())
    }

    fn shutdown(&self) -> EngineResult<()> {
        let _ = self.send(Command::Shutdown);
        if let Ok(mut guard) = self.thread.lock() {
            if let Some(handle) = guard.take() {
                let _ = handle.join();
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// PipeWire thread
// ---------------------------------------------------------------------------

/// Per-thread runtime state shared between the registry/command callbacks.
struct Runtime {
    registry: Rc<pw::registry::Registry>,
    /// Bound proxies for each discovered lane sink, used to set volume/mute.
    sinks: HashMap<ChannelRole, pw::node::Node>,
    /// Channel count per lane sink (for building channelVolumes arrays).
    sink_channels: HashMap<ChannelRole, usize>,
    /// The "default" metadata proxy, used to route streams to a lane.
    default_metadata: Option<pw::metadata::Metadata>,
    /// Application streams we've reported, so we can emit removals.
    streams: HashSet<u32>,
    /// Upward event channel to the async daemon.
    events: UnboundedSender<PwUpdate>,
}

/// Entry point for the dedicated PipeWire thread.
fn run_loop(
    events: UnboundedSender<PwUpdate>,
    cmd_rx: pw::channel::Receiver<Command>,
    init_tx: std_mpsc::Sender<Result<(), String>>,
) -> Result<(), pw::Error> {
    pw::init();

    let mainloop = match pw::main_loop::MainLoop::new(None) {
        Ok(ml) => ml,
        Err(e) => {
            let _ = init_tx.send(Err(format!("creating main loop: {e}")));
            return Err(e);
        }
    };
    let context = match pw::context::Context::new(&mainloop) {
        Ok(c) => c,
        Err(e) => {
            let _ = init_tx.send(Err(format!("creating context: {e}")));
            return Err(e);
        }
    };
    let core = match context.connect(None) {
        Ok(c) => c,
        Err(e) => {
            let _ = init_tx.send(Err(format!("connecting to PipeWire: {e}")));
            return Err(e);
        }
    };
    let registry = match core.get_registry() {
        Ok(r) => Rc::new(r),
        Err(e) => {
            let _ = init_tx.send(Err(format!("getting registry: {e}")));
            return Err(e);
        }
    };

    let runtime = Rc::new(RefCell::new(Runtime {
        registry: registry.clone(),
        sinks: HashMap::new(),
        sink_channels: HashMap::new(),
        default_metadata: None,
        streams: HashSet::new(),
        events: events.clone(),
    }));

    // Registry listener: discover lane sinks, app streams and the default
    // metadata as objects appear on the graph.
    let rt_global = runtime.clone();
    let rt_remove = runtime.clone();
    let _registry_listener = registry
        .add_listener_local()
        .global(move |global| on_global(&rt_global, global))
        .global_remove(move |id| on_global_remove(&rt_remove, id))
        .register();

    // Command receiver attached to the loop (wakes it on send).
    let rt_cmd = runtime.clone();
    let ml_for_cmd = mainloop.clone();
    let _cmd_receiver = cmd_rx.attach(mainloop.loop_(), move |cmd| {
        handle_command(&rt_cmd, &ml_for_cmd, cmd);
    });

    // Connected: tell the daemon and start pumping.
    let _ = events.send(PwUpdate::Connected(true));
    let _ = init_tx.send(Ok(()));
    info!("PipeWire main loop running");

    mainloop.run();

    // Loop returned (Shutdown command called quit()).
    let _ = events.send(PwUpdate::Connected(false));
    info!("PipeWire main loop stopped");
    Ok(())
}

/// Handle a newly-appeared global object.
fn on_global(runtime: &Rc<RefCell<Runtime>>, global: &pw::registry::GlobalObject<&spa::utils::dict::DictRef>) {
    let Some(props) = global.props else {
        return;
    };

    match global.type_ {
        pw::types::ObjectType::Node => {
            let media_class = props.get("media.class").unwrap_or("");
            let node_name = props.get("node.name").unwrap_or("");

            // A zOnix lane sink? Bind it so we can control volume/mute.
            if media_class == "Audio/Sink" {
                if let Some(role) = role_from_sink_name(node_name) {
                    // Clone the registry handle first so we don't hold an
                    // immutable borrow of the RefCell across the borrow_mut
                    // below (that would panic — and panicking inside a C
                    // callback aborts the whole process).
                    let registry = runtime.borrow().registry.clone();
                    match registry.bind::<pw::node::Node, _>(global) {
                        Ok(node) => {
                            let channels = props
                                .get("audio.channels")
                                .and_then(|s| s.parse::<usize>().ok())
                                .unwrap_or(2);
                            let mut rt = runtime.borrow_mut();
                            rt.sinks.insert(role, node);
                            rt.sink_channels.insert(role, channels.max(1));
                            let _ = rt.events.send(PwUpdate::SinkReady {
                                role,
                                node_id: global.id,
                            });
                            info!(role = %role.slug(), node_id = global.id, "lane sink ready");
                        }
                        Err(e) => warn!(%e, node = node_name, "failed to bind lane sink"),
                    }
                }
                return;
            }

            // An application playback stream? Report it to the daemon.
            if media_class == "Stream/Output/Audio" {
                // Ignore our own loopback playback streams.
                if node_name.starts_with(SINK_PREFIX) {
                    return;
                }
                let app_name = props
                    .get("application.name")
                    .or_else(|| props.get("node.description"))
                    .unwrap_or(node_name)
                    .to_string();
                let pid = props
                    .get("application.process.id")
                    .and_then(|s| s.parse::<u32>().ok());
                let stream = AppStream {
                    node_id: global.id,
                    app_name,
                    pid,
                    role: ChannelRole::Media, // default lane; user can re-route
                    volume: 1.0,
                    muted: false,
                };
                let mut rt = runtime.borrow_mut();
                rt.streams.insert(global.id);
                let _ = rt.events.send(PwUpdate::StreamAdded(stream));
                debug!(node_id = global.id, "app stream added");
            }
        }

        pw::types::ObjectType::Metadata => {
            if props.get("metadata.name") == Some("default") {
                let registry = runtime.borrow().registry.clone();
                if let Ok(md) = registry.bind::<pw::metadata::Metadata, _>(global) {
                    runtime.borrow_mut().default_metadata = Some(md);
                    debug!("bound default metadata (stream routing enabled)");
                }
            }
        }

        _ => {}
    }
}

/// Handle a removed global object.
fn on_global_remove(runtime: &Rc<RefCell<Runtime>>, id: u32) {
    let mut rt = runtime.borrow_mut();
    if rt.streams.remove(&id) {
        let _ = rt.events.send(PwUpdate::StreamRemoved(id));
        debug!(node_id = id, "app stream removed");
        return;
    }
    // If a lane sink vanished (e.g. config reloaded), drop its proxy.
    let gone: Vec<ChannelRole> = rt
        .sinks
        .iter()
        .filter_map(|(role, node)| (node.upcast_ref().id() == id).then_some(*role))
        .collect();
    for role in gone {
        rt.sinks.remove(&role);
        rt.sink_channels.remove(&role);
        warn!(role = %role.slug(), "lane sink disappeared");
    }
}

/// Execute a command coming from the async daemon.
fn handle_command(runtime: &Rc<RefCell<Runtime>>, mainloop: &pw::main_loop::MainLoop, cmd: Command) {
    match cmd {
        Command::SetVolume { role, volume } => {
            let rt = runtime.borrow();
            let channels = rt.sink_channels.get(&role).copied().unwrap_or(2);
            if let Some(node) = rt.sinks.get(&role) {
                if let Err(e) = set_node_volume(node, channels, volume) {
                    warn!(role = %role.slug(), %e, "set volume failed");
                }
            } else {
                warn!(role = %role.slug(), "no sink bound; is the loopback config installed?");
            }
        }
        Command::SetMute { role, muted } => {
            let rt = runtime.borrow();
            if let Some(node) = rt.sinks.get(&role) {
                if let Err(e) = set_node_mute(node, muted) {
                    warn!(role = %role.slug(), %e, "set mute failed");
                }
            } else {
                warn!(role = %role.slug(), "no sink bound for mute");
            }
        }
        Command::RouteStream { node_id, role } => {
            let rt = runtime.borrow();
            match &rt.default_metadata {
                Some(md) => {
                    let target = format!("{SINK_PREFIX}{}", role.slug());
                    // WirePlumber re-links the stream to this target sink.
                    md.set_property(node_id, "target.object", Some("Spa:String"), Some(&target));
                    debug!(node_id, target, "routed stream via default metadata");
                }
                None => warn!("default metadata not available; cannot route stream yet"),
            }
        }
        Command::Shutdown => {
            info!("PipeWire loop received shutdown");
            mainloop.quit();
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Map a lane sink node name (`zonix.sink.game`) to its [`ChannelRole`].
fn role_from_sink_name(name: &str) -> Option<ChannelRole> {
    let slug = name.strip_prefix(SINK_PREFIX)?;
    // Ignore the `.out` playback side of the loopback.
    if slug.contains('.') {
        return None;
    }
    ChannelRole::PLAYBACK.into_iter().find(|r| r.slug() == slug)
}

/// Build and send a `Props` param setting `channelVolumes` on a node.
fn set_node_volume(node: &pw::node::Node, channels: usize, volume: f32) -> Result<(), String> {
    let volumes = vec![volume; channels.max(1)];
    let pod = build_props_pod(Some(&volumes), None)?;
    node.set_param(
        spa::param::ParamType::Props,
        0,
        pod_from_bytes(&pod)?,
    );
    Ok(())
}

/// Build and send a `Props` param setting `mute` on a node.
fn set_node_mute(node: &pw::node::Node, muted: bool) -> Result<(), String> {
    let pod = build_props_pod(None, Some(muted))?;
    node.set_param(
        spa::param::ParamType::Props,
        0,
        pod_from_bytes(&pod)?,
    );
    Ok(())
}

/// Serialize a SPA `Props` object with the requested fields into POD bytes.
fn build_props_pod(volumes: Option<&[f32]>, mute: Option<bool>) -> Result<Vec<u8>, String> {
    use spa::pod::{serialize::PodSerializer, Object, Property, PropertyFlags, Value, ValueArray};

    let mut properties = Vec::new();
    if let Some(m) = mute {
        properties.push(Property {
            key: spa::sys::SPA_PROP_mute,
            flags: PropertyFlags::empty(),
            value: Value::Bool(m),
        });
    }
    if let Some(v) = volumes {
        properties.push(Property {
            key: spa::sys::SPA_PROP_channelVolumes,
            flags: PropertyFlags::empty(),
            value: Value::ValueArray(ValueArray::Float(v.to_vec())),
        });
    }

    let object = Value::Object(Object {
        type_: spa::sys::SPA_TYPE_OBJECT_Props,
        id: spa::sys::SPA_PARAM_Props,
        properties,
    });

    PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &object)
        .map(|(cursor, _)| cursor.into_inner())
        .map_err(|e| format!("serializing Props pod: {e}"))
}

/// View raw POD bytes as a `&Pod` (validated).
fn pod_from_bytes(bytes: &[u8]) -> Result<&spa::pod::Pod, String> {
    spa::pod::Pod::from_bytes(bytes).ok_or_else(|| "invalid POD bytes".to_string())
}
