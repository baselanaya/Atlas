# Changelog

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
