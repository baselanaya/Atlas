//! The island's voice: when Voicebox (github.com/jamiepine/voicebox) is
//! running, Atlas can speak — chat replies read aloud, approval requests and
//! finished sessions announced — through its local REST API on
//! 127.0.0.1:17493.
//!
//! Nothing here requires Voicebox: detection is a cheap cached probe, and
//! every call fails quietly when the studio isn't there. No other TTS engine
//! is silently spawned — the user picked the voice they want, in Voicebox.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

const BASE: &str = "http://127.0.0.1:17493";
/// Identifies us to Voicebox, so its per-client voice bindings can target
/// Atlas specifically.
const CLIENT: &str = "atlas";
/// Re-probe at most this often; a miss is remembered too, so a closed
/// Voicebox costs one fast failed request per minute.
const PROBE_TTL: Duration = Duration::from_secs(60);

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct VoiceStatus {
    pub available: bool,
    pub profiles: Vec<String>,
}

#[derive(Default)]
struct Cache {
    probed_at: Option<Instant>,
    status: VoiceStatus,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

/// The profile names from Voicebox, however it chooses to shape the list.
fn parse_profiles(body: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<Value>(body) else { return Vec::new() };
    let list = match &v {
        Value::Array(items) => items.as_slice(),
        Value::Object(map) => map
            .get("profiles")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]),
        _ => &[],
    };
    list.iter()
        .filter_map(|p| {
            ["name", "id", "profile_id", "profileName"]
                .iter()
                .find_map(|k| p.get(*k).and_then(Value::as_str))
                .map(str::to_string)
        })
        .collect()
}

async fn probe(force: bool) -> VoiceStatus {
    {
        let guard = CACHE.lock().unwrap();
        if !force {
            if let Some(cache) = guard.as_ref() {
                if let Some(at) = cache.probed_at {
                    if at.elapsed() < PROBE_TTL {
                        return cache.status.clone();
                    }
                }
            }
        }
    }

    let status = match reqwest::Client::builder()
        .timeout(Duration::from_millis(700))
        .build()
    {
        Ok(client) => match client.get(format!("{BASE}/profiles")).send().await {
            Ok(resp) if resp.status().is_success() => {
                let body = resp.text().await.unwrap_or_default();
                VoiceStatus { available: true, profiles: parse_profiles(&body) }
            }
            _ => VoiceStatus { available: false, profiles: Vec::new() },
        },
        Err(_) => VoiceStatus { available: false, profiles: Vec::new() },
    };

    *CACHE.lock().unwrap() = Some(Cache { probed_at: Some(Instant::now()), status: status.clone() });
    status
}

pub async fn status() -> VoiceStatus {
    probe(false).await
}

/// Says `text` out loud. Quietly does nothing when Voicebox is closed — a
/// missing studio is not an error the island should ever show.
pub async fn speak(text: &str, profile: &str) {
    let text: String = text.chars().take(240).collect();
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
    else {
        return;
    };
    let mut body = serde_json::json!({ "text": text });
    if !profile.is_empty() {
        body["profile_id"] = Value::String(profile.to_string());
    }
    let _ = client
        .post(format!("{BASE}/speak"))
        .header("X-Voicebox-Client-Id", CLIENT)
        .json(&body)
        .send()
        .await;
}

/// Audio → text, for the chat bubble's mic button. `audio_b64` is raw
/// recorder output (webm/ogg), forwarded to Voicebox as-is.
pub async fn transcribe(audio_b64: &str) -> Result<String, String> {
    let bytes = decode64(audio_b64)?;
    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name("atlas.webm")
        .mime_str("audio/webm")
        .map_err(|e| e.to_string())?;
    let form = reqwest::multipart::Form::new().part("file", part);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .post(format!("{BASE}/transcribe"))
        .header("X-Voicebox-Client-Id", CLIENT)
        .multipart(form)
        .send()
        .await
        .map_err(|_| "Voicebox isn't running.".to_string())?;
    if !resp.status().is_success() {
        return Err(format!("Voicebox transcribe failed ({})", resp.status()));
    }
    let v: Value = resp.json().await.map_err(|e| e.to_string())?;
    let text = ["text", "transcript", "content"]
        .iter()
        .find_map(|k| v.get(*k).and_then(Value::as_str))
        .unwrap_or("")
        .trim()
        .to_string();
    if text.is_empty() {
        return Err("Voicebox returned no transcript.".into());
    }
    Ok(text)
}

/// Standard-alphabet base64 decode — the mirror of claude.rs's encoder,
/// still not worth a dependency.
fn decode64(input: &str) -> Result<Vec<u8>, String> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    let mut buf: u32 = 0;
    let mut bits = 0u32;
    for ch in input.bytes() {
        if ch == b'=' || ch == b'\n' || ch == b'\r' {
            continue;
        }
        let val = TABLE
            .iter()
            .position(|&t| t == ch)
            .ok_or_else(|| "bad audio payload".to_string())? as u32;
        buf = (buf << 6) | val;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{decode64, parse_profiles};

    #[test]
    fn decode64_round_trips_the_encoder_contract() {
        assert_eq!(decode64("").unwrap(), b"");
        assert_eq!(decode64("Zg==").unwrap(), b"f");
        assert_eq!(decode64("Zm9v").unwrap(), b"foo");
        assert!(decode64("?").is_err());
    }

    #[test]
    fn profile_lists_parse_from_the_shapes_voicebox_might_use() {
        assert_eq!(parse_profiles(r#"[{"name":"Aria"},{"name":"Moth"}]"#), ["Aria", "Moth"]);
        assert_eq!(parse_profiles(r#"[{"id":"v1"},{"id":"v2"}]"#), ["v1", "v2"]);
        assert_eq!(
            parse_profiles(r#"{"profiles":[{"profileName":"Kai"}]}"#),
            ["Kai".to_string()].as_slice()
        );
        assert!(parse_profiles("not json").is_empty());
        assert!(parse_profiles("[]").is_empty());
    }
}
