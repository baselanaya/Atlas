// One installer per coding agent. Each adapter knows where its harness keeps
// its hook configuration and what shape the entries take; everything risky —
// backups, fingerprints, diffs, atomic writes — lives in hooks.rs and is
// identical for all of them.
//
// Claude Code:  ~/.claude/settings.json    hooks.<Event>[].hooks[].command
// zcode:        ~/.zcode/cli/config.json   hooks.events.<Event>[].hooks[] (process)
// Codex:        ~/.codex/hooks.json        hooks.<Event>[].command
//
// All three relay through the same atlas-hook binary, tagged --agent <id>, and
// all three answer PermissionRequest with the same hookSpecificOutput JSON.

mod claude;
mod codex;
mod zcode;

use std::path::PathBuf;

use serde_json::Value;

/// Identifies an Atlas entry inside any of the three configs: the command (or
/// process command) always contains the relay's path.
pub const MARKER: &str = "atlas-hook";

/// True for any relay command we ever wrote — Atlas's own, or the Coucou-era
/// one this project was forked from, so an upgrade cleans up after its
/// predecessor instead of leaving dead entries in the agents' configs.
pub fn is_relay_command(cmd: &str) -> bool {
    cmd.contains(MARKER) || cmd.contains("coucou-hook")
}

/// The agents Atlas can watch, in the order the settings window shows them.
pub fn ids() -> [&'static str; 3] {
    ["claude", "zcode", "codex"]
}

pub fn by_id(id: &str) -> Option<&'static dyn AgentHooks> {
    match id {
        "claude" => Some(&claude::AGENT),
        "zcode" => Some(&zcode::AGENT),
        "codex" => Some(&codex::AGENT),
        _ => None,
    }
}

pub trait AgentHooks {
    fn id(&self) -> &'static str;
    /// Human name for the settings window.
    fn name(&self) -> &'static str;
    /// One line the settings window shows under the name.
    fn blurb(&self) -> &'static str;
    fn settings_path(&self) -> PathBuf;
    /// Reads the config. Only "file missing" means "start from nothing".
    fn read(&self) -> Result<Value, String>;
    /// The config with Atlas's entries added (`install`) or only ours removed.
    /// Everything else — other tools' hooks, plugins, preferences — survives.
    fn rewrite(&self, current: &Value, install: bool) -> Value;
    /// True when any of our entries is present.
    fn installed(&self, current: &Value) -> bool;
}
