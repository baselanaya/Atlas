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

/// The desktop app serves 17493; the docker compose maps the same API to
/// 17600 on the host. Whichever answers first wins, remembered for the call.
const BASES: [&str; 2] = ["http://127.0.0.1:17493", "http://127.0.0.1:17600"];
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

    let mut status = VoiceStatus { available: false, profiles: Vec::new() };
    if let Ok(client) = reqwest::Client::builder().timeout(Duration::from_millis(700)).build() {
        for base in BASES {
            if let Ok(resp) = client.get(format!("{base}/profiles")).send().await {
                if resp.status().is_success() {
                    let body = resp.text().await.unwrap_or_default();
                    status = VoiceStatus { available: true, profiles: parse_profiles(&body) };
                    break;
                }
            }
        }
    }

    *CACHE.lock().unwrap() = Some(Cache { probed_at: Some(Instant::now()), status: status.clone() });
    status
}

pub async fn status() -> VoiceStatus {
    probe(false).await
}

/// One speaker at a time: a second line while the first is still generating
/// would rather skip than pile up — announcements are news, not a queue.
/// tokio's mutex, because the guard lives across awaits.
static SPEAKING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Says `text` out loud, end to end: make room on the GPU, hand the line to
/// Voicebox, wait for the audio, play it on the host. Quietly does nothing
/// when Voicebox is closed — a missing studio is not an error the island
/// should ever show.
pub async fn speak(text: &str, profile: &str, output_dir: &str) {
    let text: String = text.chars().take(240).collect();
    if let Ok(_turn) = SPEAKING.try_lock() {
        speak_locked(&text, profile, output_dir).await;
    }

}

async fn speak_locked(text: &str, profile: &str, output_dir: &str) {
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
    else {
        return;
    };
    let base = match alive_base(&client).await {
        Some(base) => base,
        None => return,
    };

    // An 8 GB card holds one engine: clear whoever else is resident before
    // this line needs the GPU.
    if let Some(engine) = profile_engine(&client, &base, profile).await {
        make_engine_room(&client, &base, &engine).await;
    }

    let mut body = serde_json::json!({ "text": text });
    if !profile.is_empty() {
        // The API resolves `profile` by name or id — `profile_id` alone is a 400.
        body["profile"] = Value::String(profile.to_string());
    }
    let Ok(resp) = client
        .post(format!("{base}/speak"))
        .header("X-Voicebox-Client-Id", CLIENT)
        .json(&body)
        .send()
        .await
    else {
        return;
    };
    if !resp.status().is_success() {
        return;
    }
    let Ok(v) = resp.json::<Value>().await else { return };
    let Some(id) = v.get("id").and_then(Value::as_str) else { return };

    // The container has no audio stack of its own — Atlas plays the finished
    // WAV on the host, from the directory Voicebox's bind mount writes to.
    // The file is the truth: it appears the moment generation lands, no SSE
    // racing, and its name is the generation id.
    if output_dir.is_empty() {
        return;
    }
    let path = std::path::Path::new(output_dir).join(format!("{id}.wav"));
    let deadline = Instant::now() + Duration::from_secs(240);
    while Instant::now() < deadline {
        match std::fs::metadata(&path) {
            Ok(m) if m.len() > 1024 => {
                play_host(&path);
                return;
            }
            _ => tokio::time::sleep(Duration::from_millis(400)).await,
        }
    }
    crate::log::line(format!("voice: audio for {id} never landed in {output_dir}"));
}

/// The first Voicebox base that answers; both stay supported so the docker
/// deployment and a desktop install can coexist.
async fn alive_base(client: &reqwest::Client) -> Option<&'static str> {
    for base in BASES {
        if let Ok(resp) = client.get(format!("{base}/profiles")).send().await {
            if resp.status().is_success() {
                return Some(base);
            }
        }
    }
    None
}

/// The engine a profile uses, from its own metadata.
async fn profile_engine(client: &reqwest::Client, base: &str, profile: &str) -> Option<String> {
    if profile.is_empty() {
        return None;
    }
    let resp = client.get(format!("{base}/profiles")).send().await.ok()?;
    let v: Value = resp.json().await.ok()?;
    v.as_array()?
        .iter()
        .find(|p| p.get("name").and_then(Value::as_str) == Some(profile))
        .and_then(|p| p.get("default_engine"))?
        .as_str()
        .map(str::to_string)
}

/// Unloads every resident model that is not the engine we are about to use
/// (Whisper included in the "leave alone" set — it is tiny and it transcribes).
async fn make_engine_room(client: &reqwest::Client, base: &str, keep: &str) {
    let Ok(resp) = client.get(format!("{base}/models/status")).send().await else { return };
    let Ok(v) = resp.json::<Value>().await else { return };
    let Some(models) = v.get("models").and_then(Value::as_array) else { return };
    for m in models {
        let loaded = m.get("loaded").and_then(Value::as_bool).unwrap_or(false);
        let Some(name) = m.get("model_name").and_then(Value::as_str) else { continue };
        if !loaded || name.contains("whisper") || name.starts_with(keep) {
            continue;
        }
        let _ = client
            .post(format!("{base}/models/{name}/unload"))
            .send()
            .await;
        crate::log::line(format!("voice: unloaded {name} to make room for {keep}"));
    }
}


/// Trims trailing silence from a 16-bit PCM WAV — the TTS engines pad
/// their output (measured: 0.46s on a 1.73s clip, 27% dead air). Finds the
/// last audible sample, keeps a 60ms tail for naturalness, rewrites the file.
/// Any parse failure leaves the original untouched.
fn trim_trailing_silence(path: &std::path::Path) {
    let Ok(bytes) = std::fs::read(path) else {
        crate::log::line("voice: trim — could not read file".to_string());
        return;
    };
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return;
    }

    // Walk the chunks to find "fmt " and "data".
    let mut pos = 12;
    let mut channels: u16 = 0;
    let mut rate: u32 = 0;
    let mut bits: u16 = 0;
    let mut data_start: usize = 0;
    let mut data_len: usize = 0;
    while pos + 8 <= bytes.len() {
        let chunk_id = &bytes[pos..pos + 4];
        let chunk_len = u32::from_le_bytes([bytes[pos + 4], bytes[pos + 5], bytes[pos + 6], bytes[pos + 7]]) as usize;
        if chunk_id == b"fmt " && pos + 8 + 16 <= bytes.len() {
            channels = u16::from_le_bytes([bytes[pos + 10], bytes[pos + 11]]);
            rate = u32::from_le_bytes([bytes[pos + 12], bytes[pos + 13], bytes[pos + 14], bytes[pos + 15]]);
            bits = u16::from_le_bytes([bytes[pos + 22], bytes[pos + 23]]);
        } else if chunk_id == b"data" {
            data_start = pos + 8;
            data_len = chunk_len.min(bytes.len() - data_start);
            break;
        }
        pos += 8 + chunk_len + (chunk_len & 1); // chunks are word-aligned
    }

    // Only trim what we understand: 16-bit PCM.
    if bits != 16 || channels == 0 || rate == 0 || data_len < 4 {
        return;
    }

    let bytes_per_frame = (bits / 8) as usize * channels as usize;
    let frames = data_len / bytes_per_frame;
    let samples: &[i16] = unsafe {
        std::slice::from_raw_parts(bytes[data_start..].as_ptr() as *const i16, data_len / 2)
    };

    // Find the last frame where any channel exceeds the threshold.
    const THRESHOLD: i16 = 300;
    let mut last_audible_frame = 0;
    for frame in (0..frames).rev() {
        let base = frame * channels as usize;
        if (0..channels as usize).any(|c| samples[base + c].abs() > THRESHOLD) {
            last_audible_frame = frame;
            break;
        }
    }

    // Keep 60ms of tail past the last audible frame, but never more than
    // the original.
    let tail_frames = (rate as usize * 60 / 1000).max(1);
    let keep_frames = (last_audible_frame + tail_frames).min(frames);
    if keep_frames >= frames {
        crate::log::line("voice: trim — nothing to trim".to_string());
        return;
    }
    crate::log::line(format!(
        "voice: trimming {} → {} frames ({}s → {}s)",
        frames, keep_frames, frames as f64 / rate as f64, keep_frames as f64 / rate as f64
    ));

    let keep_bytes = keep_frames * bytes_per_frame;
    let trimmed_len = data_start + keep_bytes;
    let mut out = bytes[..trimmed_len].to_vec();
    // Fix the RIFF and data chunk sizes.
    let total = (trimmed_len - 8) as u32;
    out[4..8].copy_from_slice(&total.to_le_bytes());
    let dlen = keep_bytes as u32;
    let data_size_pos = data_start - 4;
    out[data_size_pos..data_size_pos + 4].copy_from_slice(&dlen.to_le_bytes());

    // The original belongs to the container's user and can't be modified;
    // write a trimmed copy beside it (the directory is world-writable).
    let trimmed_path = path.with_extension("trimmed.wav");
    match std::fs::write(&trimmed_path, &out) {
        Ok(()) => {
            crate::log::line(format!("voice: trimmed copy at {}", trimmed_path.display()));
        }
        Err(err) => {
            crate::log::line(format!("voice: trim write failed: {err}"));
        }
    }
}

/// Plays the finished line through the host's audio stack. PipeWire first,
/// PulseAudio fallback — the same stack the desktop already uses.
fn play_host(path: &std::path::Path) {
    if !path.exists() {
        return;
    }
    trim_trailing_silence(path);
    // The trimmer writes a `.trimmed.wav` beside the original when it can't
    // overwrite the container's file; prefer it when present.
    let trimmed = path.with_extension("trimmed.wav");
    let play_path = if trimmed.exists() { &trimmed } else { path };
    #[cfg(unix)]
    {
        for player in ["pw-play", "paplay"] {
            if let Ok(mut child) = std::process::Command::new(player)
                .arg(play_path)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
            {
                // Reap what we spawn: an island that never waits for its own
                // children fills the process table with zombies.
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                return;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

/// Audio → text, for the chat bubble's mic button. `audio_b64` is raw
/// recorder output (webm/ogg), forwarded to Voicebox as-is.
pub async fn transcribe(audio_b64: &str) -> Result<String, String> {
    let bytes = decode64(audio_b64)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let mut last_err = String::from("Voicebox isn't running.");
    for base in BASES {
        let part = reqwest::multipart::Part::bytes(bytes.clone())
            .file_name("atlas.webm")
            .mime_str("audio/webm")
            .map_err(|e| e.to_string())?;
        let form = reqwest::multipart::Form::new().part("file", part);
        match client
            .post(format!("{base}/transcribe"))
            .header("X-Voicebox-Client-Id", CLIENT)
            .multipart(form)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                let v: Value = resp.json().await.map_err(|e| e.to_string())?;
                return transcribe_value(v);
            }
            Ok(resp) => {
                last_err = format!("Voicebox transcribe failed ({})", resp.status());
            }
            Err(_) => continue,
        }
    }
    return Err(last_err);
}

fn transcribe_value(v: Value) -> Result<String, String> {
    let text = ["text", "transcript", "content"]
        .iter()
        .find_map(|k| v.get(*k).and_then(Value::as_str))
        .unwrap_or("")
        .trim()
        .to_string();
    if text.is_empty() {
        return Err("Voicebox returned no transcript.".to_string());
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
