<div align="center">

# Atlas

**A little ringed planet that lives at the top of your screen and watches your coding agents work.**

Approve permissions, set access levels, watch sessions, drop a file, chat — without leaving what you're doing.

![Linux](https://img.shields.io/badge/Linux-KDE%20%2F%20X11-1793D1?logo=linux)
![Windows 10/11](https://img.shields.io/badge/Windows-10%2F11-0078D4?logo=windows)
![Tauri 2](https://img.shields.io/badge/Tauri-2-FFC131?logo=tauri&logoColor=black)
![Rust](https://img.shields.io/badge/backend-Rust-000?logo=rust)
![License: MIT](https://img.shields.io/badge/license-MIT-green)

</div>

---

## What Atlas does

Atlas watches **Claude Code**, **ZCode** and **Codex** sessions and shows them in a small island at the top of your screen — one pill per agent, each with its own live state, steps and sounds.

- 🤖 **Every agent, one island** — each harness gets its own pill; concurrent sessions stay separated.
- ✅ **Approve from the island** — permission requests pop up with Allow / Deny, labeled with the agent and the exact command. If you don't answer in time, the terminal takes over — the agent is never blocked.
- 🔐 **Access levels** — flip any agent between *Ask*, *Auto* and *Root* (bypass prompts / full sandbox) with one click. Writes the agent's own config, dated backup first.
- 💬 **Chat** — a bubble that talks to any Anthropic-compatible endpoint: Anthropic itself, or GLM via Z.AI's coding-plan endpoint.
- 📎 **Drop a file** — Atlas swallows it and answers questions about it.
- 🎭 **A real character** — an ice-blue planet with a gold ring: blinks, follows your cursor, gets dizzy if you poke it too much, 28 synthesized sounds.
- 🔌 **Integrations** — optional pollers for GitHub, Vercel, Stripe, n8n, Resend, Notion, Cal.com. Off by default.
- 🔒 **Private by design** — no telemetry, no account. Keys live in your system keyring (KWallet / Secret Service on Linux, Credential Manager on Windows). The app only talks to services you configure.

## Install

Grab `Atlas-Linux.AppImage` or `Atlas-Linux_<version>.deb` (and the Windows setup) from [Releases](../../releases). On Linux, KDE with X11/XWayland is the best-tested home; the island needs X11 for self-positioning and the global cursor, so Atlas forces the X11 GDK backend (a Wayland-native path is on the roadmap).

## Build it yourself

Requirements: Rust (rustup), Node 20+, and on Linux `webkit2gtk-4.1`, `libayatana-appindicator`, `gtk3`, GStreamer plugins (base/good/libav) — on Arch: `sudo pacman -S webkit2gtk-4.1 libayatana-appindicator gtk3 gst-plugins-base gst-plugins-good gst-libav`.

```bash
git clone https://github.com/baselanaya/Atlas.git
cd Atlas
npm install
npm run tauri dev      # live-reloading development build
npm run pack           # installers land in release/
```

```
src/            island front end (TypeScript, no framework)
  atlas/        the character, drawn in Canvas 2D
  island/       state machine, hook routing, integrations
  views/        every island view
  settings/     the settings window
src-tauri/      Rust backend: window, transports, chat, pollers, access levels
hook/           atlas-hook, the relay the agents invoke on every hook event
sounds/         the 28 synthesized WAVs
```

## How it works

Each agent's hook config points at a tiny relay, `atlas-hook`. On every hook event the relay reads the JSON from stdin, tags it with the agent's name, and forwards it to the app — over a Unix domain socket on Linux (`$XDG_RUNTIME_DIR/atlas.sock`, mode 0600), a named pipe on Windows. `PermissionRequest` is the only event that waits: the island shows the card, a human clicks, the decision goes back on the same connection.

The installer never touches your other hooks: it reads the config, takes a dated backup, shows you a diff, and writes only after you click. Uninstall removes only Atlas's entries — including any left by the Coucou-era binary this project was forked from.

## Credits & license

Atlas is a fork of [Coucou](https://github.com/Louis-CFM/coucou) by Louis Raillé — the Linux port, multi-agent support, access levels and the GLM chat path are this project's own work, built on his original design. The code is MIT (see [LICENSE](LICENSE)); the original Copyright (c) 2025 Louis Raillé notice is retained as the license requires.

The Atlas name, the ringed-planet character, the icon and the 28 synthesized sounds are original to this project.

## Contributing

Issues and PRs welcome — see [CONTRIBUTING.md](CONTRIBUTING.md). The rules that matter most: never block a coding agent, never write a config without a shown diff and a click, no telemetry, secrets only in the keyring.
