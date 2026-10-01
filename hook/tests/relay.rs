//! Golden tests for the relay protocol: exactly what leaves atlas-hook must
//! stay byte-stable, because three coding agents parse it and the island
//! answers it. These run the real binary (CARGO_BIN_EXE_) against a mock
//! app on a private socket, with a private $XDG_RUNTIME_DIR so the test can
//! never touch a live Atlas.

#![cfg(unix)]

use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::process::{Command, Stdio};
use std::time::Duration;

fn hook() -> Command {
    Command::new(env!("CARGO_BIN_EXE_atlas-hook"))
}

/// One relay run: sends `stdin_json`, returns (stdout, exit_code).
fn run(args: &[&str], stdin_json: &str, runtime_dir: &std::path::Path) -> (String, Option<i32>) {
    let child = hook()
        .args(args)
        .env("XDG_RUNTIME_DIR", runtime_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn atlas-hook");
    {
        let mut stdin = child.stdin.as_ref().unwrap();
        stdin.write_all(stdin_json.as_bytes()).unwrap();
    }
    let out = child.wait_with_output().expect("wait");
    (String::from_utf8_lossy(&out.stdout).to_string(), out.status.code())
}

#[test]
fn a_dead_socket_costs_the_session_nothing() {
    let dir = std::env::temp_dir().join(format!("atlas-relay-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // No server, no socket file: instant exit 0, nothing on stdout.
    let (out, code) = run(
        &["PreToolUse"],
        r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","cwd":"/tmp"}"#,
        &dir,
    );
    assert_eq!(code, Some(0), "a closed app must never block the agent");
    assert!(out.trim().is_empty(), "and must print nothing");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fire_and_forget_events_reach_the_app_and_get_no_answer() {
    let dir = std::env::temp_dir().join(format!("atlas-relay-ff-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("atlas.sock");
    let server = UnixListener::bind(&path).unwrap();

    let payload = r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"ls"},"cwd":"/tmp/x","session_id":"t1"}"#;
    let mut child = hook()
        .args(["--agent", "zcode", "PreToolUse"])
        .env("XDG_RUNTIME_DIR", &dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.as_mut().unwrap().write_all(payload.as_bytes()).unwrap();
    drop(child.stdin.take());

    let (mut stream, _) = server.accept().unwrap();
    let mut buf = String::new();
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    stream.read_to_string(&mut buf).unwrap();

    // The wire format: one JSON line, newline-terminated, agent-stamped,
    // event name normalized in, oversized fields dropped.
    assert!(buf.ends_with('\n'), "payload is newline-delimited");
    let v: serde_json::Value = serde_json::from_str(buf.trim()).unwrap();
    assert_eq!(v["hook_event_name"], "PreToolUse");
    assert_eq!(v["agent"], "zcode");
    assert_eq!(v["cwd"], "/tmp/x");
    assert!(v.get("transcript_path").is_none(), "big fields are dropped");

    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty(), "non-permission events are fire-and-forget");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_permission_request_round_trips_the_documented_decision() {
    let dir = std::env::temp_dir().join(format!("atlas-relay-perm-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("atlas.sock");
    let server = UnixListener::bind(&path).unwrap();

    let payload = r#"{"hook_event_name":"PermissionRequest","tool_name":"Bash","tool_input":{"command":"rm -rf /"},"cwd":"/tmp","session_id":"t2"}"#;
    let mut child = hook()
        .args(["PermissionRequest"])
        .env("XDG_RUNTIME_DIR", &dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.as_mut().unwrap().write_all(payload.as_bytes()).unwrap();
    drop(child.stdin.take());

    let (mut stream, _) = server.accept().unwrap();
    let mut buf = String::new();
    let mut chunk = [0u8; 1024];
    // The relay holds the connection open while it waits for the island's
    // word, so the server reads to the newline, never to EOF.
    loop {
        let n = stream.read(&mut chunk).unwrap();
        buf.push_str(&String::from_utf8_lossy(&chunk[..n]));
        if buf.contains('\n') || n == 0 { break; }
    }
    let v: serde_json::Value = serde_json::from_str(buf.trim()).unwrap();
    assert_eq!(v["hook_event_name"], "PermissionRequest");
    // No --agent flag means Claude, the compatibility default.
    assert_eq!(v["agent"], "claude");

    // The island clicks Allow: one bare word back on the same connection.
    stream.write_all(b"allow\n").unwrap();
    stream.flush().unwrap();
    drop(stream);

    let out = child.wait_with_output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}"#,
        "the decision JSON is the byte-stable shape all three agents parse"
    );
    assert_eq!(out.status.code(), Some(0));

    let _ = std::fs::remove_dir_all(&dir);
}
