# Changelog

## 0.7.0 — 2026-10-02

- **MCP over stdio** — `atlas --mcp-stdio` bridges stdin/stdout JSON-RPC to
  the island's HTTP server, so stdio-only clients work too. Verified
  end-to-end with Codex as the client: `codex mcp add atlas -- atlas
  --mcp-stdio`, then "use the atlas_status tool" returns the live agent
  state. `ATLAS_MCP_PORT` overrides the forwarded port.
- docs/voicebox.md covers stdio registration.

## 0.6.0 — 2026-10-02

- **Token tracking** — real usage per agent per day, read from the agents'
  own session transcripts (Claude Code's per-message `usage`, Codex's
  cumulative `token_count`), shown in Settings → Stats and available to MCP
  clients as `atlas_tokens`. zcode writes no readable transcripts and reports
  nothing here — no estimates, only measurements.
- **Voicebox pairing guide** — docs/voicebox.md covers both directions:
  Voicebox as Atlas's voice, and Voicebox (or any MCP client) talking to
  Atlas's tool server.

## 0.5.0 — 2026-10-02

- **MCP-out** — Atlas is now a tool server: any local MCP client can call
  `atlas_status`, `atlas_pending`, `atlas_decide`, `atlas_speak` and
  `atlas_stats` over 127.0.0.1 (JSON-RPC, opt-in in Settings → MCP). Voice
  assistants and IDEs can finally *ask* Atlas what the agents are doing.
- **Dictation in the chat** — a mic button records and transcribes through
  Voicebox's Whisper endpoint straight into the input.
- **Ollama route** — the chat bubble can answer from a local Ollama
  (Settings → Chat → Answers); the model field is now a free-text combo.
- **Stats** — per-agent daily counters (sessions, tool calls, approvals,
  allow/deny decisions) in Settings → Stats and via `atlas_stats`.
- **System notifications** — approval requests raise a desktop notification
  (notify-send) even when the island is collapsed; toggleable.

## 0.4.0 — 2026-10-01

- **The island speaks** — Atlas integrates [Voicebox](https://github.com/jamiepine/voicebox)
  (local, open source): chat replies read aloud, permission requests and
  finished sessions announced, in any voice profile you've cloned. Detected
  automatically on its local API (127.0.0.1:17493); every control is a
  separate toggle in Settings → Voice, and nothing is spoken without the
  master switch. Bonus: Voicebox's own global dictation hotkey already types
  into Atlas's chat field — zero code needed.

## 0.3.0 — 2026-10-01

- **New icon** — a proper space-scene SVG (glowing planet, ring with its
  shadow, starfield), rasterized at every bundle size.
- **Native Wayland (experimental)** — when `gtk3-layer-shell` is installed,
  the island runs as a layer surface positioned by the compositor; XWayland
  remains the fallback (and `ATLAS_ISLAND=x11|wayland` forces either).
  Eye tracking pauses on native Wayland — a Wayland client can't see the
  cursor outside its own windows.
- **Chat routing** — the bubble can answer through a logged-in Codex or
  Claude Code instead of the API key (Settings → Chat → Answers), riding on
  the subscription you already pay for.
- **Relay protocol tests** — golden tests run the real `atlas-hook` binary
  against a mock app: wire format, agent stamping, and the byte-stable
  PermissionRequest decision JSON.
- **Emote synthesis** — zcode and Codex sessions now show the rate-limit and
  question faces, inferred from what their turns say (they have no
  Notification event of their own).
- **First-run onboarding** — a fresh install opens the settings window where
  the hooks live, so the island never starts life watching nothing.
- Issue templates, GitHub Discussions, localizable strings (`src/core/strings.ts`),
  AUR publishing guide (`packaging/README.md`).

## 0.2.1 — 2026-10-01

- English step labels in the session ticker (Runs / Reads / Edits / …) — the
  island no longer speaks the fork's original French.
- More tool names recognized (Codex's `Shell`, `apply_patch`, zcode's `Agent`,
  `Exit`).
- README screenshot, AUR packaging (`packaging/PKGBUILD`), CI builds on every
  push, installers attached to `v*` tags automatically.

## 0.2.0 — 2026-10-01

First release as **Atlas**, forked from [Coucou](https://github.com/Louis-CFM/coucou)
by Louis Raillé (MIT; his copyright notice is retained).

- **Linux support** — KDE/XWayland island with X11 input-shape click-through,
  Unix-socket relay transport, XDG paths, libsecret/KWallet keyring, deb +
  AppImage packaging. Windows support carried over (named pipe, Credential
  Manager, NSIS).
- **Three coding agents** — Claude Code, ZCode and Codex, one pill each, with
  per-agent sessions, steps and badges. Allow/Deny approvals from the island,
  labeled with the asking agent and the exact command.
- **Access levels** — Ask / Auto / Root per agent: one click rewrites the
  agent's own permission config (dated backup, atomic write), with legacy
  Coucou-era hook entries swept out on upgrade.
- **Chat on any Anthropic-compatible endpoint** — Anthropic or GLM via Z.AI's
  coding-plan endpoint; web search and fallbacks gate on the provider.
- **Original identity** — the ringed-planet character drawn in code, a
  generated icon, and 28 synthesized sounds. No third-party art or audio.

## Before the fork

See the upstream Coucou project for the macOS original and its history.
