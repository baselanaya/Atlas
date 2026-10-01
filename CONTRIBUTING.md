# Contributing to Atlas

Thanks for wanting to help! Issues and PRs are both welcome.

## The rules that never bend

1. **Never block a coding agent.** The relay gives the app 300 ms; silence means
   the terminal takes over. Any change to the hook path must keep that promise.
2. **Never write a user's config without a backup, a diff and a click.** The hook
   installers and the access-level writer all take dated backups and write
   atomically; new config writes must do the same.
3. **No telemetry, no accounts.** Network calls go only to services the user
   configured, with keys that live in the system keyring.
4. **No third-party art or audio.** The character is drawn in code and the sounds
   are synthesized — that is what lets this fork ship its own identity.

## Working on it

```bash
npm install
npm run dev            # the island in a plain browser (no Tauri needed)
npm run tauri dev      # the real thing, live-reloading
cargo test --workspace # the Rust side
npm run build          # typecheck + frontend build
```

`src/` is plain TypeScript with no framework; `src-tauri/` is Rust. The
per-agent hook installers live in `src-tauri/src/agents/` — if you add an agent,
follow the existing adapter shape and add a round-trip test in `hooks.rs`.

## Sending a PR

Small, focused PRs with a sentence of "why" beat big ones. If it touches the
character or the island's behavior, a screenshot or short clip helps a lot.
