//! Real token usage, read from where the agents actually write it: their
//! session transcripts. Claude Code's JSONL logs one `usage` block per
//! assistant message (incremental — sum them); Codex's rollouts log
//! `token_count` events that are cumulative per session (take the last per
//! file, day from the path). zcode keeps no readable transcripts — it simply
//! reports nothing here rather than guessing.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

#[derive(Default, Clone, Copy, Serialize, PartialEq, Debug)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
}

impl Tokens {
    fn add(&mut self, v: &Value) {
        let get = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
        self.input += get("input_tokens");
        self.output += get("output_tokens");
        self.cache_read += get("cached_input_tokens").max(get("cache_read_input_tokens"));
    }
}

pub type TokenDays = BTreeMap<String, BTreeMap<String, Tokens>>;

static CACHE: Mutex<Option<(Instant, usize, TokenDays)>> = Mutex::new(None);

/// The last `days` days of per-agent token usage, cached for a minute —
/// transcripts are not small and the stats view is not in a hurry.
pub fn scan(days: usize) -> TokenDays {
    {
        let guard = CACHE.lock().unwrap();
        if let Some((at, cached_days, map)) = guard.as_ref() {
            if at.elapsed() < Duration::from_secs(60) && *cached_days == days {
                return map.clone();
            }
        }
    }

    let mut out: TokenDays = BTreeMap::new();
    let cutoff = cutoff_day(days);
    scan_claude(&mut out, &cutoff);
    scan_codex(&mut out, &cutoff);
    out.retain(|_, agents| !agents.is_empty());

    *CACHE.lock().unwrap() = Some((Instant::now(), days, out.clone()));
    out
}

/// "YYYY-MM-DD" of `days` days ago, local time.
fn cutoff_day(days: usize) -> String {
    let t = crate::log::LocalTime::now();
    let (y, m, d) = days_back(t.year, t.month as i32, t.day as i32, days.saturating_sub(1));
    format!("{y:04}-{m:02}-{d:02}")
}

fn days_back(mut y: i32, mut m: i32, mut d: i32, back: usize) -> (i32, i32, i32) {
    for _ in 0..back {
        d -= 1;
        if d == 0 {
            m -= 1;
            if m == 0 {
                y -= 1;
                m = 12;
            }
            d = match m {
                1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
                4 | 6 | 9 | 11 => 30,
                2 => {
                    if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) { 29 } else { 28 }
                }
                _ => 31,
            };
        }
    }
    (y, m, d)
}

fn scan_claude(out: &mut TokenDays, cutoff: &String) {
    let root = home().join(".claude").join("projects");
    let Ok(entries) = std::fs::read_dir(&root) else { return };
    for project in entries.flatten() {
        let Ok(files) = std::fs::read_dir(project.path()) else { continue };
        for file in files.flatten() {
            let path = file.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            for line in text.lines() {
                let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
                let Some(usage) = v.get("message").and_then(|m| m.get("usage")) else { continue };
                let day = v
                    .get("timestamp")
                    .and_then(Value::as_str)
                    .map(|ts| ts.get(..10).unwrap_or("").to_string())
                    .unwrap_or_default();
                if day < *cutoff {
                    continue;
                }
                let t = out.entry(day).or_default().entry("claude".into()).or_default();
                t.add(usage);
            }
        }
    }
}

fn scan_codex(out: &mut TokenDays, cutoff: &String) {
    let root = home().join(".codex").join("sessions");
    // sessions/YYYY/MM/DD/rollout-*.jsonl — the path is the day.
    let Ok(years) = std::fs::read_dir(&root) else { return };
    for y in years.flatten() {
        let Ok(months) = std::fs::read_dir(y.path()) else { continue };
        for m in months.flatten() {
            let Ok(days_) = std::fs::read_dir(m.path()) else { continue };
            for d in days_.flatten() {
                let day = format!(
                    "{}-{}-{}",
                    y.file_name().to_string_lossy(),
                    m.file_name().to_string_lossy(),
                    d.file_name().to_string_lossy()
                );
                if day.len() != 10 || day < *cutoff {
                    continue;
                }
                let Ok(files) = std::fs::read_dir(d.path()) else { continue };
                for file in files.flatten() {
                    let path = file.path();
                    if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                        continue;
                    }
                    let Ok(text) = std::fs::read_to_string(&path) else { continue };
                    // Cumulative counters: the last record in the file wins.
                    let mut last: Option<Value> = None;
                    for line in text.lines().rev() {
                        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
                        if let Some(found) = v
                            .pointer("/payload/info/total_token_usage")
                            .or_else(|| v.pointer("/info/total_token_usage"))
                            .or_else(|| v.get("total_token_usage"))
                        {
                            last = Some(found.clone());
                            break;
                        }
                    }
                    if let Some(usage) = last {
                        let t = out.entry(day.clone()).or_default().entry("codex".into()).or_default();
                        t.add(&usage);
                    }
                }
            }
        }
    }
}

fn home() -> PathBuf {
    #[cfg(windows)]
    let key = "USERPROFILE";
    #[cfg(unix)]
    let key = "HOME";
    std::env::var_os(key).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_usage_parses_from_both_agents_shapes() {
        let claude_line = r#"{"timestamp":"2026-10-02T10:00:00Z","message":{"usage":{"input_tokens":100,"output_tokens":50,"cache_read_input_tokens":0}}}"#;
        let v: Value = serde_json::from_str(claude_line).unwrap();
        let mut t = Tokens::default();
        t.add(v.pointer("/message/usage").unwrap());
        assert_eq!(t, Tokens { input: 100, output: 50, cache_read: 0 });

        let codex_line = r#"{"type":"token_count","info":{"total_token_usage":{"input_tokens":18194,"cached_input_tokens":7936,"output_tokens":6}}}"#;
        let v: Value = serde_json::from_str(codex_line).unwrap();
        let mut t = Tokens::default();
        t.add(v.pointer("/info/total_token_usage").unwrap());
        assert_eq!(t, Tokens { input: 18194, output: 6, cache_read: 7936 });
    }

    #[test]
    fn cutoff_walks_back_over_month_ends() {
        assert_eq!(days_back(2026, 10, 2, 2), (2026, 9, 30));
        assert_eq!(days_back(2026, 3, 1, 2), (2026, 2, 27));
        assert_eq!(days_back(2024, 3, 1, 1), (2024, 2, 29), "leap year");
        assert_eq!(days_back(2026, 1, 1, 1), (2025, 12, 31), "year boundary");
    }
}
