//! IPC transport conventions shared by the daemon and its clients.
//!
//! We use a Unix domain socket under `$XDG_RUNTIME_DIR` so the socket is
//! per-user, automatically cleaned up on logout, and never exposed on the
//! network. Framing is newline-delimited JSON: one [`crate::protocol::RpcMessage`]
//! per line. This is trivial to implement from any language (the Tauri Rust
//! side, a Python test client, `socat`, etc.).

use std::path::PathBuf;

/// Subdirectory (under the runtime dir) that holds zOnix's runtime files.
pub const RUNTIME_SUBDIR: &str = "zonix";

/// File name of the control socket.
pub const SOCKET_NAME: &str = "daemon.sock";

/// Resolve the control socket path: `$XDG_RUNTIME_DIR/zonix/daemon.sock`,
/// falling back to `/tmp/zonix-$UID/daemon.sock` when the runtime dir is unset.
pub fn socket_path() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            // Fallback keeps things per-user even without a login session.
            let uid = users_uid();
            PathBuf::from(format!("/tmp/zonix-{uid}"))
        });
    base.join(RUNTIME_SUBDIR).join(SOCKET_NAME)
}

/// Best-effort current uid without pulling in an extra crate.
fn users_uid() -> u32 {
    // SAFETY: getuid() is always successful and has no side effects.
    unsafe { libc_getuid() }
}

// Minimal libc shim so the fallback path does not require the `libc` crate in
// this dependency-light shared crate. `getuid` never fails.
extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_path_ends_with_expected_components() {
        std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000");
        let p = socket_path();
        assert_eq!(p, PathBuf::from("/run/user/1000/zonix/daemon.sock"));
    }
}
