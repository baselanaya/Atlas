// Preferences, stored as plain JSON: %APPDATA%\Atlas\settings.json on Windows,
// ~/.config/atlas/settings.json on Linux.
// No secret ever lands here — API keys live in the Credential Manager on Windows
// and in the Secret Service (KWallet / gnome-keyring) on Linux.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub sound_enabled: bool,
    pub sound_volume: f64,
    pub auto_close_interval: f64,
    pub absence_interval: f64,
    pub active_integrations: Vec<String>,
    /// "primary" = the main display, "cursor" = whichever display the mouse is on.
    pub screen: String,
    pub autostart: bool,
    /// Hooks installed in ~/.claude/settings.json.
    pub hooks_installed: bool,
    /// Hooks installed in ~/.zcode/cli/config.json.
    #[serde(default)]
    pub hooks_installed_zcode: bool,
    /// Hooks installed in ~/.codex/hooks.json.
    #[serde(default)]
    pub hooks_installed_codex: bool,
    /// Claude model used by the chat. Changeable in the settings window.
    /// Defaulted explicitly so a settings.json written by an older build still loads.
    #[serde(default = "default_model")]
    pub model: String,
    /// Where the chat's Messages API lives. Anything Anthropic-compatible works:
    /// the default is Anthropic itself; Z.AI's GLM endpoint
    /// (https://api.z.ai/api/anthropic) serves glm-5.3 and friends from there.
    #[serde(default = "default_api_base")]
    pub api_base: String,
    /// Who answers the bubble: the API above, or a logged-in CLI agent —
    /// "api" | "codex" | "claude". CLI routes ride on the user's own
    /// subscription instead of an API key.
    #[serde(default = "default_chat_route")]
    pub chat_route: String,
    /// The island speaks through Voicebox when the studio is running.
    #[serde(default)]
    pub voice_enabled: bool,
    /// Voice profile by name; empty = Voicebox's default.
    #[serde(default)]
    pub voice_profile: String,
    /// Read chat replies out loud.
    #[serde(default = "default_true")]
    pub voice_speak_chat: bool,
    /// Announce permission requests and finished sessions.
    #[serde(default = "default_true")]
    pub voice_speak_events: bool,
    /// Where Voicebox's bind mount writes finished audio on the host; the
    /// island plays from here because the container has no sound card.
    #[serde(default)]
    pub voice_output_dir: String,
    /// Atlas as an MCP server (127.0.0.1 only) — opt-in, like every service.
    #[serde(default)]
    pub mcp_enabled: bool,
    #[serde(default = "default_mcp_port")]
    pub mcp_port: u16,
    /// System notifications for approvals and errors.
    #[serde(default = "default_true")]
    pub notify_enabled: bool,
    /// Agents the user switched off: their pills hide, their events are
    /// ignored (permission requests fall straight back to the terminal),
    /// their hooks stay installed until explicitly uninstalled.
    #[serde(default)]
    pub disabled_agents: Vec<String>,
}

fn default_mcp_port() -> u16 {
    crate::mcp::DEFAULT_PORT
}

fn default_true() -> bool {
    true
}

fn default_chat_route() -> String {
    "api".to_string()
}

fn default_model() -> String {
    crate::claude::DEFAULT_MODEL.to_string()
}

fn default_api_base() -> String {
    crate::claude::DEFAULT_API_BASE.to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            sound_enabled: true,
            sound_volume: 0.12,
            auto_close_interval: 15.0,
            absence_interval: 180.0,
            active_integrations: vec![
                "integration_resend".into(),
                "integration_n8n".into(),
                "integration_vercel".into(),
                "integration_github".into(),
            ],
            screen: "primary".into(),
            autostart: false,
            hooks_installed: false,
            hooks_installed_zcode: false,
            hooks_installed_codex: false,
            model: default_model(),
            api_base: default_api_base(),
            chat_route: default_chat_route(),
            voice_enabled: false,
            voice_profile: String::new(),
            voice_speak_chat: true,
            voice_speak_events: true,
            voice_output_dir: String::new(),
            mcp_enabled: false,
            mcp_port: default_mcp_port(),
            notify_enabled: true,
            disabled_agents: Vec::new(),
        }
    }
}

/// Directory name under the base: %APPDATA%\Atlas on Windows,
/// ~/.config/atlas on Linux (XDG names are lowercase by convention).
#[cfg(windows)]
const DIR_NAME: &str = "Atlas";
#[cfg(unix)]
const DIR_NAME: &str = "atlas";

/// %APPDATA%\Atlas — XDG_CONFIG_HOME (~/.config) + atlas on Linux.
pub fn config_dir() -> PathBuf {
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    #[cfg(unix)]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home_dir().join(".config"));
    base.join(DIR_NAME)
}

/// %LOCALAPPDATA%\Atlas — hook binary, log, dropped-file inbox.
/// XDG_STATE_HOME (~/.local/state) + atlas on Linux.
pub fn local_dir() -> PathBuf {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    #[cfg(unix)]
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home_dir().join(".local").join("state"));
    base.join(DIR_NAME)
}

/// $HOME on Unix — the counterpart of USERPROFILE, kept next to its uses.
#[cfg(unix)]
fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// The relay the hook settings point at: `atlas-hook.exe` on Windows,
/// `atlas-hook` everywhere else.
pub const HOOK_EXE_NAME: &str = if cfg!(windows) { "atlas-hook.exe" } else { "atlas-hook" };

pub fn hook_exe_path() -> PathBuf {
    local_dir().join("bin").join(HOOK_EXE_NAME)
}

/// Where the app listens for atlas-hook on Unix: a Unix domain socket.
/// $XDG_RUNTIME_DIR is per-user (0700, /run/user/<uid>), so no account can ever
/// reach another account's socket — the reason the pipe name carries the SID on
/// Windows. Must match socket_path() in hook/src/unix.rs exactly.
#[cfg(unix)]
pub fn socket_path() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        let dir = PathBuf::from(dir);
        if dir.is_dir() {
            return dir.join("atlas.sock");
        }
    }
    local_dir().join("runtime").join("atlas.sock")
}

fn settings_path() -> PathBuf {
    config_dir().join("settings.json")
}

/// Whether the user's settings file exists — the first-run signal.
pub fn config_exists() -> bool {
    settings_path().exists()
}

pub fn load() -> Settings {
    match std::fs::read(settings_path()) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => Settings::default(),
    }
}

pub fn save(settings: &Settings) -> std::io::Result<()> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)?;
    let json = serde_json::to_vec_pretty(settings)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(settings_path(), json)
}
