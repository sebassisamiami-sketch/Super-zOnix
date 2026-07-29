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

    // --- Build the audio engine ------------------------------------------------
    // Phase 1: in-memory stub. Phase 2 swaps this for PipeWireEngine::new()? .
    let engine: Arc<dyn AudioEngine> = Arc::new(StubEngine::new());
    if let Err(err) = engine.initialize_graph() {
        warn!(%err, "engine graph init failed; continuing in degraded mode");
    }

    // --- Shared state ----------------------------------------------------------
    let mut initial = SystemState::default();
    // The stub engine has no real PipeWire connection; mark accordingly.
    initial.pipewire_connected = false;
    let shared = SharedState::new(initial);

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
