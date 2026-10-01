// Claude Code — the original integration.
//
// `~/.claude/settings.json`, entries under `hooks.<Event>`, each a matcher
// group holding one command hook. The command is the quoted relay path in
// forward slashes plus the event name: Claude Code runs hook commands through
// a shell (Git Bash on Windows), and anything with PowerShell or cmd in it
// breaks.

use std::path::PathBuf;

use serde_json::{json, Map, Value};

use super::{is_relay_command, AgentHooks};

/// Every event the island reacts to, with the hook timeout in seconds.
/// PermissionRequest waits for a human, so it gets the decision timeout + 10 s.
pub const HOOK_EVENTS: &[(&str, u64)] = &[
    ("SessionStart", 10),
    ("SessionEnd", 10),
    ("UserPromptSubmit", 10),
    ("PreToolUse", 10),
    ("PostToolUse", 10),
    ("PostToolUseFailure", 10),
    ("PermissionRequest", 120),
    ("Notification", 10),
    ("Stop", 10),
    ("StopFailure", 10),
    ("SubagentStart", 10),
    ("SubagentStop", 10),
];

pub struct Claude;

pub static AGENT: Claude = Claude;

fn home() -> PathBuf {
    #[cfg(windows)]
    let key = "USERPROFILE";
    #[cfg(unix)]
    let key = "HOME";
    std::env::var_os(key)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn settings_path() -> PathBuf {
    home().join(".claude").join("settings.json")
}

fn hook_command(event: &str) -> String {
    let exe = crate::settings::hook_exe_path().to_string_lossy().replace('\\', "/");
    format!("\"{exe}\" {event}")
}

fn entry_is_ours(entry: &Value) -> bool {
    entry
        .get("hooks")
        .and_then(Value::as_array)
        .map(|hooks| {
            hooks.iter().any(|h| {
                h.get("command")
                    .and_then(Value::as_str)
                    .map(is_relay_command)
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn merged(existing: &Value) -> Value {
    let mut root = existing.as_object().cloned().unwrap_or_default();
    let mut hooks = root
        .get("hooks")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_else(Map::new);

    for (event, timeout) in HOOK_EVENTS {
        let mut list = hooks
            .get(*event)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        list.retain(|entry| !entry_is_ours(entry));
        list.push(json!({
            "hooks": [{
                "type": "command",
                "command": hook_command(event),
                "timeout": timeout,
            }]
        }));
        hooks.insert((*event).to_string(), Value::Array(list));
    }

    root.insert("hooks".into(), Value::Object(hooks));
    Value::Object(root)
}

fn without_ours(existing: &Value) -> Value {
    let mut root = existing.as_object().cloned().unwrap_or_default();
    let Some(hooks) = root.get("hooks").and_then(Value::as_object).cloned() else {
        return Value::Object(root);
    };
    let mut out = Map::new();
    for (event, value) in hooks {
        match value.as_array() {
            Some(list) => {
                let kept: Vec<Value> =
                    list.iter().filter(|e| !entry_is_ours(e)).cloned().collect();
                if !kept.is_empty() {
                    out.insert(event, Value::Array(kept));
                }
            }
            None => {
                out.insert(event, value);
            }
        }
    }
    if out.is_empty() {
        root.remove("hooks");
    } else {
        root.insert("hooks".into(), Value::Object(out));
    }
    Value::Object(root)
}

impl AgentHooks for Claude {
    fn id(&self) -> &'static str {
        "claude"
    }

    fn name(&self) -> &'static str {
        "Claude Code"
    }

    fn blurb(&self) -> &'static str {
        "Live sessions and approvals in the island"
    }

    fn settings_path(&self) -> PathBuf {
        settings_path()
    }

    fn read(&self) -> Result<Value, String> {
        let path = settings_path();
        match std::fs::read(&path) {
            Ok(bytes) => crate::hooks::parse_settings(&bytes, &path.display().to_string()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
            // A lock, a permission problem, a bad drive: all of them mean we do
            // not know what is in there, and not knowing is not the same as empty.
            Err(err) => Err(format!("Can't read {}: {err}", path.display())),
        }
    }

    fn rewrite(&self, current: &Value, install: bool) -> Value {
        if install {
            merged(current)
        } else {
            without_ours(current)
        }
    }

    fn installed(&self, current: &Value) -> bool {
        current
            .get("hooks")
            .and_then(Value::as_object)
            .map(|hooks| {
                hooks
                    .values()
                    .filter_map(Value::as_array)
                    .flatten()
                    .any(entry_is_ours)
            })
            .unwrap_or(false)
    }
}
