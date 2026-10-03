//! What the agents actually did, per day, per harness — the numbers behind
//! the Stats section and the MCP tools. Fed from the relay's event stream in
//! pipe.rs, persisted to ~/.local/state/atlas/stats.json, kept small: counters
//! and a last-seen line, nothing that identifies a project or a command.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde::Serialize;
use serde_json::Value;

#[derive(Default, Clone, Serialize, serde::Deserialize)]
pub struct AgentStats {
    pub sessions: u64,
    pub tool_calls: u64,
    pub errors: u64,
    pub approvals: u64,
    pub allows: u64,
    pub denies: u64,
    /// The last event seen from this agent (SessionStart, PreToolUse, …).
    pub last_event: String,
    /// Working / finished / error / approval — the island's vocabulary.
    pub state: String,
}

#[derive(Default, Clone, Serialize, serde::Deserialize)]
pub struct DayStats {
    pub agents: BTreeMap<String, AgentStats>,
}

#[derive(Default, Clone, Serialize, serde::Deserialize)]
pub struct Stats {
    /// "YYYY-MM-DD" of the rolling day, local time.
    pub today: String,
    pub days: BTreeMap<String, DayStats>,
}

static STATS: Mutex<Option<Stats>> = Mutex::new(None);
/// Counters are cheap; disk writes are not — flush every so many records and
/// on the events that end something (Stop, decisions, errors).
const FLUSH_EVERY: u64 = 25;
static SINCE_FLUSH: Mutex<u64> = Mutex::new(0);

fn stats_path() -> std::path::PathBuf {
    crate::settings::local_dir().join("stats.json")
}

fn load() -> Stats {
    let mut guard = STATS.lock().unwrap();
    if guard.is_none() {
        let loaded: Stats = std::fs::read(stats_path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        *guard = Some(loaded);
    }
    guard.as_ref().unwrap().clone()
}

fn today() -> String {
    let t = crate::log::LocalTime::now();
    format!("{:04}-{:02}-{:02}", t.year, t.month, t.day)
}

/// One relay event, folded in. `extra` carries the decision for
/// PermissionRequest outcomes ("allow" / "deny").
pub fn record(agent: &str, event: &str, extra: Option<&str>) {
    if agent.is_empty() {
        return;
    }
    let day = today();
    {
        let mut guard = STATS.lock().unwrap();
        let stats = guard.get_or_insert_with(Stats::default);
        if stats.today != day {
            stats.today = day.clone();
        }
        let entry = stats.days.entry(day).or_default().agents.entry(agent.to_string()).or_default();
        if event == "Decision" {
            // The outcome of an approval already counted on arrival — only
            // the decision counters move, and the state stays as it was.
            if let Some(decision) = extra {
                match decision {
                    "allow" => entry.allows += 1,
                    "deny" => entry.denies += 1,
                    _ => {}
                }
            }
            drop(guard);
            persist();
            return;
        }
        match event {
            "SessionStart" => entry.sessions += 1,
            "PreToolUse" => entry.tool_calls += 1,
            "StopFailure" => entry.errors += 1,
            "PermissionRequest" => entry.approvals += 1,
            _ => {}
        }
        entry.last_event = event.to_string();
        entry.state = match event {
            "SessionStart" | "UserPromptSubmit" => "working".into(),
            "PreToolUse" | "PostToolUse" | "PostToolUseFailure" => "working".into(),
            "PermissionRequest" => "approval".into(),
            "Stop" => "finished".into(),
            "StopFailure" => "error".into(),
            "SessionEnd" => "idle".into(),
            _ => "working".into(),
        };
    }

    let flush = {
        let mut since = SINCE_FLUSH.lock().unwrap();
        *since += 1;
        *since >= FLUSH_EVERY || matches!(event, "Stop" | "StopFailure" | "SessionEnd")
    };
    if flush {
        persist();
    }
}

pub fn persist() {
    *SINCE_FLUSH.lock().unwrap() = 0;
    let guard = STATS.lock().unwrap();
    if let Some(stats) = guard.as_ref() {
        // Keep the last 60 days — a companion, not a census.
        let mut stats = stats.clone();
        let mut days: Vec<String> = stats.days.keys().cloned().collect();
        days.sort();
        while days.len() > 60 {
            let oldest = days.remove(0);
            stats.days.remove(&oldest);
        }
        if let Ok(json) = serde_json::to_vec_pretty(&stats) {
            let path = stats_path();
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(&path, json);
        }
    }
}

/// The snapshot behind the Stats section and the MCP tools: the last `days`
/// days, today first.
/// Today's totals as a CheckContext for the achievement system.
pub fn today_context() -> crate::achievements::CheckContext {
    let stats = load();
    let day = today();
    let mut ctx = crate::achievements::CheckContext::default();
    if let Some(today) = stats.days.get(&day) {
        for (agent, s) in &today.agents {
            ctx.total_sessions += s.sessions;
            ctx.total_tool_calls += s.tool_calls;
            ctx.total_approvals += s.approvals;
            ctx.total_tokens += s.tool_calls * 2_000; // rough per-call estimate
            ctx.agents_today.push(agent.clone());
        }
    }
    let t = crate::log::LocalTime::now();
    ctx.is_night = t.hour >= 0 && t.hour < 5;
    ctx.is_early = t.hour >= 5 && t.hour < 7;
    ctx
}

pub fn snapshot_json(days: usize) -> Value {
    let stats = load();
    let mut keys: Vec<&String> = stats.days.keys().collect();
    keys.sort();
    keys.reverse();
    let picked: Vec<&&String> = keys.iter().take(days).collect();
    let mut out = serde_json::Map::new();
    for key in picked {
        out.insert((*key).clone(), serde_json::to_value(&stats.days[*key]).unwrap_or_default());
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_fold_into_states_and_decisions() {
        // Same lock discipline as the filesystem tests: one stats test at a time.
        let _lock = crate::TEST_HOME_LOCK.lock().unwrap();
        let tmp = std::env::temp_dir().join(format!("atlas-stats-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        #[cfg(unix)]
        std::env::set_var("HOME", &tmp);
        #[cfg(windows)]
        std::env::set_var("USERPROFILE", &tmp);
        *STATS.lock().unwrap() = None;
        *SINCE_FLUSH.lock().unwrap() = 0;

        record("codex", "SessionStart", None);
        record("codex", "PreToolUse", None);
        record("codex", "PreToolUse", None);
        record("codex", "PermissionRequest", None);
        record("codex", "Decision", Some("allow"));
        record("zcode", "PreToolUse", None);
        record("zcode", "StopFailure", None);

        let snap = snapshot_json(1);
        let today_key = snap.as_object().unwrap().keys().next().unwrap().clone();
        let codex = &snap[&today_key]["agents"]["codex"];
        assert_eq!(codex["sessions"], 1);
        assert_eq!(codex["tool_calls"], 2);
        assert_eq!(codex["approvals"], 1);
        assert_eq!(codex["allows"], 1);
        assert_eq!(codex["state"], "approval");
        let zcode = &snap[&today_key]["agents"]["zcode"];
        assert_eq!(zcode["errors"], 1);
        assert_eq!(zcode["state"], "error");

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
