// Atlas — app wiring and the commands the island calls.

mod access;
mod agents;
mod claude;
mod files;
mod hooks;
mod integrations;
mod island;
pub mod layershell;
mod mcp;
mod notify;
mod stats;
mod tokens;
mod log;
mod pipe;
mod secrets;
mod settings;
mod tray;
mod voice;
#[cfg(windows)]
mod win_user;

#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_autostart::{ManagerExt, MacosLauncher};

use claude::{Chat, ChatContext, ChatReply};
use files::DroppedFile;
use hooks::{HookPreview, HookStatus};
use island::{PollGate, ScreenInfo};
use pipe::Pending;
use settings::Settings;

/// Keeps spawned helpers from flashing a console window.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Home-directory-shaped tests (hooks installer, access levels) point $HOME at
/// temp directories; the lock keeps them from racing each other process-wide.
#[cfg(test)]
static TEST_HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub struct Shared {
    pub settings: Mutex<Settings>,
    pub gate: Arc<PollGate>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootInfo {
    settings: Settings,
    screen: ScreenInfo,
    version: String,
    hook_path: String,
}

#[tauri::command]
fn boot(app: AppHandle, shared: State<Shared>) -> BootInfo {
    let mut settings = shared.settings.lock().unwrap().clone();
    // The real state of each agent's config file wins over whatever we stored.
    for status in hooks::all_statuses() {
        match status.agent.as_str() {
            "claude" => settings.hooks_installed = status.installed,
            "zcode" => settings.hooks_installed_zcode = status.installed,
            "codex" => settings.hooks_installed_codex = status.installed,
            _ => {}
        }
    }
    let screen = island::screen_info(&app, &settings.screen);
    BootInfo {
        settings,
        screen,
        version: env!("CARGO_PKG_VERSION").to_string(),
        hook_path: settings::hook_exe_path().to_string_lossy().to_string(),
    }
}

#[tauri::command]
fn save_settings(app: AppHandle, shared: State<Shared>, settings: Settings) {
    let (screen_changed, autostart_changed) = {
        let mut current = shared.settings.lock().unwrap();
        let screen_changed = current.screen != settings.screen;
        let autostart_changed = current.autostart != settings.autostart;
        *current = settings.clone();
        (screen_changed, autostart_changed)
    };
    if let Err(err) = settings::save(&settings) {
        eprintln!("[atlas] could not save settings: {err}");
    }
    if autostart_changed {
        let manager = app.autolaunch();
        let result = if settings.autostart { manager.enable() } else { manager.disable() };
        if let Err(err) = result {
            eprintln!("[atlas] autostart: {err}");
        }
    }
    if screen_changed {
        let collapsed = shared.gate.collapsed.load(Ordering::Relaxed);
        island::apply_geometry(&app, &settings.screen, collapsed);
    }
    // Keep the other window in step (island ⇄ settings window).
    let _ = app.emit("settings-changed", settings);
}

/// Hidden island → shrink the window to the invisible wake strip and park the
/// cursor poll; anything else → full panel and 60 Hz polling.
#[tauri::command]
fn set_collapsed(app: AppHandle, shared: State<Shared>, collapsed: bool) {
    let pref = shared.settings.lock().unwrap().screen.clone();
    shared.gate.collapsed.store(collapsed, Ordering::Relaxed);
    island::apply_geometry(&app, &pref, collapsed);
    // The wake strip must always take the mouse, and a resize invalidates the flag.
    island::set_ignore_cursor(&app, false);
    shared.gate.forget_ignore_state();
    shared.gate.set_active(!collapsed);
}

/// The front end pushes the island shape; Rust decides click-through from it.
#[tauri::command]
fn set_island_rect(shared: State<Shared>, x: f64, y: f64, width: f64, height: f64) {
    shared.gate.set_rect(island::IslandRect { x, y, w: width, h: height });
}

#[tauri::command]
fn focus_window(app: AppHandle, focused: bool) {
    let Some(win) = island::window(&app) else { return };
    island::set_activating(&win, focused);
    if focused {
        let _ = win.set_focus();
    }
}

#[tauri::command]
fn reposition(app: AppHandle, shared: State<Shared>) {
    let pref = shared.settings.lock().unwrap().screen.clone();
    let collapsed = shared.gate.collapsed.load(Ordering::Relaxed);
    island::apply_geometry(&app, &pref, collapsed);
}

#[tauri::command]
fn open_url(url: String) {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return;
    }
    #[cfg(windows)]
    let mut cmd = {
        let mut cmd = Command::new("rundll32.exe");
        cmd.args(["url.dll,FileProtocolHandler", &url]);
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd
    };
    #[cfg(unix)]
    let mut cmd = {
        let mut cmd = Command::new("xdg-open");
        cmd.arg(&url);
        cmd
    };
    let _ = cmd.spawn();
}

/// "Open folder" for a session's working directory: the file manager on Linux
/// (xdg-open → Dolphin on KDE), VS Code when `code` is on PATH on Windows with
/// Explorer as the fallback. The island never launches an editor the user
/// didn't ask for.
#[tauri::command]
fn open_project(path: Option<String>) -> bool {
    #[cfg(unix)]
    {
        if let Some(p) = path.as_deref().filter(|p| !p.is_empty()) {
            if Command::new("xdg-open").arg(p).spawn().is_ok() {
                return true;
            }
        }
        return false;
    }
    #[cfg(windows)]
    {
        // No `cmd /C` anywhere near this. The path is a project folder chosen by
        // whoever is using the agents, and cmd would happily read `&`, `^` and `%`
        // in a folder name as syntax. Finding the launcher ourselves and handing
        // the path over as a separate argument keeps it a path.
        if let Some(code) = find_on_path("code") {
            let mut cmd = Command::new(code);
            if let Some(p) = path.as_deref().filter(|p| !p.is_empty()) {
                cmd.arg(p);
            }
            if cmd.creation_flags(CREATE_NO_WINDOW).spawn().is_ok() {
                return true;
            }
        }
        if let Some(p) = path.as_deref().filter(|p| !p.is_empty()) {
            let _ = Command::new("explorer").arg(p).spawn();
        }
        false
    }
}

/// Our own `which`: walks %PATH% against %PATHEXT% on Windows, checks the
/// executable bit on Unix — no shell involved.
/// Rust quotes arguments correctly for `.cmd`/`.bat` targets since 1.77, so
/// spawning `code.cmd` directly is safe.
#[cfg(windows)]
fn find_on_path(stem: &str) -> Option<std::path::PathBuf> {
    let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    let dirs = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&dirs) {
        for ext in exts.split(';').filter(|e| !e.is_empty()) {
            let candidate = dir.join(format!("{stem}{}", ext.to_lowercase()));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Kept for parity with the Windows launcher search; the Linux "Open folder"
/// goes straight through xdg-open.
#[cfg(unix)]
#[allow(dead_code)]
fn find_on_path(stem: &str) -> Option<std::path::PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let dirs = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&dirs) {
        let candidate = dir.join(stem);
        if let Ok(meta) = std::fs::metadata(&candidate) {
            if meta.is_file() && meta.permissions().mode() & 0o111 != 0 {
                return Some(candidate);
            }
        }
    }
    None
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

/// Tray → Pause. Paused means paused: the pollers stop talking to the network,
/// not just the island stopping showing things.
#[tauri::command]
fn set_paused(paused: bool) {
    integrations::set_paused(paused);
}

// ── MCP-out / stats ────────────────────────────────────────────────────────────

#[tauri::command]
fn mcp_status(app: AppHandle) -> mcp::McpStatus {
    mcp::status(&app)
}

#[tauri::command]
fn stats_snapshot(days: Option<usize>) -> serde_json::Value {
    stats::snapshot_json(days.unwrap_or(7).clamp(1, 60))
}

#[tauri::command]
fn tokens_snapshot(days: Option<usize>) -> serde_json::Value {
    serde_json::to_value(tokens::scan(days.unwrap_or(7).clamp(1, 60))).unwrap_or_default()
}

/// Voice → text through Voicebox's /transcribe (multipart audio upload).
#[tauri::command]
async fn voice_transcribe(audio_b64: String) -> Result<String, String> {
    crate::voice::transcribe(&audio_b64).await
}

// ── Voice (Voicebox) ───────────────────────────────────────────────────────────

#[tauri::command]
async fn voice_status() -> voice::VoiceStatus {
    voice::status().await
}

/// Spoken announcements (approvals, finishes), from the island's own events.
#[tauri::command]
async fn voice_speak(shared: State<'_, Shared>, text: String) -> Result<(), String> {
    let (enabled, profile) = {
        let settings = shared.settings.lock().unwrap();
        (settings.voice_enabled, settings.voice_profile.clone())
    };
    if !enabled {
        return Ok(());
    }
    voice::speak(&text, &profile).await;
    Ok(())
}

// ── Access levels ──────────────────────────────────────────────────────────────

#[tauri::command]
fn access_get(agent: String) -> access::AccessInfo {
    access::info(&agent)
}

/// Only ever called from an explicit click on a level.
#[tauri::command]
fn access_set(agent: String, level: String) -> Result<String, String> {
    let level = access::AccessLevel::parse(&level)
        .ok_or_else(|| format!("Unknown access level '{level}'"))?;
    access::set(&agent, level)
}

// ── Agent hooks (Claude Code, zcode, Codex) ────────────────────────────────────

#[tauri::command]
fn hooks_status(agent: String) -> Result<HookStatus, String> {
    hooks::status(&agent)
}

/// One entry per agent, for the settings window's boot.
#[tauri::command]
fn hooks_statuses() -> Vec<HookStatus> {
    hooks::all_statuses()
}

/// Returns the diff the user has to look at before anything is written.
#[tauri::command]
fn hooks_preview(agent: String, install: bool) -> Result<HookPreview, String> {
    hooks::preview(&agent, install)
}

/// Only ever called from an explicit click in the settings window.
#[tauri::command]
fn hooks_apply(
    app: AppHandle,
    shared: State<Shared>,
    agent: String,
    install: bool,
    fingerprint: String,
) -> Result<String, String> {
    // The fingerprint comes from the preview the user actually looked at, so a
    // config that changed in between is refused rather than overwritten.
    let backup = hooks::write(&agent, install, &fingerprint)?;
    let updated = {
        let mut current = shared.settings.lock().unwrap();
        match agent.as_str() {
            "claude" => current.hooks_installed = install,
            "zcode" => current.hooks_installed_zcode = install,
            "codex" => current.hooks_installed_codex = install,
            _ => {}
        }
        let _ = settings::save(&current);
        current.clone()
    };
    let _ = app.emit("settings-changed", updated);
    Ok(backup)
}

#[tauri::command]
fn approval_decision(app: AppHandle, request_id: String, decision: String) {
    pipe::answer(&app, &request_id, &decision);
}

/// The island has the card on screen, so the long wait for a human may begin.
/// Until this arrives the relay only waits a few hundred milliseconds, which is
/// what stops a paused or unresponsive island from freezing Claude Code.
#[tauri::command]
fn approval_ack(app: AppHandle, request_id: String) {
    pipe::acknowledge(&app, &request_id);
}

/// Nobody can act on this request — the island is paused, or another card is
/// already up. Claude Code falls back to asking in the terminal immediately.
#[tauri::command]
fn approval_decline(app: AppHandle, request_id: String) {
    pipe::decline(&app, &request_id);
}

// ── Chat, files and secrets ───────────────────────────────────────────────────

/// One chat turn. The API key and any file bytes stay on the Rust side.
#[tauri::command]
async fn chat_send(
    shared: State<'_, Shared>,
    chat: State<'_, Chat>,
    query: String,
    context: Option<ChatContext>,
) -> Result<ChatReply, String> {
    let (model, api_base, route) = {
        let settings = shared.settings.lock().unwrap();
        (settings.model.clone(), settings.api_base.clone(), settings.chat_route.clone())
    };
    let reply = claude::send(&chat, &model, &api_base, &route, query, context).await;
    if let Ok(text) = &reply {
        let (enabled, profile, speak_chat) = {
            let settings = shared.settings.lock().unwrap();
            (settings.voice_enabled, settings.voice_profile.clone(), settings.voice_speak_chat)
        };
        if enabled && speak_chat {
            voice::speak(&text.text, &profile).await;
        }
    }
    reply
}

#[tauri::command]
fn chat_reset(chat: State<Chat>) {
    chat.reset();
}

/// Copies a dropped file into the inbox and reports its name back.
#[tauri::command]
fn ingest_file(path: String) -> Result<DroppedFile, String> {
    files::ingest(&path)
}

/// The island may only ask whether a key exists — never read it.
#[tauri::command]
fn secret_present(key: String) -> bool {
    secrets::present(&key)
}

#[tauri::command]
fn secret_set(key: String, value: String) -> Result<(), String> {
    secrets::set(&key, &value)
}

#[tauri::command]
fn secret_clear(key: String) -> Result<(), String> {
    secrets::clear(&key)
}

/// Opens the configured n8n instance — the URL lives in the Credential Manager.
#[tauri::command]
fn open_n8n() {
    if let Some(url) = secrets::get("n8n-url") {
        open_url(url);
    }
}

/// Refresh buttons in the integration cards.
#[tauri::command]
async fn refresh_integration(app: AppHandle, id: String) {
    integrations::poll_once(app, &id).await;
}

/// Lets the island write to the same log as the Rust side.
#[tauri::command]
fn log_line(message: String) {
    log::line(format!("ui  {message}"));
}

// ── Settings window ───────────────────────────────────────────────────────────

/// WebView2 allows exactly one browser environment per app, and its options are
/// fixed by whichever webview is created first. Every window must therefore ask
/// for the *same* arguments as the island (see `additionalBrowserArgs` in
/// tauri.conf.json) — a mismatch makes the second window come up blank, with no
/// error anywhere.
const BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required";

/// In a dev build the pages are served by Vite, so the second window needs the
/// absolute dev URL; a bundled build resolves it inside the app bundle.
fn settings_page_url(app: &AppHandle) -> WebviewUrl {
    #[cfg(dev)]
    if let Some(mut base) = app.config().build.dev_url.clone() {
        base.set_path("/settings.html");
        return WebviewUrl::External(base);
    }
    let _ = app;
    WebviewUrl::App("settings.html".into())
}

/// The settings window is created hidden at launch and only ever shown and
/// hidden afterwards. A WebView2 window created later — on the main thread or
/// not — silently comes up blank in this app, so the window that works is the
/// one that exists before the island's webview does.
fn create_settings_window(app: &AppHandle) {
    let url = settings_page_url(app);
    match WebviewWindowBuilder::new(app, "settings", url)
        .additional_browser_args(BROWSER_ARGS)
        .title("Settings — Atlas")
        .inner_size(560.0, 680.0)
        .min_inner_size(460.0, 480.0)
        .resizable(true)
        .visible(false)
        .center()
        .build()
    {
        Ok(win) => {
            // Closing it must only hide it, or it could never be reopened.
            let hidden = win.clone();
            win.on_window_event(move |event| {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = hidden.hide();
                }
            });
        }
        Err(err) => log::line(format!("settings window failed: {err}")),
    }
}

pub fn show_settings_window(app: &AppHandle) {
    let Some(win) = app.get_webview_window("settings") else {
        log::line("settings window missing");
        return;
    };
    let _ = win.unminimize();
    let _ = win.show();
    let _ = win.set_focus();
}

#[tauri::command]
fn open_settings_window(app: AppHandle) {
    show_settings_window(&app);
}

pub fn run() {
    let loaded = settings::load();
    // A fresh install has never saved settings: open the window where the
    // hooks get installed, so the island isn't a pet that watches nothing.
    let first_run = !settings::config_exists();
    let gate = Arc::new(PollGate::new());

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            let _ = app.emit_to(island::WINDOW_LABEL, "tray", "open".to_string());
        }))
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))
        .manage(Shared {
            settings: Mutex::new(loaded.clone()),
            gate: gate.clone(),
        })
        .manage(Pending::default())
        .manage(Chat::default())
        .invoke_handler(tauri::generate_handler![
            boot,
            save_settings,
            set_collapsed,
            set_island_rect,
            focus_window,
            reposition,
            open_url,
            open_project,
            quit_app,
            access_get,
            access_set,
            voice_status,
            voice_speak,
            voice_transcribe,
            mcp_status,
            stats_snapshot,
            tokens_snapshot,
            hooks_status,
            hooks_statuses,
            hooks_preview,
            hooks_apply,
            approval_decision,
            approval_ack,
            approval_decline,
            log_line,
            chat_send,
            chat_reset,
            ingest_file,
            secret_present,
            secret_set,
            secret_clear,
            refresh_integration,
            open_n8n,
            open_settings_window,
            set_paused,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            tray::build(&handle)?;
            // Before the island: see create_settings_window.
            create_settings_window(&handle);

            // Native Wayland: the compositor places a layer surface; the
            // margin centers it on the primary output.
            #[cfg(target_os = "linux")]
            if std::env::var("GDK_BACKEND").as_deref() == Ok("wayland") {
                if let Some(gtk_win) = layershell::island_window() {
                    let scale = island::screen_info(&handle, "primary").scale;
                    let width = island::screen_info(&handle, "primary").width;
                    let margin = ((width - island::PANEL_W) / 2.0).round() as i32;
                    let ok = layershell::try_init(
                        gtk_win,
                        margin,
                        (island::PANEL_W * scale).round() as i32,
                        (island::PANEL_H * scale).round() as i32,
                    );
                    if ok {
                        log::line(format!(
                            "island: native Wayland layer surface (margin {margin})"
                        ));
                    }
                }
            }

            if let Some(win) = island::window(&handle) {
                island::make_non_activating(&win);
                // GTK drops position requests made while the window is still
                // unmapped — the WM centers it on first show instead — so the
                // island is placed after it is on screen.
                let _ = win.show();
                island::apply_geometry(&handle, &loaded.screen, false);
            }
            // And once more after the WM has had its say (Linux placement
            // policies override pre-map positions; see pin_after_show).
            #[cfg(unix)]
            island::pin_after_show(handle.clone());
            gate.collapsed.store(false, Ordering::Relaxed);
            gate.set_active(true);
            island::spawn_cursor_poll(handle.clone(), gate.clone());

            log::line(format!("--- Atlas {} started ---", env!("CARGO_PKG_VERSION")));
            hooks::ensure_hook_exe(&handle);
            pipe::start(handle.clone());
            integrations::start(handle.clone());
            if first_run {
                crate::show_settings_window(&handle);
            }
            mcp::start_if_enabled(&handle);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Atlas");
}
