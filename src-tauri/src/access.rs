// Access levels for the coding agents — how much rope each harness gets.
//
// Ask is the safe default every harness ships with; Auto answers the routine
// stuff itself; Root takes the questions away entirely. Changing a level
// rewrites the agent's own config (a dated backup first, an atomic rename
// last, exactly like the hook installer) and applies to sessions started
// after the change.
//
// Claude Code:  ~/.claude/settings.json → permissions.defaultMode
// Codex:        ~/.codex/config.toml    → approval_policy + sandbox_mode
// zcode:        no persistent key — its permission mode is a per-session
//               picker in the client, so the island says so instead of
//               writing a key nothing reads.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AccessLevel {
    Ask,
    Auto,
    Root,
}

impl AccessLevel {
    pub fn parse(s: &str) -> Option<AccessLevel> {
        match s {
            "ask" => Some(AccessLevel::Ask),
            "auto" => Some(AccessLevel::Auto),
            "root" => Some(AccessLevel::Root),
            _ => None,
        }
    }

}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessInfo {
    pub agent: String,
    pub level: Option<AccessLevel>,
    /// False when the harness has no persistent access setting (zcode).
    pub configurable: bool,
    pub note: String,
}

fn home() -> PathBuf {
    #[cfg(windows)]
    let key = "USERPROFILE";
    #[cfg(unix)]
    let key = "HOME";
    std::env::var_os(key)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn claude_settings() -> PathBuf {
    home().join(".claude").join("settings.json")
}

fn codex_config() -> PathBuf {
    home().join(".codex").join("config.toml")
}

/// ── reading ─────────────────────────────────────────────────────────────────

fn claude_level() -> Result<AccessLevel, String> {
    let path = claude_settings();
    let bytes = std::fs::read(&path).map_err(|e| format!("Can't read {}: {e}", path.display()))?;
    let value = crate::hooks::parse_settings(&bytes, &path.display().to_string())?;
    match value
        .get("permissions")
        .and_then(|p| p.get("defaultMode"))
        .and_then(Value::as_str)
        .unwrap_or("default")
    {
        "bypassPermissions" => Ok(AccessLevel::Root),
        "acceptEdits" => Ok(AccessLevel::Auto),
        _ => Ok(AccessLevel::Ask),
    }
}

fn codex_level() -> Result<AccessLevel, String> {
    let path = codex_config();
    let text = std::fs::read_to_string(&path).map_err(|e| format!("Can't read {}: {e}", path.display()))?;
    let doc = text.parse::<toml_edit::DocumentMut>().map_err(|e| format!("{path:?} isn't valid TOML: {e}"))?;
    let sandbox = doc.get("sandbox_mode").and_then(|v| v.as_str()).unwrap_or("");
    let approval = doc.get("approval_policy").and_then(|v| v.as_str()).unwrap_or("");
    Ok(match (sandbox, approval) {
        ("danger-full-access", _) => AccessLevel::Root,
        (_, "never") => AccessLevel::Root,
        (_, "on-failure") => AccessLevel::Auto,
        _ => AccessLevel::Ask,
    })
}

pub fn info(agent: &str) -> AccessInfo {
    match agent {
        "claude" => AccessInfo {
            agent: agent.into(),
            level: claude_level().ok(),
            configurable: true,
            note: "permissions.defaultMode in ~/.claude/settings.json".into(),
        },
        "codex" => AccessInfo {
            agent: agent.into(),
            level: codex_level().ok(),
            configurable: true,
            note: "approval_policy and sandbox_mode in ~/.codex/config.toml".into(),
        },
        _ => AccessInfo {
            agent: agent.into(),
            level: None,
            configurable: false,
            note: "zcode's permission mode is chosen per session in its own window".into(),
        },
    }
}

/// ── writing ─────────────────────────────────────────────────────────────────

/// Dated backup beside the file, then write beside and rename over it: a crash
/// or a full disk leaves the original config intact rather than half of it.
fn write_with_backup(path: &Path, contents: &[u8]) -> Result<String, String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let t = crate::log::LocalTime::now();
    let stamp = format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        t.year, t.month, t.day, t.hour, t.minute, t.second
    );
    let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
    let backup = path.with_file_name(format!("{name}.bak-{stamp}"));
    if path.exists() {
        std::fs::copy(path, &backup).map_err(|e| format!("backup failed: {e}"))?;
    }
    let temp = path.with_file_name(format!("{name}.atlas-{}", std::process::id()));
    std::fs::write(&temp, contents).map_err(|e| format!("write failed: {e}"))?;
    if let Err(err) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("write failed: {err}"));
    }
    Ok(backup.to_string_lossy().to_string())
}

fn set_claude(level: AccessLevel) -> Result<String, String> {
    let path = claude_settings();
    let mode = match level {
        AccessLevel::Ask => "default",
        AccessLevel::Auto => "acceptEdits",
        AccessLevel::Root => "bypassPermissions",
    };
    let bytes = std::fs::read(&path).map_err(|e| format!("Can't read {}: {e}", path.display()))?;
    let mut value = crate::hooks::parse_settings(&bytes, &path.display().to_string())?;
    let mut root = value.as_object_mut().unwrap().clone();
    let mut permissions = root
        .get("permissions")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    permissions.insert("defaultMode".into(), json!(mode));
    root.insert("permissions".into(), Value::Object(permissions));
    value = Value::Object(root);
    let mut text = serde_json::to_string_pretty(&value).unwrap_or_default();
    text.push('\n');
    write_with_backup(&path, text.as_bytes())
}

fn set_codex(level: AccessLevel) -> Result<String, String> {
    let path = codex_config();
    let text = std::fs::read_to_string(&path).map_err(|e| format!("Can't read {}: {e}", path.display()))?;
    let mut doc = text
        .parse::<toml_edit::DocumentMut>()
        .map_err(|e| format!("{path:?} isn't valid TOML: {e}"))?;
    let (approval, sandbox) = match level {
        AccessLevel::Ask => ("on-request", "workspace-write"),
        AccessLevel::Auto => ("on-failure", "workspace-write"),
        AccessLevel::Root => ("never", "danger-full-access"),
    };
    // Top-level keys: toml_edit writes them before the first table, keeping
    // every comment and section exactly as the user left them.
    doc["approval_policy"] = toml_edit::value(approval);
    doc["sandbox_mode"] = toml_edit::value(sandbox);
    write_with_backup(&path, doc.to_string().as_bytes())
}

/// Sets the level and returns the backup path. Only configurable agents.
pub fn set(agent: &str, level: AccessLevel) -> Result<String, String> {
    match agent {
        "claude" => set_claude(level),
        "codex" => set_codex(level),
        _ => Err(format!("{agent} has no persistent access setting")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Everything filesystem-shaped in one test: it points the home directory
    /// at a temp directory, and that is process-wide.
    #[test]
    fn claude_roundtrip_preserves_everything_else() {
        let _home = crate::TEST_HOME_LOCK.lock().unwrap();
        let tmp = std::env::temp_dir().join(format!("atlas-access-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join(".claude")).unwrap();
        #[cfg(windows)]
        std::env::set_var("USERPROFILE", &tmp);
        #[cfg(unix)]
        std::env::set_var("HOME", &tmp);

        let original = r#"{"theme":"dark","hooks":{"PreToolUse":[{"hooks":[{"type":"command","command":"keep.exe"}]}]}}"#;
        let path = claude_settings();
        std::fs::write(&path, original).unwrap();

        assert_eq!(claude_level().unwrap(), AccessLevel::Ask);
        let backup = set("claude", AccessLevel::Root).unwrap();
        assert!(backup.contains(".bak-"));
        assert_eq!(claude_level().unwrap(), AccessLevel::Root);

        let after: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(after["theme"], "dark");
        assert_eq!(
            after["hooks"]["PreToolUse"][0]["hooks"][0]["command"], "keep.exe",
            "the hook installer's entries must survive an access change"
        );
        assert_eq!(after["permissions"]["defaultMode"], "bypassPermissions");

        set("claude", AccessLevel::Ask).unwrap();
        assert_eq!(claude_level().unwrap(), AccessLevel::Ask);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn codex_roundtrip_preserves_comments_and_tables() {
        let _home = crate::TEST_HOME_LOCK.lock().unwrap();
        let tmp = std::env::temp_dir().join(format!("atlas-access-toml-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join(".codex")).unwrap();
        #[cfg(windows)]
        std::env::set_var("USERPROFILE", &tmp);
        #[cfg(unix)]
        std::env::set_var("HOME", &tmp);

        let original = "# my codex\nmodel = \"gpt-6-astra\"\nmodel_reasoning_effort = \"medium\"\n\n[projects.\"/home/reverb\"]\ntrust_level = \"trusted\"\n";
        let path = codex_config();
        std::fs::write(&path, original).unwrap();

        assert_eq!(codex_level().unwrap(), AccessLevel::Ask);
        set("codex", AccessLevel::Root).unwrap();
        assert_eq!(codex_level().unwrap(), AccessLevel::Root);

        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("# my codex"), "comments survive");
        assert!(after.contains("model = \"gpt-6-astra\""), "scalars survive");
        assert!(after.contains("[projects.\"/home/reverb\"]"), "tables survive");
        assert!(after.contains("approval_policy = \"never\""));
        assert!(after.contains("sandbox_mode = \"danger-full-access\""));

        // Root keys must stay top-level, before the first [table].
        let tables_at = after.find("[projects").unwrap();
        let approval_at = after.find("approval_policy").unwrap();
        assert!(approval_at < tables_at, "top-level keys must precede tables");

        set("codex", AccessLevel::Ask).unwrap();
        assert_eq!(codex_level().unwrap(), AccessLevel::Ask);

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
