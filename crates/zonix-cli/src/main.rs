//! zonixctl — a tiny command-line client for the zOnix daemon.
//!
//! It connects to the control socket, performs the `Hello` handshake, then runs
//! one command. Useful for scripting and for testing the daemon without the GUI.
//!
//! Usage:
//!   zonixctl state
//!   zonixctl volume <game|chat|media|system> <0.0..1.0>
//!   zonixctl mute   <game|chat|media|system> <on|off>

use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use zonix_core::{
    ipc::socket_path,
    model::ChannelRole,
    protocol::{Request, Response, RpcMessage},
    PROTOCOL_VERSION,
};

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("zonixctl: {err:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        print_usage();
        bail!("no command given");
    }

    let path = socket_path();
    let stream = UnixStream::connect(&path)
        .await
        .with_context(|| format!("connecting to daemon at {} (is zonixd running?)", path.display()))?;
    let (read_half, mut write_half) = stream.into_split();
    let mut lines = BufReader::new(read_half).lines();
    let mut next_id: u64 = 0;

    // Handshake first.
    let hello = send(
        &mut write_half,
        &mut lines,
        &mut next_id,
        Request::Hello {
            protocol: PROTOCOL_VERSION,
        },
    )
    .await?;
    if let Response::Error { error } = hello {
        bail!("handshake rejected: {error}");
    }

    // Dispatch the requested command.
    let request = parse_command(&args)?;
    let response = send(&mut write_half, &mut lines, &mut next_id, request).await?;
    print_response(&response);
    Ok(())
}

/// Turn CLI args into a [`Request`].
fn parse_command(args: &[String]) -> Result<Request> {
    match args[0].as_str() {
        "state" => Ok(Request::GetState),
        "volume" => {
            let role = parse_role(arg(args, 1)?)?;
            let volume: f32 = arg(args, 2)?.parse().context("volume must be a number")?;
            Ok(Request::SetSinkVolume { role, volume })
        }
        "mute" => {
            let role = parse_role(arg(args, 1)?)?;
            let muted = matches!(arg(args, 2)?, "on" | "true" | "1");
            Ok(Request::SetSinkMute { role, muted })
        }
        other => {
            print_usage();
            bail!("unknown command: {other}");
        }
    }
}

fn parse_role(s: &str) -> Result<ChannelRole> {
    Ok(match s {
        "game" => ChannelRole::Game,
        "chat" => ChannelRole::Chat,
        "media" => ChannelRole::Media,
        "system" => ChannelRole::System,
        "mic" | "microphone" => ChannelRole::Microphone,
        other => bail!("unknown role: {other}"),
    })
}

fn arg(args: &[String], i: usize) -> Result<&str> {
    args.get(i)
        .map(String::as_str)
        .with_context(|| format!("missing argument #{i}"))
}

/// Send one request and await the matching response.
async fn send(
    write_half: &mut tokio::net::unix::OwnedWriteHalf,
    lines: &mut tokio::io::Lines<BufReader<tokio::net::unix::OwnedReadHalf>>,
    next_id: &mut u64,
    request: Request,
) -> Result<Response> {
    let id = *next_id;
    *next_id += 1;

    let mut frame = serde_json::to_string(&RpcMessage::Request { id, request })?;
    frame.push('\n');
    write_half.write_all(frame.as_bytes()).await?;
    write_half.flush().await?;

    // Read until we see the response with our id (skipping any events).
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(RpcMessage::Response { id: rid, response }) =
            serde_json::from_str::<RpcMessage>(&line)
        {
            if rid == id {
                return Ok(response);
            }
        }
    }
    bail!("connection closed before a response arrived");
}

fn print_response(response: &Response) {
    match response {
        Response::State { state } => {
            println!("PipeWire connected: {}", state.pipewire_connected);
            println!("Lanes:");
            for sink in &state.sinks {
                println!(
                    "  {:<8} vol={:>4.0}%  {}",
                    sink.role.label(),
                    sink.volume * 100.0,
                    if sink.muted { "[muted]" } else { "" }
                );
            }
            println!("Active streams: {}", state.streams.len());
        }
        Response::Ok => println!("ok"),
        Response::Error { error } => println!("error: {error}"),
        other => {
            // Fall back to pretty JSON for the less common responses.
            match serde_json::to_string_pretty(other) {
                Ok(s) => println!("{s}"),
                Err(_) => println!("{other:?}"),
            }
        }
    }
}

fn print_usage() {
    eprintln!(
        "usage:\n  \
         zonixctl state\n  \
         zonixctl volume <game|chat|media|system> <0.0..1.0>\n  \
         zonixctl mute   <game|chat|media|system> <on|off>"
    );
}
