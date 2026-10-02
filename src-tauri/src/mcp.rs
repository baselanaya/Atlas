//! MCP-out: Atlas as a tool server. Any local MCP client (Voicebox, an IDE,
//! a script) can ask what the agents are doing and act on their permission
//! requests.
//!
//! Streamable-HTTP-flavoured JSON-RPC on 127.0.0.1 only, behind a settings
//! switch: opt-in, like every other service Atlas talks to. The one thing to
//! know before flipping it on: `atlas_decide` lets the connected client
//! approve or deny a request — point it only at tools you trust, same rule
//! as the agents' own hooks.


use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

/// Kept clear of Voicebox's 17493 and the usual dev ports.
pub const DEFAULT_PORT: u16 = 17510;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpStatus {
    pub enabled: bool,
    pub port: u16,
    pub running: bool,
}

pub fn status(app: &AppHandle) -> McpStatus {
    let settings = app.state::<crate::Shared>().settings.lock().unwrap().clone();
    McpStatus {
        enabled: settings.mcp_enabled,
        port: settings.mcp_port,
        running: settings.mcp_enabled && RUNNING.load(std::sync::atomic::Ordering::Acquire),
    }
}

static RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn start_if_enabled(app: &AppHandle) {
    let settings = app.state::<crate::Shared>().settings.lock().unwrap().clone();
    if !settings.mcp_enabled {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || serve(app));
}

fn serve(app: AppHandle) {
    let port = app.state::<crate::Shared>().settings.lock().unwrap().mcp_port;
    let Ok(server) = tiny_http::Server::http(("127.0.0.1", port)) else {
        crate::log::line(format!("mcp: cannot listen on 127.0.0.1:{port}"));
        return;
    };
    RUNNING.store(true, std::sync::atomic::Ordering::Release);
    crate::log::line(format!("mcp: serving on 127.0.0.1:{port}"));
    for request in server.incoming_requests() {
        let app = app.clone();
        let response = std::thread::spawn(move || handle(&app, request)).join();
        let _ = response;
    }
}

fn handle(app: &AppHandle, mut request: tiny_http::Request) {
    let mut body = String::new();
    if request.as_reader().read_to_string(&mut body).is_err() {
        return;
    }
    let method_url = request.url().to_string();
    let (code, payload) = respond(app, &method_url, &body);
    let header =
        tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
    match tiny_http::Response::from_string(payload).with_status_code(code).with_header(header) {
        response => {
            let _ = request.respond(response);
        }
    }
}

/// One JSON-RPC message in, one JSON body out. Batched messages are not
/// needed by the local clients this exists for. Tool calls are handed to
/// `call_tool`, the only part that touches app state.
fn respond(app: &AppHandle, url: &str, body: &str) -> (u16, String) {
    respond_with(
        url,
        body,
        |name, args| call_tool(app, name, args),
    )
}

/// The pure JSON-RPC half — testable without an app.
fn respond_with<F>(url: &str, body: &str, call_tool: F) -> (u16, String)
where
    F: Fn(&str, &Value) -> String,
{
    if url.split('?').next() != Some("/mcp") && url != "/" {
        return (404, "{}".into());
    }
    let Ok(msg) = serde_json::from_str::<Value>(body) else {
        return (400, error_response(None, -32700, "parse error"));
    };
    if msg.get("jsonrpc") != Some(&json!("2.0")) {
        return (400, error_response(msg.get("id").cloned(), -32600, "not JSON-RPC 2.0"));
    }
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(Value::as_str).unwrap_or_default();
    let params = msg.get("params").cloned().unwrap_or(json!({}));

    // Notifications (no id) get a bare 202 and never a body.
    let Some(id) = id else {
        return (202, String::new());
    };

    let result = match method {
        "initialize" => json!({
            "protocolVersion": "2025-06-18",
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "atlas", "version": env!("CARGO_PKG_VERSION") },
        }),
        "ping" => json!({}),
        "tools/list" => json!({ "tools": tool_specs() }),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            let text = call_tool(name, &args);
            json!({ "content": [ { "type": "text", "text": text } ], "isError": false })
        }
        _ => {
            return (200, error_response(Some(id), -32601, &format!("unknown method {method}")));
        }
    };
    (200, serde_json::to_string(&json!({ "jsonrpc": "2.0", "id": id, "result": result })).unwrap())
}

fn error_response(id: Option<Value>, code: i64, message: &str) -> String {
    serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message },
    }))
    .unwrap()
}

fn tool_specs() -> Value {
    json!([
        {
            "name": "atlas_status",
            "description": "What the coding agents (Claude Code, ZCode, Codex) are doing right now, and today's counts.",
            "inputSchema": { "type": "object", "properties": {} },
        },
        {
            "name": "atlas_pending",
            "description": "Permission requests waiting for a human in the Atlas island.",
            "inputSchema": { "type": "object", "properties": {} },
        },
        {
            "name": "atlas_decide",
            "description": "Approve or deny a pending permission request by its id. Use only with the user's blessing — this answers an agent's permission prompt.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "request_id": { "type": "string" },
                    "decision": { "type": "string", "enum": ["allow", "deny"] },
                },
                "required": ["request_id", "decision"],
            },
        },
        {
            "name": "atlas_speak",
            "description": "Make the island speak a line through the configured Voicebox voice (if voice is enabled).",
            "inputSchema": {
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "required": ["text"],
            },
        },
        {
            "name": "atlas_tokens",
            "description": "Token usage per agent per day, read from the agents' own session transcripts (Claude Code, Codex).",
            "inputSchema": {
                "type": "object",
                "properties": { "days": { "type": "integer", "minimum": 1, "maximum": 60 } },
            },
        },
        {
            "name": "atlas_stats",
            "description": "Per-agent daily activity counters for the last N days (default 7).",
            "inputSchema": {
                "type": "object",
                "properties": { "days": { "type": "integer", "minimum": 1, "maximum": 60 } },
            },
        },
    ])
}

fn call_tool(app: &AppHandle, name: &str, args: &Value) -> String {
    match name {
        "atlas_status" => {
            let snap = crate::stats::snapshot_json(1);
            let today = snap.as_object().and_then(|o| o.values().next().cloned()).unwrap_or(json!({}));
            serde_json::to_string_pretty(&json!({
                "agents": today.get("agents").cloned().unwrap_or(json!({})),
            }))
            .unwrap_or_default()
        }
        "atlas_pending" => serde_json::to_string_pretty(&crate::pipe::pending_info()).unwrap_or_default(),
        "atlas_decide" => {
            let id = args.get("request_id").and_then(Value::as_str).unwrap_or_default();
            let decision = args.get("decision").and_then(Value::as_str).unwrap_or_default();
            if id.is_empty() || !matches!(decision, "allow" | "deny") {
                return "Expected request_id and decision ('allow' | 'deny').".into();
            }
            if !crate::pipe::pending_info().as_array().is_some_and(|list| {
                list.iter().any(|p| p.get("request_id") == Some(&json!(id)))
            }) {
                return format!("No pending request {id}.");
            }
            crate::pipe::answer(app, id, decision);
            format!("Decision '{decision}' sent for {id}.")
        }
        "atlas_speak" => {
            let text = args.get("text").and_then(Value::as_str).unwrap_or_default().to_string();
            let (enabled, profile) = {
                let shared = app.state::<crate::Shared>();
                let settings = shared.settings.lock().unwrap();
                (settings.voice_enabled, settings.voice_profile.clone())
            };
            if !enabled {
                return "Voice is disabled in Atlas settings.".into();
            }
            let spoken = text.clone();
            let dir = {
                let shared = app.state::<crate::Shared>();
                let dir = shared.settings.lock().unwrap().voice_output_dir.clone();
                dir
            };
            tauri::async_runtime::spawn(async move {
                crate::voice::speak(&spoken, &profile, &dir).await;
            });
            format!("Speaking: {text}")
        }
        "atlas_tokens" => {
            let days = args.get("days").and_then(Value::as_u64).unwrap_or(7).clamp(1, 60) as usize;
            serde_json::to_string_pretty(&crate::tokens::scan(days)).unwrap_or_default()
        }
        "atlas_stats" => {
            let days = args.get("days").and_then(Value::as_u64).unwrap_or(7).clamp(1, 60) as usize;
            serde_json::to_string_pretty(&crate::stats::snapshot_json(days)).unwrap_or_default()
        }
        _ => format!("Unknown tool {name}."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rpc(body: &str) -> (u16, String) {
        respond_with("/mcp", body, |name, _args| format!("stub:{name}"))
    }

    #[test]
    fn jsonrpc_initialize_lists_tools_and_calls_them() {
        let init = rpc(r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#);
        assert_eq!(init.0, 200);
        let v: Value = serde_json::from_str(&init.1).unwrap();
        assert_eq!(v["result"]["serverInfo"]["name"], "atlas");

        let list = rpc(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
        let v: Value = serde_json::from_str(&list.1).unwrap();
        let names: Vec<&str> = v["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"atlas_status"));
        assert!(names.contains(&"atlas_decide"));

        let call = rpc(
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"atlas_status","arguments":{}}}"#,
        );
        let v: Value = serde_json::from_str(&call.1).unwrap();
        assert_eq!(v["result"]["content"][0]["text"], "stub:atlas_status");

        let ping = rpc(r#"{"jsonrpc":"2.0","id":3,"method":"ping"}"#);
        assert!(ping.1.contains("\"result\""));

        // Notifications: no id, no body.
        let note = rpc(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
        assert_eq!(note.0, 202);
        assert!(note.1.is_empty());

        // Unknown method: proper JSON-RPC error.
        let bad = rpc(r#"{"jsonrpc":"2.0","id":4,"method":"nope"}"#);
        assert!(bad.1.contains("-32601"));

        // Wrong path: 404.
        let off = respond_with("/nope", "{}", |n, _| n.to_string());
        assert_eq!(off.0, 404);
    }
}


// ── stdio bridge ──────────────────────────────────────────────────────────────

/// Forwards stdin JSON-RPC lines to the island's HTTP MCP, line by line, so
/// stdio-only clients (Codex and friends) can use the same tools. Fails with
/// a readable message when the island isn't running — a dead bridge should
/// say so, not hang.
pub fn stdio_bridge() -> i32 {
    use std::io::{BufRead, Write};

    let port = std::env::var("ATLAS_MCP_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(DEFAULT_PORT);
    let url = format!("http://127.0.0.1:{port}/mcp");

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let target = url.clone();
        let response = std::thread::scope(|scope| {
            scope.spawn(move || {
                let client = reqwest::blocking::Client::builder()
                    .timeout(std::time::Duration::from_secs(30))
                    .build()
                    .map_err(|e| e.to_string())?;
                let resp = client
                    .post(&target)
                    .header("content-type", "application/json")
                    .body(line)
                    .send()
                    .map_err(|_| format!("Atlas isn't running at {}", target.clone()))?;
                let _ = resp.status();
                let body = resp.text().map_err(|e| e.to_string())?;
                // Empty bodies (notifications) and error bodies both pass
                // through as-is — the client's JSON-RPC layer knows the shapes.
                Ok::<String, String>(body)
            })
            .join()
        });
        match response {
            Ok(Ok(text)) if !text.is_empty() => {
                let _ = writeln!(stdout, "{text}");
                let _ = stdout.flush();
            }
            Ok(Ok(_)) => {} // 202-style empties: notifications
            Ok(Err(e)) => {
                let _ = writeln!(stdout, "{{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{{\"code\":-32000,\"message\":\"{e}\"}}}}");
                let _ = stdout.flush();
            }
            Err(_) => break,
        }
    }
    0
}
