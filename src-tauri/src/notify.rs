//! System notifications — the island's backup channel for the moments that
//! shouldn't wait for a hover: a permission request sitting in the island, a
//! session that died. Plain `notify-send` on Linux (KDE and GNOME both ship a
//! notifier that renders it); a no-op everywhere else. Nothing is sent when
//! the binary or the user's toggle is missing.

use serde_json::Value;
use tauri::{AppHandle, Manager};

pub fn approval(app: &AppHandle, agent: &str, payload: &Value) {
    let enabled = app
        .try_state::<crate::Shared>()
        .map(|s| s.settings.lock().unwrap().notify_enabled)
        .unwrap_or(false);
    if !enabled {
        return;
    }
    let tool = payload.get("tool_name").and_then(Value::as_str).unwrap_or("a tool");
    let target = payload
        .get("tool_input")
        .and_then(|i| i.get("command").or_else(|| i.get("file_path")).or_else(|| i.get("path")))
        .and_then(Value::as_str)
        .unwrap_or("")
        .chars()
        .take(100)
        .collect::<String>();
    let body = if target.is_empty() {
        format!("{agent} · {tool}")
    } else {
        format!("{agent} · {tool} · {target}")
    };
    send("Atlas — approval needed", &body);
}

pub fn send(title: &str, body: &str) {
    #[cfg(unix)]
    {
        let mut cmd = std::process::Command::new("notify-send");
        cmd.args(["--app-name=Atlas", "--expire-time=8000", "--icon=atlas", title, body]);
        let _ = cmd.spawn();
    }
    #[cfg(not(unix))]
    {
        let _ = (title, body);
    }
}
