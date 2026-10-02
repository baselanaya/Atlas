//! Achievements — milestones the island celebrates. Stored alongside the
//! daily stats, checked on every relay event, announced through the
//! character's emote system and (if voice is on) spoken.
//!
//! The design principle: achievements are earned by *doing*, not by waiting.
//! Every one is reachable in a normal working day, and none require the
//! island to be watched — they fire whenever the milestone crosses.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Achievement {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub emote: &'static str,
    pub sound: &'static str,
}

/// The catalogue, in unlock order. Checked top to bottom; first new hit wins
/// the celebration (one at a time — a wall of toasts is noise, not joy).
const CATALOGUE: &[Achievement] = &[
    Achievement { id: "first_session", name: "First Contact", description: "Your first agent session through the island", emote: "love", sound: "greet" },
    Achievement { id: "ten_sessions", name: "Getting Serious", description: "Ten sessions watched", emote: "proud", sound: "proud" },
    Achievement { id: "hundred_tools", name: "Tool Collector", description: "One hundred tool calls across all agents", emote: "wink", sound: "pop" },
    Achievement { id: "thousand_tokens", name: "Kilo-tokens", description: "1,000 tokens processed in a day", emote: "surprised", sound: "blip" },
    Achievement { id: "hundred_k_tokens", name: "Centi-kilo", description: "100,000 tokens in a day", emote: "love", sound: "finish" },
    Achievement { id: "million_tokens", name: "Millionaire", description: "1,000,000 tokens in a day — the heavy hitter", emote: "proud", sound: "finish" },
    Achievement { id: "first_approval", name: "Trust Exercise", description: "First permission approved from the island", emote: "wink", sound: "approve" },
    Achievement { id: "ten_approvals", name: "Gatekeeper", description: "Ten permissions decided", emote: "proud", sound: "proud" },
    Achievement { id: "triple_agent", name: "Air Traffic", description: "All three agents ran today", emote: "surprised", sound: "open" },
    Achievement { id: "night_owl", name: "Night Owl", description: "A session after midnight", emote: "yawn", sound: "sleep" },
    Achievement { id: "early_bird", name: "Early Bird", description: "A session before 7 AM", emote: "love", sound: "greet" },
    Achievement { id: "first_voice", name: "Finding Voice", description: "The island spoke for the first time", emote: "surprised", sound: "blip" },
];

#[derive(Default, Clone, Serialize, serde::Deserialize)]
pub struct AchievementState {
    pub unlocked: BTreeMap<String, String>, // id → date unlocked (YYYY-MM-DD)
}

impl AchievementState {
    fn path() -> std::path::PathBuf {
        crate::settings::local_dir().join("achievements.json")
    }

    fn load() -> AchievementState {
        std::fs::read(Self::path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn save(&self) {
        if let Ok(json) = serde_json::to_vec_pretty(self) {
            let path = Self::path();
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(&path, json);
        }
    }
}

/// The context from the current event/stats, checked against the catalogue.
#[derive(Default)]
pub struct CheckContext {
    pub total_sessions: u64,
    pub total_tool_calls: u64,
    pub total_approvals: u64,
    pub total_tokens: u64,
    pub agents_today: Vec<String>,
    pub is_night: bool,
    pub is_early: bool,
    pub voice_spoken: bool,
}

/// Checks all achievements, saves state, and returns the newly-unlocked one
/// (if any) so the caller can fire the emote and sound.
pub fn check(ctx: &CheckContext) -> Option<Achievement> {
    let mut state = AchievementState::load();
    let today = {
        let t = crate::log::LocalTime::now();
        format!("{:04}-{:02}-{:02}", t.year, t.month, t.day)
    };

    for ach in CATALOGUE {
        if state.unlocked.contains_key(ach.id) {
            continue;
        }
        let earned = match ach.id {
            "first_session" => ctx.total_sessions >= 1,
            "ten_sessions" => ctx.total_sessions >= 10,
            "hundred_tools" => ctx.total_tool_calls >= 100,
            "thousand_tokens" => ctx.total_tokens >= 1_000,
            "hundred_k_tokens" => ctx.total_tokens >= 100_000,
            "million_tokens" => ctx.total_tokens >= 1_000_000,
            "first_approval" => ctx.total_approvals >= 1,
            "ten_approvals" => ctx.total_approvals >= 10,
            "triple_agent" => ctx.agents_today.len() >= 3,
            "night_owl" => ctx.is_night,
            "early_bird" => ctx.is_early,
            "first_voice" => ctx.voice_spoken,
            _ => false,
        };
        if earned {
            state.unlocked.insert(ach.id.to_string(), today.clone());
            state.save();
            return Some(ach.clone());
        }
    }
    None
}

/// The full list with unlocked status, for the settings window.
pub fn list() -> Value {
    let state = AchievementState::load();
    let today = {
        let t = crate::log::LocalTime::now();
        format!("{:04}-{:02}-{:02}", t.year, t.month, t.day)
    };
    let mut out: Vec<Value> = CATALOGUE
        .iter()
        .map(|a| {
            let unlocked = state.unlocked.get(a.id).cloned();
            serde_json::json!({
                "id": a.id,
                "name": a.name,
                "description": a.description,
                "emote": a.emote,
                "sound": a.sound,
                "unlocked": unlocked,
                "isNew": unlocked.as_deref() == Some(&today),
            })
        })
        .collect();
    // Unlocked first, then locked, each in catalogue order.
    out.sort_by(|a, b| {
        let au = a["unlocked"].is_string();
        let bu = b["unlocked"].is_string();
        bu.cmp(&au)
    });
    Value::Array(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_session_unlocks_then_never_again() {
        let _lock = crate::TEST_HOME_LOCK.lock().unwrap();
        let tmp = std::env::temp_dir().join(format!("atlas-ach-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        #[cfg(unix)]
        std::env::set_var("HOME", &tmp);

        let ctx = CheckContext { total_sessions: 1, ..Default::default() };
        let first = check(&ctx);
        assert!(first.is_some(), "the very first session must unlock");
        assert_eq!(first.unwrap().id, "first_session");

        // Already unlocked — nothing fires.
        let again = check(&ctx);
        assert!(again.is_none(), "an earned achievement stays earned");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn token_tiers_escalate() {
        let _lock = crate::TEST_HOME_LOCK.lock().unwrap();
        let tmp = std::env::temp_dir().join(format!("atlas-ach-t-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        #[cfg(unix)]
        std::env::set_var("HOME", &tmp);

        let ctx = CheckContext { total_tokens: 150_000, ..Default::default() };
        let hit = check(&ctx).expect("150k tokens should unlock something");
        assert_eq!(hit.id, "thousand_tokens", "tiers unlock in order");

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
