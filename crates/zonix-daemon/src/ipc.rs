//! Unix-domain-socket IPC server.
//!
//! Listens on `$XDG_RUNTIME_DIR/zonix/daemon.sock`, accepts multiple concurrent
//! clients, and for each connection:
//!   1. reads newline-delimited JSON [`RpcMessage`] frames,
//!   2. dispatches [`Request`]s to [`handle_request`],
//!   3. writes back the correlated [`Response`], and
//!   4. forwards daemon-wide [`Event`]s pushed via the broadcast channel.

use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use zonix_core::{
    ipc::socket_path,
    protocol::{ErrorCode, Event, Request, Response, RpcError, RpcMessage},
    PROTOCOL_VERSION,
};

use crate::engine::AudioEngine;
use crate::state::SharedState;

/// Shared context handed to every request handler.
#[derive(Clone)]
pub struct Ctx {
    pub state: SharedState,
    pub engine: Arc<dyn AudioEngine>,
}

/// Bind the socket and serve clients until the task is cancelled/aborted.
pub async fn serve(ctx: Ctx) -> Result<()> {
    let path = socket_path();
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir)
            .await
            .with_context(|| format!("creating runtime dir {}", dir.display()))?;
    }
    // Remove a stale socket left behind by a previous crash.
    if path.exists() {
        let _ = tokio::fs::remove_file(&path).await;
    }

    let listener = UnixListener::bind(&path)
        .with_context(|| format!("binding control socket {}", path.display()))?;
    info!(socket = %path.display(), "IPC server listening");

    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let ctx = ctx.clone();
                tokio::spawn(async move {
                    if let Err(err) = handle_connection(stream, ctx).await {
                        debug!(%err, "client connection ended");
                    }
                });
            }
            Err(err) => {
                error!(%err, "failed to accept client");
            }
        }
    }
}

/// Drive a single client connection to completion.
async fn handle_connection(stream: UnixStream, ctx: Ctx) -> Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut lines = BufReader::new(read_half).lines();

    // Fan events from the broadcast channel into this connection's writer via a
    // small mpsc queue, so reads and event pushes don't contend on the socket.
    let (out_tx, mut out_rx) = mpsc::channel::<RpcMessage>(128);

    // Event forwarding task: broadcast -> this client.
    let mut events = ctx.state.subscribe();
    let event_tx = out_tx.clone();
    let event_task = tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => {
                    if event_tx.send(RpcMessage::Event { event }).await.is_err() {
                        break; // client writer gone
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    warn!(dropped = n, "client lagged on event stream");
                }
                Err(_) => break, // channel closed
            }
        }
    });

    // Writer task: serialize any RpcMessage to a JSON line.
    let writer_task = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            match serde_json::to_string(&msg) {
                Ok(mut line) => {
                    line.push('\n');
                    if write_half.write_all(line.as_bytes()).await.is_err() {
                        break;
                    }
                    let _ = write_half.flush().await;
                }
                Err(err) => error!(%err, "failed to serialize outgoing message"),
            }
        }
    });

    // Read loop: one request per line.
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let msg: RpcMessage = match serde_json::from_str(&line) {
            Ok(m) => m,
            Err(err) => {
                warn!(%err, "ignoring malformed frame");
                continue;
            }
        };

        if let RpcMessage::Request { id, request } = msg {
            let response = handle_request(&ctx, request).await;
            if out_tx
                .send(RpcMessage::Response { id, response })
                .await
                .is_err()
            {
                break;
            }
        } else {
            // Clients are not expected to send responses or events.
            warn!("client sent a non-request frame; ignoring");
        }
    }

    // Client disconnected: drop our sender so the writer task finishes, then
    // stop the event forwarder.
    drop(out_tx);
    event_task.abort();
    let _ = writer_task.await;
    Ok(())
}

/// Execute a single request against the engine + shared state.
async fn handle_request(ctx: &Ctx, request: Request) -> Response {
    match request {
        Request::Hello { protocol } => {
            if protocol.0 != PROTOCOL_VERSION.0 {
                return Response::Error {
                    error: RpcError::new(
                        ErrorCode::IncompatibleVersion,
                        format!(
                            "client protocol {}.{} incompatible with daemon {}.{}",
                            protocol.0, protocol.1, PROTOCOL_VERSION.0, PROTOCOL_VERSION.1
                        ),
                    ),
                };
            }
            Response::Hello {
                protocol: PROTOCOL_VERSION,
                daemon_version: env!("CARGO_PKG_VERSION").to_string(),
            }
        }

        Request::GetState => Response::State {
            state: Box::new(ctx.state.snapshot().await),
        },

        Request::SetSinkVolume { role, volume } => {
            map(ctx.engine.set_sink_volume(role, volume), || async {
                ctx.state.set_sink_volume(role, volume).await;
            })
            .await
        }

        Request::SetSinkMute { role, muted } => {
            map(ctx.engine.set_sink_mute(role, muted), || async {
                ctx.state.set_sink_mute(role, muted).await;
            })
            .await
        }

        Request::RouteStream { node_id, role } => match ctx.engine.route_stream(node_id, role) {
            Ok(()) => Response::Ok,
            Err(err) => engine_err(err),
        },

        Request::SetStreamVolume { node_id, volume } => {
            debug!(node_id, volume, "set stream volume (per-app)");
            // Per-app volume is applied directly on the stream node in Phase 2.
            Response::Ok
        }

        Request::SetEqualizer { role, equalizer } => {
            match ctx.engine.apply_equalizer(role, &equalizer) {
                Ok(()) => {
                    ctx.state.set_equalizer(role, equalizer).await;
                    Response::Ok
                }
                Err(err) => engine_err(err),
            }
        }

        Request::SetMicSettings { settings } => {
            ctx.state.set_mic(settings).await;
            Response::Ok
        }

        Request::SetSpatial { settings } => {
            ctx.state.set_spatial(settings).await;
            Response::Ok
        }

        // Profile persistence is implemented in a later phase (profiles module).
        Request::ListProfiles => Response::Profiles { profiles: vec![] },
        Request::SaveProfile { .. }
        | Request::LoadProfile { .. }
        | Request::DeleteProfile { .. } => Response::Ok,

        Request::SetAutoSwitch { enabled } => {
            debug!(enabled, "auto profile switching toggled");
            Response::Ok
        }
    }
}

/// Run an engine call; on success run the state mutation `on_ok` and return Ok,
/// on failure map the engine error onto an RPC error response.
async fn map<F, Fut>(engine_result: crate::engine::EngineResult<()>, on_ok: F) -> Response
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    match engine_result {
        Ok(()) => {
            on_ok().await;
            Response::Ok
        }
        Err(err) => engine_err(err),
    }
}

/// Convert an [`crate::engine::EngineError`] into a [`Response::Error`].
fn engine_err(err: crate::engine::EngineError) -> Response {
    use crate::engine::EngineError::*;
    let code = match err {
        PipeWire(_) => ErrorCode::PipeWire,
        NotFound(_) => ErrorCode::NotFound,
        Internal(_) => ErrorCode::Internal,
    };
    Response::Error {
        error: RpcError::new(code, err.to_string()),
    }
}

/// Best-effort removal of the socket file on shutdown.
pub async fn cleanup_socket() {
    let path = socket_path();
    if path.exists() {
        let _ = tokio::fs::remove_file(&path).await;
        debug!(socket = %path.display(), "removed control socket");
    }
}

// Keep the `Event` import used even before all event producers exist.
#[allow(dead_code)]
fn _assert_event_type(_e: Event) {}
