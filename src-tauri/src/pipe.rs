// Transport server for atlas-hook.
//
// Windows: the named pipe `\\.\pipe\atlas-<sid>` — one instance per connection.
// Linux: the Unix domain socket $XDG_RUNTIME_DIR/atlas.sock (per-user by
// design, so it plays the role the SID plays in the pipe name). The wire format
// is identical either way: one JSON object per connection, newline-terminated.
//
// Every hook event is forwarded to the island as a `hook` event.
// `PermissionRequest` is the only one that keeps its connection open: it waits
// for the island's decision and writes it back on the same stream, which is how
// approving from the island works.
//
// Claude Code is never blocked by us. Three things guarantee it:
//   * atlas-hook gives the connection 300 ms and exits cleanly if we are closed;
//   * we only wait for a human once the island has *confirmed* the card is on
//     screen, so a paused island or a webview that is not listening costs a few
//     hundred milliseconds, not two minutes;
//   * whatever happens we drop the connection after the decision timeout, and
//     the terminal takes over.
//
// What we write back is the bare word `allow` or `deny`. Turning that into the
// documented hookSpecificOutput JSON is atlas-hook's job, so the wire format
// Claude Code expects lives in exactly one place.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
#[cfg(unix)]
use tokio::net::UnixListener;
#[cfg(windows)]
use tokio::net::windows::named_pipe::ServerOptions;
use tokio::sync::mpsc;

use crate::island::WINDOW_LABEL;
use crate::log;

/// Slightly under atlas-hook's own 110 s wait, so we always answer first.
const DECISION_TIMEOUT: Duration = Duration::from_secs(108);
/// How long the island gets to say "the card is up". This is the whole of B4:
/// without it, an island that is paused, hidden behind a crashed webview or
/// simply not listening would leave Claude Code staring at a prompt nobody can
/// see for nearly two minutes.
const ACK_TIMEOUT: Duration = Duration::from_millis(800);
const MAX_PAYLOAD: usize = 1 << 20;

/// What the island can say about a permission request.
pub enum Reply {
    /// The card is on screen and a human can act on it.
    Ack,
    /// A human clicked: `allow` or `deny`.
    Decision(String),
    /// Nobody can act on it — paused, or another request already holds the card.
    Decline,
}

/// Permission requests the island has been told about.
#[derive(Default)]
pub struct Pending(pub Mutex<HashMap<String, mpsc::Sender<Reply>>>);

/// The same requests, described for humans and MCP clients: what is being
/// asked, by which agent, under which id. Kept beside the senders and removed
/// with them.
#[derive(Default)]
pub struct PendingInfo(pub Mutex<Vec<serde_json::Value>>);

/// The pending card list, newest last — what `atlas_pending` reports.
pub fn pending_info() -> Value {
    PENDING_INFO.0.lock().unwrap().clone().into()
}

static PENDING_INFO: PendingInfo = PendingInfo(Mutex::new(Vec::new()));

static COUNTER: AtomicU64 = AtomicU64::new(1);

/// `\\.\pipe\atlas-<sid>` — must match atlas-hook's `pipe_path()` exactly.
#[cfg(windows)]
fn pipe_name() -> String {
    let key = crate::win_user::current_user_sid()
        .unwrap_or_else(|| std::env::var("USERNAME").unwrap_or_else(|_| "user".into()));
    format!(r"\\.\pipe\atlas-{key}")
}

pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        #[cfg(windows)]
        serve_windows(app).await;
        #[cfg(unix)]
        serve_unix(app).await;
    });
}

#[cfg(windows)]
async fn serve_windows(app: AppHandle) {
    let name = pipe_name();
    // first_pipe_instance also means we refuse to join a pipe somebody else
    // already owns under our name, rather than serving on top of it.
    let mut server = match ServerOptions::new().first_pipe_instance(true).create(&name) {
        Ok(s) => s,
        Err(err) => {
            log::line(format!("cannot open the relay pipe: {err}"));
            return;
        }
    };
    loop {
        if server.connect().await.is_err() {
            tokio::time::sleep(Duration::from_millis(200)).await;
            continue;
        }
        // Hand the connected instance to a task and listen on a fresh one.
        let next = match ServerOptions::new().create(&name) {
            Ok(s) => s,
            Err(err) => {
                log::line(format!("cannot reopen the relay pipe: {err}"));
                return;
            }
        };
        let connected = std::mem::replace(&mut server, next);
        let app = app.clone();
        tauri::async_runtime::spawn(async move { handle(app, connected).await });
    }
}

#[cfg(unix)]
async fn serve_unix(app: AppHandle) {
    let path = crate::settings::socket_path();
    if let Some(dir) = path.parent() {
        if std::fs::create_dir_all(dir).is_err() {
            log::line(format!("cannot create {}", dir.display()));
            return;
        }
    }
    // A crashed instance leaves the socket file behind and bind() cannot reuse
    // it. A *live* second instance is already prevented by the single-instance
    // plugin, so removing the file here is always the right call.
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(err) => {
            log::line(format!("cannot open the relay socket at {}: {err}", path.display()));
            return;
        }
    };
    // Only our own user may talk to us. $XDG_RUNTIME_DIR is already a 0700
    // per-user directory, but the fallback under ~/.local/state is not.
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move { handle(app, stream).await });
            }
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }
}

/// Everything past the accept is the same on every platform; dropping the stream
/// at the end closes it (and disconnects the pipe instance on Windows).
async fn handle<S>(app: AppHandle, mut stream: S)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match stream.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.contains(&b'\n') || buf.len() > MAX_PAYLOAD {
                    break;
                }
            }
            Err(_) => return,
        }
    }
    let line = match buf.iter().position(|b| *b == b'\n') {
        Some(i) => &buf[..i],
        None => &buf[..],
    };
    let Ok(mut payload) = serde_json::from_slice::<Value>(line) else { return };
    if !payload.is_object() {
        return;
    }

    let event = payload
        .get("hook_event_name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let agent = payload.get("agent").and_then(Value::as_str).unwrap_or("claude").to_string();

    crate::stats::record(&agent, &event, None);

    if event != "PermissionRequest" {
        log::line(format!("hook {event}"));
        let _ = app.emit_to(WINDOW_LABEL, "hook", payload);
        return;
    }

    let id = format!("{}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed));
    let (tx, mut rx) = mpsc::channel::<Reply>(4);
    {
        let pending = app.state::<Pending>();
        pending.0.lock().unwrap().insert(id.clone(), tx);
    }
    payload["request_id"] = json!(id);
    log::line(format!("hook PermissionRequest id={id}"));

    // The human-facing card list (ours + the MCP tools').
    {
        let card = json!({
            "request_id": id,
            "agent": agent,
            "tool": payload.get("tool_name").and_then(Value::as_str).unwrap_or("Tool"),
            "target": payload
                .get("tool_input")
                .and_then(|i| i.get("command").or_else(|| i.get("file_path")).or_else(|| i.get("path")))
                .and_then(Value::as_str)
                .unwrap_or("")
                .chars().take(120).collect::<String>(),
        });
        PENDING_INFO.0.lock().unwrap().push(card);
    }
    crate::notify::approval(&app, &agent, &payload);
    let _ = app.emit_to(WINDOW_LABEL, "hook", payload);

    let decision = wait_for_decision(&id, &mut rx).await;
    app.state::<Pending>().0.lock().unwrap().remove(&id);
    PENDING_INFO.0.lock().unwrap().retain(|c| c.get("request_id") != Some(&json!(id)));
    if let Some(d) = &decision {
        crate::stats::record(&agent, "Decision", Some(d));
    }

    // No decision: say nothing at all. atlas-hook then writes nothing to stdout
    // and Claude Code asks in the terminal, exactly as if Atlas were closed.
    if let Some(d) = decision {
        let _ = stream.write_all(format!("{d}\n").as_bytes()).await;
        let _ = stream.flush().await;
    }
}

/// Two waits: a short one for "the card is up", then the long one for a human.
async fn wait_for_decision(id: &str, rx: &mut mpsc::Receiver<Reply>) -> Option<String> {
    match tokio::time::timeout(ACK_TIMEOUT, rx.recv()).await {
        Ok(Some(Reply::Ack)) => {}
        // A click that beats the ack is still a click.
        Ok(Some(Reply::Decision(d))) => {
            log::line(format!("hook id={id} answered {d}"));
            return Some(d);
        }
        Ok(Some(Reply::Decline)) => {
            log::line(format!("hook id={id} not shown — terminal takes over"));
            return None;
        }
        Ok(None) => return None,
        Err(_) => {
            log::line(format!("hook id={id} island never acknowledged — terminal takes over"));
            return None;
        }
    }

    match tokio::time::timeout(DECISION_TIMEOUT, rx.recv()).await {
        Ok(Some(Reply::Decision(d))) => {
            log::line(format!("hook id={id} answered {d}"));
            Some(d)
        }
        Ok(Some(Reply::Decline)) => {
            log::line(format!("hook id={id} released without a decision"));
            None
        }
        _ => {
            log::line(format!("hook id={id} timed out — terminal takes over"));
            None
        }
    }
}

fn send(app: &AppHandle, request_id: &str, reply: Reply, keep: bool) {
    let sender = {
        let pending = app.state::<Pending>();
        let mut map = pending.0.lock().unwrap();
        if keep { map.get(request_id).cloned() } else { map.remove(request_id) }
    };
    match sender {
        Some(tx) => {
            let _ = tx.try_send(reply);
        }
        None => log::line(format!("reply for id={request_id} — no pending request")),
    }
}

/// The island has the card on screen; the long wait may begin.
pub fn acknowledge(app: &AppHandle, request_id: &str) {
    send(app, request_id, Reply::Ack, true);
}

/// Nobody can act on this one — paused, or another card already holds the view.
pub fn decline(app: &AppHandle, request_id: &str) {
    log::line(format!("decline id={request_id}"));
    send(app, request_id, Reply::Decline, false);
}

/// Called by the island's Allow / Deny buttons. Only ever a bare word: turning
/// it into Claude Code's JSON is atlas-hook's job.
pub fn answer(app: &AppHandle, request_id: &str, decision: &str) {
    let word = match decision {
        "allow" | "always" => "allow",
        _ => "deny",
    };
    log::line(format!("decision id={request_id} {word}"));
    send(app, request_id, Reply::Decision(word.to_string()), false);
}
