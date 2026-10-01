# Upstream PR brief — offer the Linux port to Coucou

Not submitted: this is the ready-to-send text for a PR from this fork to
`Louis-CFM/coucou`, plus the branch recipe. Sending it is the maintainer's call.

## Branch recipe

The PR should carry the *substance* of the Linux port under the original
Coucou naming (the brand, character and sounds stay his). From a clone of the
upstream repo:

```bash
git clone https://github.com/Louis-CFM/coucou.git upstream-coucou
cd upstream-coucou/windows
# apply the Atlas diff, reverted to Coucou naming:
#   atlas-hook → coucou-hook, atlas.sock → coucou.sock, atlas_lib → coucou_lib,
#   com.baselanaya.atlas → fr.louisraille.coucou, ~/.config|state/atlas → Coucou
git checkout -b linux-port
git commit -am "Linux support: X11 island, Unix-socket relay, XDG paths, libsecret keyring, deb + AppImage"
git push
```

The pieces worth upstreaming, in order of value: `src-tauri/src/agents/`
(three-agent hooks), the X11 input-shape click-through in `island.rs`, the
Unix-socket transport, the platform config overlays, and `pack.mjs`.

## PR body draft

> ## Linux support for the Windows (Tauri) app
>
> This ports the Tauri app to Linux and generalizes it from "watch Claude
> Code" to "watch coding agents" — Claude Code, plus any Claude-Code-compatible
> harness (verified with zcode and OpenAI Codex via their hooks configs).
>
> - **Transport**: the relay speaks to a Unix domain socket
>   (`$XDG_RUNTIME_DIR/coucou.sock`, 0600) alongside the Windows named pipe —
>   same wire format, shared handler.
> - **Island on X11**: runs under XWayland (`GDK_BACKEND=x11`) since Wayland
>   clients can't self-position or query the global cursor; the clickable area
>   is an X11 input shape (XShapeCombineRectangles) rather than a toggled
>   click-through flag, which avoids a GTK input-shape quirk where the window
>   goes dead to clicks.
> - **Paths/secrets**: XDG config/state dirs, libsecret (KWallet/gnome-keyring)
>   via keyring's sync-secret-service, xdg-open.
> - **Per-agent installers** (`src-tauri/src/agents/`): the Claude Code
>   settings.json installer is refactored onto a shared backup/diff/fingerprint
>   core, with adapters for zcode (`~/.zcode/cli/config.json`,
>   `hooks.events`, seven supported events, `enabled: true`) and Codex
>   (`~/.codex/hooks.json`, matcher-group entries, one-time `/hooks` trust).
>   The PermissionRequest decision JSON is identical for all three.
> - **Packaging**: per-platform tauri config overlays (nsis / deb+appimage) and
>   a platform-aware pack script; CI builds both OSes.
>
> 12 unit tests + 3 relay protocol tests cover the config merges (round-trips
> preserving foreign hooks, plugins and TOML comments) and the wire format.
> Developed and daily-driven on KDE Plasma / Wayland (CachyOS).
