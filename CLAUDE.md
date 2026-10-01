# Atlas — guide for AI coding agents

Atlas is a Tauri 2 app (Rust + TypeScript, no frontend framework): a little ringed
planet lives at the top of the screen, shows Claude Code / ZCode / Codex sessions,
lets the user approve permissions, set access levels, chat and drop files.

## Where things are
- `src/` — all TypeScript. `src/atlas/` — the character, drawn in Canvas 2D.
  `src/island/` — state machine, hook routing, integrations. `src/views/` — island
  views. `src/settings/` — the settings window. `sounds/` — the 28 WAVs.
- `src-tauri/src/agents/` — one installer per coding agent (claude, zcode, codex);
  `src-tauri/src/access.rs` — access levels; `src-tauri/src/pipe.rs` — the relay
  transport (Unix socket on Linux, named pipe on Windows); `hook/` — atlas-hook,
  the relay binary itself.
- `src-tauri/tauri.{linux,windows}.conf.json` — per-platform bundle overlays.

## Build
```
npm install
cargo test --workspace            # 12 tests
npm run tauri dev                 # dev build
npm run pack                      # installers in release/
```
A bare `cargo build --release` produces a dev-mode binary without the embedded
frontend — build through the tauri CLI (`npm run tauri build`) or add
`--features tauri/custom-protocol`.

## Rules
- Never block a coding agent: if the app doesn't answer a relay connection in
  300 ms, the relay exits 0 and the session carries on untouched. Only
  PermissionRequest waits, and silence means "let the terminal ask".
- Never write an agent's config without a dated backup, a shown diff and an
  explicit click. Uninstall removes only Atlas's entries.
- zcode supports exactly seven hook events and needs `hooks.enabled: true`;
  Codex entries are matcher groups (`{"hooks":[{"type":"command",…}]}`) and need
  one-time `/hooks` trust. The PermissionRequest decision JSON is the same
  `hookSpecificOutput.decision.behavior` shape for all three.
- On Linux the island runs under XWayland (`GDK_BACKEND=x11`, forced in main.rs)
  and its clickable area is an X11 input shape, not a click-through flag.
- Secrets live in the system keyring, never on disk. No telemetry; network calls
  only to services the user configured.
- The character is drawn in code and the sounds are synthesized — no third-party
  art or audio assets in this repo, and keep it that way (the fork's license
  depends on it).
