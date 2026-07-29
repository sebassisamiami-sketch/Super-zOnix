# GUI (Phase 4)

The desktop frontend is a **Tauri 2 + React + TypeScript** app.

Planned layout:

```
gui/
├── src/                      # React app (TypeScript)
│   ├── components/           # Fader, EqGraph, MicPanel, ProfileBar...
│   ├── lib/ipc.ts            # thin wrapper over the Tauri command bridge
│   └── App.tsx
├── src-tauri/                # Rust side of Tauri
│   └── src/main.rs           # connects to the zonixd Unix socket, exposes
│                             # commands + emits events to the webview
└── package.json
```

### Why Tauri (vs. PyQt6/GTK4)

- Ships a small native binary (no bundled Chromium — uses the system webview),
  which keeps idle RAM low, matching the "lightweight" requirement.
- The Rust side can reuse `zonix-core` directly, so the IPC contract is shared
  and type-checked on both ends.
- Modern, fluid UI (React) without writing C++/Qt boilerplate.

The Tauri Rust process does **not** re-implement audio logic. It is a thin
client that opens the same `daemon.sock`, relays `Request`s from the webview,
and forwards `Event`s (level meters, stream added/removed) back to React.

> Not scaffolded yet — Phase 4.
