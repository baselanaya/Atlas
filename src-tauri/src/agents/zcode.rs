// zcode — the same idea as Claude Code, a different settings file and schema.
//
// `~/.zcode/cli/config.json` (or a workspace <repo>/.zcode/config.json, which
// we never touch). Hooks live under `hooks.events.<Event>`, and two zcode
// specifics matter:
//
//   * configuration-file hooks are disabled by default — installing ours must
//     also set `hooks.enabled: true`, or nothing will ever fire;
//   * exactly seven events exist (SessionStart, UserPromptSubmit, PreToolUse,
//     PermissionRequest, PostToolUse, PostToolUseFailure, Stop). The others
//     Atlas listens to — Notification, SessionEnd, StopFailure, SubagentStart,
//     SubagentStop — are not supported and must not be written.
//
// A `process` hook (an argument vector, no shell) is the most portable form, so
// the relay is invoked as `atlas-hook --agent zcode <Event>` with timeouts in
// milliseconds.

use std::path::PathBuf;

use serde_json::{json, Map, Value};

use super::{is_relay_command, AgentHooks};

/// (event, timeoutMs). PermissionRequest gets the decision budget + margin.
const HOOK_EVENTS: &[(&str, u64)] = &[
    ("SessionStart", 10_000),
    ("UserPromptSubmit", 10_000),
    ("PreToolUse", 10_000),
    ("PostToolUse", 10_000),
    ("PostToolUseFailure", 10_000),
    ("PermissionRequest", 120_000),
    ("Stop", 10_000),
];

pub struct Zcode;

pub static AGENT: Zcode = Zcode;

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
    home().join(".zcode").join("cli").join("config.json")
}

fn hook_entry(event: &str, timeout_ms: u64) -> Value {
    let exe = crate::settings::hook_exe_path().to_string_lossy().to_string();
    json!({
        "hooks": [{
            "type": "process",
            "command": exe,
            "args": ["--agent", "zcode", event],
            "timeoutMs": timeout_ms,
            "statusMessage": "Atlas",
        }]
    })
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

fn events_of(hooks: &Value) -> Map<String, Value> {
    hooks
        .get("events")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

fn merged(existing: &Value) -> Value {
    let mut root = existing.as_object().cloned().unwrap_or_default();
    let mut hooks = root.get("hooks").and_then(Value::as_object).cloned().unwrap_or_default();
    let mut events = events_of(&Value::Object(hooks.clone()));

    for (event, timeout_ms) in HOOK_EVENTS {
        let mut list = events
            .get(*event)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        list.retain(|entry| !entry_is_ours(entry));
        list.push(hook_entry(event, *timeout_ms));
        events.insert((*event).to_string(), Value::Array(list));
    }

    // Not optional: without it, configuration-file hooks never run at all.
    hooks.insert("enabled".into(), Value::Bool(true));
    hooks.insert("events".into(), Value::Object(events));
    root.insert("hooks".into(), Value::Object(hooks));
    Value::Object(root)
}

fn without_ours(existing: &Value) -> Value {
    let mut root = existing.as_object().cloned().unwrap_or_default();
    let Some(mut hooks) = root.get("hooks").and_then(Value::as_object).cloned() else {
        return Value::Object(root);
    };
    let mut events = events_of(&Value::Object(hooks.clone()));

    let event_names: Vec<String> = events.keys().cloned().collect();
    for event in event_names {
        if let Some(list) = events.get(&event).and_then(Value::as_array).cloned() {
            let kept: Vec<Value> = list.iter().filter(|e| !entry_is_ours(e)).cloned().collect();
            events.insert(event, Value::Array(kept));
        }
    }
    events.retain(|_, v| {
        v.as_array().map(|list| !list.is_empty()).unwrap_or(true)
    });

    if events.is_empty() {
        hooks.remove("events");
        // `enabled: true` with no events left is a flag we added for entries
        // that are no longer there; drop it with them.
        if hooks.len() == 1 && hooks.contains_key("enabled") {
            root.remove("hooks");
            return Value::Object(root);
        }
    } else {
        hooks.insert("events".into(), Value::Object(events));
    }
    root.insert("hooks".into(), Value::Object(hooks));
    Value::Object(root)
}

impl AgentHooks for Zcode {
    fn id(&self) -> &'static str {
        "zcode"
    }

    fn name(&self) -> &'static str {
        "ZCode"
    }

    fn blurb(&self) -> &'static str {
        "GLM sessions and approvals in the island"
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
            .and_then(|h| h.get("events"))
            .and_then(Value::as_object)
            .map(|events| {
                events
                    .values()
                    .filter_map(Value::as_array)
                    .flatten()
                    .any(entry_is_ours)
            })
            .unwrap_or(false)
    }
}
