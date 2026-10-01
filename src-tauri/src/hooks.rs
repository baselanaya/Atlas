// Hook installation, shared across the three coding agents (see agents/).
//
// The rule from CLAUDE.md is strict and is followed to the letter for every
// config: read it, take a dated backup, merge without touching anybody else's
// hooks, show the diff, and write only after an explicit click. Uninstall
// removes Atlas's entries and nothing else. The per-agent shapes live in
// agents/; the risky parts — backup, fingerprint, diff, atomic write — are
// here, exactly once.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Manager};

use crate::agents::{self, AgentHooks};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookStatus {
    pub agent: String,
    pub name: String,
    pub blurb: String,
    pub installed: bool,
    pub settings_path: String,
    pub hook_path: String,
    pub hook_ready: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookPreview {
    pub diff: String,
    pub backup: String,
    pub settings_path: String,
    /// Identifies the bytes this diff was computed from; handed back to `write`
    /// so we only ever apply what the user actually looked at.
    pub fingerprint: String,
}

/// Parses a config file's bytes. The only error that means "start from nothing"
/// is the file not being there (checked by the caller); content we cannot use
/// is an error, because treating somebody's unreadable settings as an empty
/// object and writing that back over them is the one unforgivable move.
pub fn parse_settings(bytes: &[u8], path: &str) -> Result<Value, String> {
    // PowerShell writes a UTF-8 BOM with `Set-Content -Encoding utf8`, and
    // serde_json refuses it. Stripping it is safe and well defined; guessing at
    // anything else is not.
    let text = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    if text.iter().all(u8::is_ascii_whitespace) {
        return Ok(serde_json::json!({}));
    }
    match serde_json::from_slice::<Value>(text) {
        Ok(v) if v.is_object() => Ok(v),
        Ok(_) => Err(format!("{path} isn't a JSON object — Atlas won't touch it.")),
        Err(err) => Err(format!(
            "{path} isn't valid JSON ({err}). Fix or move it, then try again — Atlas won't overwrite it."
        )),
    }
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default()
}

/// Down to the second: installing then uninstalling in the same minute must not
/// quietly overwrite the first backup.
fn stamp() -> String {
    let t = crate::log::LocalTime::now();
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        t.year, t.month, t.day, t.hour, t.minute, t.second
    )
}

fn backup_path(path: &Path) -> PathBuf {
    path.with_file_name(format!("{}.bak-{}", path.file_name().unwrap_or_default().to_string_lossy(), stamp()))
}

/// Identifies the exact bytes a preview was computed from. FNV-1a is plenty:
/// the question is only "is this still the file I showed the user?".
fn fingerprint(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    format!("{hash:016x}")
}

fn current_fingerprint(path: &Path) -> String {
    match std::fs::read(path) {
        Ok(bytes) => fingerprint(&bytes),
        Err(_) => fingerprint(b""),
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

fn agent_or_err(id: &str) -> Result<&'static dyn AgentHooks, String> {
    agents::by_id(id).ok_or_else(|| format!("Unknown agent '{id}'"))
}

pub fn status(agent_id: &str) -> Result<HookStatus, String> {
    let agent = agent_or_err(agent_id)?;
    let installed = agent.read().map(|current| agent.installed(&current)).unwrap_or(false);
    let hook_path = crate::settings::hook_exe_path();
    Ok(HookStatus {
        agent: agent.id().to_string(),
        name: agent.name().to_string(),
        blurb: agent.blurb().to_string(),
        installed,
        settings_path: agent.settings_path().to_string_lossy().to_string(),
        hook_ready: hook_path.exists(),
        hook_path: hook_path.to_string_lossy().to_string(),
    })
}

/// Status of every agent at once, for the settings window's boot.
pub fn all_statuses() -> Vec<HookStatus> {
    agents::ids().iter().filter_map(|id| status(id).ok()).collect()
}

pub fn preview(agent_id: &str, install: bool) -> Result<HookPreview, String> {
    let agent = agent_or_err(agent_id)?;
    let path = agent.settings_path();
    let current = agent.read()?;
    let next = agent.rewrite(&current, install);
    Ok(HookPreview {
        diff: unified_diff(&pretty(&current), &pretty(&next)),
        backup: backup_path(&path).to_string_lossy().to_string(),
        settings_path: path.to_string_lossy().to_string(),
        fingerprint: current_fingerprint(&path),
    })
}

/// Writes the merged (or cleaned) config after taking a dated backup.
///
/// `fingerprint` is the one the preview was computed from. If the file changed
/// in between — another tool, another window, the user's own editor — we stop
/// and make them look at a fresh diff, because the only thing worse than not
/// installing the hooks is silently reverting somebody else's edit.
pub fn write(agent_id: &str, install: bool, fingerprint_in: &str) -> Result<String, String> {
    let agent = agent_or_err(agent_id)?;
    let path = agent.settings_path();
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;

    // Read before the backup: an unreadable file must abort before we touch
    // anything at all.
    let current = agent.read()?;
    if current_fingerprint(&path) != fingerprint_in {
        return Err(format!(
            "{} changed since the preview. Nothing was written — review the new diff.",
            path.display()
        ));
    }

    let backup = backup_path(&path);
    if path.exists() {
        std::fs::copy(&path, &backup).map_err(|e| format!("backup failed: {e}"))?;
    }

    let next = agent.rewrite(&current, install);
    let mut text = pretty(&next);
    text.push('\n');

    // Write beside the target and rename over it: a crash or a full disk leaves
    // the original config intact rather than half a file.
    let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
    let temp = path.with_file_name(format!("{name}.atlas-{}", std::process::id()));
    std::fs::write(&temp, text.as_bytes()).map_err(|e| format!("write failed: {e}"))?;
    if let Err(err) = std::fs::rename(&temp, &path) {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("write failed: {err}"));
    }
    Ok(backup.to_string_lossy().to_string())
}

/// Copies the relay (atlas-hook.exe on Windows, atlas-hook elsewhere) next to
/// the app's data on launch. In a bundled install it comes from the app
/// resources; in `tauri dev` it sits next to atlas in the workspace target
/// directory.
///
/// Every candidate is tried rather than just the first, because getting this
/// wrong is silent and fatal: `resources` used to be a glob, which made NSIS
/// mirror the source path into `_up_\target\release\`, no candidate matched, and
/// the relay was simply never installed. It only looked healthy on a developer
/// machine, where a leftover copy from `tauri dev` was already sitting in bin/.
pub fn ensure_hook_exe(app: &AppHandle) {
    let name = crate::settings::HOOK_EXE_NAME;
    let dest = crate::settings::hook_exe_path();
    let Some(dir) = dest.parent() else { return };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(p) = app.path().resolve(name, tauri::path::BaseDirectory::Resource) {
        candidates.push(p);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            // Installed build, then `tauri dev` (target/debug) next to the
            // release hook the pre-build step produces.
            candidates.push(parent.join(name));
            candidates.push(parent.join("../release").join(name));
            // Belt and braces: where the old glob form used to land it.
            #[cfg(windows)]
            candidates.push(parent.join("_up_/target/release/atlas-hook.exe"));
        }
    }

    let tried: Vec<String> = candidates.iter().map(|p| p.display().to_string()).collect();
    let Some(src) = candidates.into_iter().find(|p| p.exists()) else {
        crate::log::line(format!(
            "{name} not found — agent hooks cannot work. Looked in: {}",
            tried.join(", ")
        ));
        return;
    };

    let same = match (std::fs::metadata(&src), std::fs::metadata(&dest)) {
        (Ok(a), Ok(b)) => a.len() == b.len() && a.modified().ok() == b.modified().ok(),
        _ => false,
    };
    if same {
        return;
    }
    // A hook may be running right now and hold the file open; keeping the old
    // copy is fine, it is the same relay.
    if let Err(err) = std::fs::copy(&src, &dest) {
        if !dest.exists() {
            crate::log::line(format!("could not install {name}: {err}"));
        }
    }
}

// ── Minimal unified diff (LCS) ────────────────────────────────────────────────

/// Configs are short, so a plain O(n·m) LCS is the simplest honest diff.
fn unified_diff(before: &str, after: &str) -> String {
    let a: Vec<&str> = before.lines().collect();
    let b: Vec<&str> = after.lines().collect();
    let (n, m) = (a.len(), b.len());

    let mut lcs = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }

    let mut out: Vec<String> = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if a[i] == b[j] {
            out.push(format!("  {}", a[i]));
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            out.push(format!("- {}", a[i]));
            i += 1;
        } else {
            out.push(format!("+ {}", b[j]));
            j += 1;
        }
    }
    while i < n {
        out.push(format!("- {}", a[i]));
        i += 1;
    }
    while j < m {
        out.push(format!("+ {}", b[j]));
        j += 1;
    }

    // Keep three lines of context around each change so the panel stays readable.
    let changed: Vec<usize> = out
        .iter()
        .enumerate()
        .filter(|(_, l)| l.starts_with('+') || l.starts_with('-'))
        .map(|(i, _)| i)
        .collect();
    if changed.is_empty() {
        return "No change.".into();
    }
    let mut keep = vec![false; out.len()];
    for idx in changed {
        let lo = idx.saturating_sub(3);
        let hi = (idx + 4).min(out.len());
        for k in lo..hi {
            keep[k] = true;
        }
    }
    let mut result = String::new();
    let mut gap = false;
    for (idx, line) in out.iter().enumerate() {
        if keep[idx] {
            result.push_str(line);
            result.push('\n');
            gap = false;
        } else if !gap {
            result.push_str("  …\n");
            gap = true;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHERE: &str = "settings.json";

    #[test]
    fn a_utf8_bom_is_stripped_not_treated_as_corruption() {
        // PowerShell 5's `Set-Content -Encoding utf8` produces exactly this.
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend(br#"{"model":"opus","hooks":{}}"#);
        let parsed = parse_settings(&bytes, WHERE).expect("a BOM must not defeat the parser");
        assert_eq!(parsed["model"], "opus");
    }

    #[test]
    fn unreadable_content_is_an_error_never_an_empty_object() {
        // This is the whole bug: returning {} here meant the merge produced a
        // file containing nothing but Atlas's hooks, and the write replaced
        // everything the user had.
        for bad in [&b"{ not json"[..], &b"[1,2,3]"[..], &b"\"a string\""[..]] {
            assert!(
                parse_settings(bad, WHERE).is_err(),
                "content we cannot use must refuse, not come back empty"
            );
        }
    }

    #[test]
    fn empty_and_whitespace_files_start_from_nothing() {
        assert_eq!(parse_settings(b"", WHERE).unwrap(), serde_json::json!({}));
        assert_eq!(parse_settings(b"  
\t ", WHERE).unwrap(), serde_json::json!({}));
    }

    #[test]
    fn a_fingerprint_notices_any_change() {
        assert_eq!(fingerprint(b"{}"), fingerprint(b"{}"));
        assert_ne!(fingerprint(b"{}"), fingerprint(b"{ }"));
        assert_ne!(fingerprint(b""), fingerprint(b"{}"));
    }

    // ── per-agent merge round-trips ──────────────────────────────────────────

    fn roundtrip(agent_id: &str, existing: serde_json::Value) {
        let agent = agents::by_id(agent_id).expect("known agent");
        let after = agent.rewrite(&existing, true);
        assert!(agent.installed(&after), "{agent_id}: our hooks must be in");
        let cleaned = agent.rewrite(&after, false);
        assert_eq!(cleaned, existing, "{agent_id}: uninstall must restore the file");
    }

    #[test]
    fn claude_merge_keeps_every_other_setting_and_every_foreign_hook() {
        roundtrip(
            "claude",
            serde_json::json!({
                "model": "claude-opus-5",
                "theme": "dark",
                "enabledPlugins": ["a", "b"],
                "hooks": {
                    "PreToolUse": [
                        { "hooks": [{ "type": "command", "command": "someone-elses-tool.exe" }] }
                    ],
                    "SomeEventWeDoNotTouch": [
                        { "hooks": [{ "type": "command", "command": "keep-me.exe" }] }
                    ]
                }
            }),
        );
    }

    #[test]
    fn zcode_merge_enables_hooks_and_touches_only_events() {
        // The real ~/.zcode/cli/config.json is full of plugin settings; a fresh
        // one has no hooks key at all, and both must survive a roundtrip.
        roundtrip("zcode", serde_json::json!({ "plugins": { "enabledPlugins": {} } }));

        let agent = agents::by_id("zcode").unwrap();
        let after = agent.rewrite(&serde_json::json!({ "plugins": {} }), true);
        assert_eq!(after["hooks"]["enabled"], true, "config hooks are off by default");
        assert_eq!(after["hooks"]["events"].as_object().unwrap().len(), 7, "zcode has exactly seven events");
        assert_eq!(after["plugins"], serde_json::json!({}));

        // With somebody else's hooks in place, uninstall keeps them and keeps
        // the runner enabled — dropping `enabled` would silently re-disable the
        // user's own hooks, which were off before Atlas ever showed up.
        let existing = serde_json::json!({
            "hooks": { "events": {
                "PreToolUse": [
                    { "hooks": [{ "type": "process", "command": "/usr/bin/other-tool" }] }
                ]
            }}
        });
        let after = agent.rewrite(&existing, true);
        let cleaned = agent.rewrite(&after, false);
        assert!(agent.installed(&after));
        assert!(!agent.installed(&cleaned));
        let kept = cleaned["hooks"]["events"]["PreToolUse"].as_array().unwrap();
        assert!(kept.iter().any(|e| serde_json::to_string(e).unwrap().contains("other-tool")));
        assert_eq!(cleaned["hooks"]["enabled"], true);
    }

    #[test]
    fn codex_merge_keeps_foreign_hooks_and_skips_missing_events() {
        roundtrip(
            "codex",
            serde_json::json!({
                "hooks": {
                    "PreToolUse": [
                        { "hooks": [{ "type": "command", "command": "node /Users/me/govern.mjs" }] }
                    ],
                    "SomeCodexOnlyEvent": [ { "hooks": [{ "type": "command", "command": "keep-me" }] } ]
                }
            }),
        );

        let agent = agents::by_id("codex").unwrap();
        let after = agent.rewrite(&serde_json::json!({}), true);
        let events = after["hooks"].as_object().unwrap();
        assert_eq!(events.len(), 9);
        assert!(events.keys().all(|k| !k.contains("Failure") && k != "Notification"));
        // The matcher-group shape codex-rs validates: handlers under "hooks",
        // tagged with their type.
        let handler = &after["hooks"]["SessionStart"][0]["hooks"][0];
        assert_eq!(handler["type"], "command");
        assert!(handler["command"].as_str().unwrap().contains("--agent codex"));
    }

    /// Everything filesystem-shaped lives in one test on purpose: it points the
    /// home directory at a temp directory, and that is process-wide.
    #[test]
    fn writing_backs_up_preserves_and_refuses_a_changed_file() {
        let _home = crate::TEST_HOME_LOCK.lock().unwrap();
        let tmp = std::env::temp_dir().join(format!("atlas-hooks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join(".claude")).unwrap();
        #[cfg(windows)]
        std::env::set_var("USERPROFILE", &tmp);
        #[cfg(unix)]
        std::env::set_var("HOME", &tmp);

        let path = agents::by_id("claude").unwrap().settings_path();
        assert!(path.starts_with(&tmp), "the test must not touch the real home");

        // A real-shaped file, written the way PowerShell 5 would: UTF-8 with BOM.
        let original = r#"{"model":"claude-opus-5","theme":"dark","tui":{"x":1},"hooks":{"PreToolUse":[{"hooks":[{"type":"command","command":"other-tool.exe"}]}]}}"#;
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend(original.as_bytes());
        std::fs::write(&path, &bytes).unwrap();

        // Install.
        let plan = preview("claude", true).expect("a BOM must not stop the preview");
        assert!(plan.diff.contains("atlas-hook"), "the diff must show what changes");
        let backup = write("claude", true, &plan.fingerprint).expect("install should succeed");

        // The backup holds the original bytes, BOM and all.
        assert_eq!(std::fs::read(&backup).unwrap(), bytes);

        // Everything else survived, and so did the other tool's hook.
        let after: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(after["model"], "claude-opus-5");
        assert_eq!(after["theme"], "dark");
        assert_eq!(after["tui"]["x"], 1);
        let pre = after["hooks"]["PreToolUse"].as_array().unwrap();
        assert!(pre.iter().any(|e| serde_json::to_string(e).unwrap().contains("other-tool.exe")));
        assert!(status("claude").unwrap().installed);

        // A file that moved since the preview is refused, and left alone.
        let stale = preview("claude", false).unwrap();
        std::fs::write(&path, br#"{"model":"someone-else-edited-this"}"#).unwrap();
        let err = write("claude", false, &stale.fingerprint).unwrap_err();
        assert!(err.contains("changed since the preview"), "got: {err}");
        let untouched: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(untouched["model"], "someone-else-edited-this");

        // Content we cannot parse is refused before anything is written.
        std::fs::write(&path, b"{ broken").unwrap();
        assert!(preview("claude", true).is_err());
        assert!(write("claude", true, "whatever").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"{ broken");

        let _ = std::fs::remove_dir_all(&tmp);
    }

}
