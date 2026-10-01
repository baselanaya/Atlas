// Codex — `~/.codex/hooks.json`, entries under `hooks.<Event>`, each a matcher
// group holding typed handlers ({"type": "command", …}) with a timeout in
// seconds — the same nesting Claude Code uses, validated by
// codex-rs/config/src/hook_config.rs (MatcherGroup::hooks → HookHandlerConfig).
//
// The event set is close to Claude Code's minus PostToolUseFailure, StopFailure
// and Notification, which Codex does not have; the island simply never sees
// those from a Codex session. Codex waits synchronously for command hooks
// (default timeout 600 s), which is what makes approving from the island work.
//
// One extra step the settings window warns about: Codex must be told to trust
// the hooks once — run `/hooks` inside Codex after installing — and it re-checks
// whenever the handler changes, so the entries below are written the same way
// every time.

use std::path::PathBuf;

use serde_json::{json, Map, Value};

use super::{is_relay_command, AgentHooks};

/// (event, timeout seconds). PermissionRequest gets the decision budget + margin.
const HOOK_EVENTS: &[(&str, u64)] = &[
    ("SessionStart", 10),
    ("SessionEnd", 10),
    ("UserPromptSubmit", 10),
    ("PreToolUse", 10),
    ("PostToolUse", 10),
    ("PermissionRequest", 120),
    ("Stop", 10),
    ("SubagentStart", 10),
    ("SubagentStop", 10),
];

pub struct Codex;

pub static AGENT: Codex = Codex;

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
    home().join(".codex").join("hooks.json")
}

/// Unquoted: Codex spawns the command itself and the relay's install path has
/// no spaces (~/.local/state/atlas/bin on Linux, %LOCALAPPDATA%\Atlas\bin on
/// Windows).
fn hook_command(event: &str) -> String {
    let exe = crate::settings::hook_exe_path().to_string_lossy().replace('\\', "/");
    format!("{exe} --agent codex {event}")
}

fn entry_is_ours(entry: &Value) -> bool {
    // Handlers nested in the matcher group, where we write them today.
    let nested = entry
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
        .unwrap_or(false);
    // And the flat command we wrote before the matcher-group shape was pinned
    // down, so an upgrade cleanly removes what the previous install left.
    let flat = entry
        .get("command")
        .and_then(Value::as_str)
        .map(is_relay_command)
        .unwrap_or(false);
    nested || flat
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
                "statusMessage": "Atlas",
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

impl AgentHooks for Codex {
    fn id(&self) -> &'static str {
        "codex"
    }

    fn name(&self) -> &'static str {
        "Codex"
    }

    fn blurb(&self) -> &'static str {
        "Approvals and sessions — run /hooks in Codex once to trust Atlas"
    }

    fn settings_path(&self) -> PathBuf {
        settings_path()
    }

    fn read(&self) -> Result<Value, String> {
        let path = settings_path();
        match std::fs::read(&path) {
            Ok(bytes) => crate::hooks::parse_settings(&bytes, &path.display().to_string()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
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
