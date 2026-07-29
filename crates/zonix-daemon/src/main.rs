//! zonixd — the zOnix audio engine daemon.
//!
//! Responsibilities:
//!   - own the connection to PipeWire (Phase 2) and create the virtual sinks,
//!   - hold the authoritative [`SystemState`],
//!   - serve the control IPC socket that the GUI/CLI connect to.
//!
//! It is designed to run as a `systemd --user` service (see systemd/), so it
//! logs to stderr (journald captures it) and shuts down cleanly on SIGTERM.

mod engine;
mod ipc;
mod state;

use std::sync::Arc;

use anyhow::Result;
use tracing::{info, warn};
use tracing_subscriber::{prelude::*, EnvFilter};

use zonix_core::model::SystemState;

use crate::engine::{stub::StubEngine, AudioEngine};
use crate::ipc::Ctx;
use crate::state::SharedState;

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    info!(version = env!("CARGO_PKG_VERSION"), "starting zonixd");

    // --- Shared state ----------------------------------------------------------
    let shared = SharedState::new(SystemState::default());

    // --- Build the audio engine ------------------------------------------------
    // With `--features pipewire-backend` we try the real PipeWire engine and
    // fall back to the in-memory stub if PipeWire is unreachable. Without the
    // feature, we always use the stub.
    let engine: Arc<dyn AudioEngine> = build_engine(&shared);
    if let Err(err) = engine.initialize_graph() {
        warn!(%err, "engine graph init failed; continuing in degraded mode");
    }

    // --- IPC server ------------------------------------------------------------
    let ctx = Ctx {
        state: shared.clone(),
        engine: engine.clone(),
    };
    let server = tokio::spawn(ipc::serve(ctx));

    // --- Wait for shutdown -----------------------------------------------------
    wait_for_shutdown().await;
    info!("shutdown signal received, cleaning up");

    server.abort();
    ipc::cleanup_socket().await;
    if let Err(err) = engine.shutdown() {
        warn!(%err, "engine shutdown reported an error");
    }
    info!("zonixd stopped");
    Ok(())
}

/// Build the audio engine when the real PipeWire backend is compiled in.
///
/// Tries to connect to PipeWire; on success it spawns a task that mirrors
/// PipeWire updates (streams appearing/leaving, connection status) into the
/// shared state. On failure it logs and falls back to the stub engine.
#[cfg(feature = "pipewire-backend")]
fn build_engine(shared: &SharedState) -> Arc<dyn AudioEngine> {
    use crate::engine::pipewire::{PipeWireEngine, PwUpdate};

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<PwUpdate>();
    match PipeWireEngine::new(tx) {
        Ok(engine) => {
            let shared = shared.clone();
            tokio::spawn(async move {
                while let Some(update) = rx.recv().await {
                    match update {
                        PwUpdate::Connected(connected) => {
                            shared.set_pipewire_connected(connected).await
                        }
                        PwUpdate::SinkReady { role, node_id } => {
                            shared.set_sink_node_id(role, node_id).await
                        }
                        PwUpdate::StreamAdded(stream) => shared.add_stream(stream).await,
                        PwUpdate::StreamRemoved(node_id) => shared.remove_stream(node_id).await,
                    }
                }
            });
            info!("using PipeWire backend");
            Arc::new(engine)
        }
        Err(err) => {
            warn!(%err, "PipeWire backend unavailable; using stub engine");
            Arc::new(StubEngine::new())
        }
    }
}

/// Build the audio engine when only the stub backend is available.
#[cfg(not(feature = "pipewire-backend"))]
fn build_engine(_shared: &SharedState) -> Arc<dyn AudioEngine> {
    info!("using stub engine (built without pipewire-backend feature)");
    Arc::new(StubEngine::new())
}

/// Initialize `tracing` with `RUST_LOG` support (default: info).
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,zonixd=debug,zonix_daemon=debug"));
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_target(false))
        .init();
}

/// Resolve when the process receives SIGINT or SIGTERM.
async fn wait_for_shutdown() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    let mut int = signal(SignalKind::interrupt()).expect("install SIGINT handler");
    tokio::select! {
        _ = term.recv() => {}
        _ = int.recv() => {}
    }
}
