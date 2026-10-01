// Claude API client — the same integration as ClaudeService.swift: multi-turn
// chat with web search, and files sent as document/image/text blocks.
//
// Everything happens here rather than in the island: the API key never leaves
// the Credential Manager, and file bytes never cross the IPC boundary.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::secrets;

const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Server-side fallback: on a policy decline the API retries the same request on
/// a fallback model inside the same call, so the island never shows a dead end.
/// Anthropic-only.
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
const MAX_TOKENS: u32 = 4096;
/// Text and code files are inlined; anything larger is skipped, as on macOS.
const MAX_INLINE_TEXT: u64 = 200_000;

pub const DEFAULT_MODEL: &str = "claude-opus-5";
/// The chat speaks the Anthropic Messages API, wherever it is served. Z.AI's
/// GLM endpoint (https://api.z.ai/api/anthropic) is compatible enough for the
/// chat: same request shape, same auth — only the server-side web search tool
/// and the fallback beta are Anthropic's and are left off elsewhere.
pub const DEFAULT_API_BASE: &str = "https://api.anthropic.com";

const SYSTEM_PROMPT: &str = "You are Atlas, a personal AI assistant living at the top of the user's screen. \
You have web search access and can help with absolutely anything — research, coding, finding places, recommendations, tasks, questions. \
Respond in the user's language. Be thorough and complete — use as much detail as the task requires. \
No markdown formatting (no **, no ##, no bullet dashes). Use plain text with line breaks.";

#[derive(Default)]
pub struct Chat {
    /// Full multi-turn history, including tool_use / tool_result blocks.
    messages: Mutex<Vec<Value>>,
    /// Plain (question, answer) turns for the CLI routes, where there are no
    /// content blocks to preserve.
    turns: Mutex<Vec<(String, String)>>,
}

impl Chat {
    pub fn reset(&self) {
        self.messages.lock().unwrap().clear();
        self.turns.lock().unwrap().clear();
    }

    fn is_empty(&self) -> bool {
        self.messages.lock().unwrap().is_empty()
    }

    fn push(&self, message: Value) {
        self.messages.lock().unwrap().push(message);
    }

    fn pop(&self) {
        self.messages.lock().unwrap().pop();
    }

    fn snapshot(&self) -> Vec<Value> {
        self.messages.lock().unwrap().clone()
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ChatContext {
    File { name: String, path: String },
    Window { app_name: String, title: String, url: Option<String> },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatReply {
    pub text: String,
}

/// One chat turn, routed by settings: the Messages API, or a logged-in CLI
/// agent answering with the subscription the user already pays for. Returns
/// the assistant's text, or a message the island shows in the note view.
pub async fn send(
    chat: &Chat,
    model: &str,
    api_base: &str,
    route: &str,
    query: String,
    context: Option<ChatContext>,
) -> Result<ChatReply, String> {
    match route {
        "codex" => return cli_turn(chat, "codex", &query).await,
        "claude" => return cli_turn(chat, "claude", &query).await,
        _ => {}
    }
    let key = secrets::get("anthropic-api-key")
        .ok_or_else(|| "API key missing. Open settings.".to_string())?;

    let mut content: Vec<Value> = Vec::new();

    // File / window context rides along with the first message only, exactly
    // like ClaudeService.chat().
    if chat.is_empty() {
        match &context {
            Some(ChatContext::File { name, path }) => {
                if let Some(block) = file_block(path) {
                    content.push(block);
                }
                content.push(json!({ "type": "text", "text": format!("File: {name}") }));
            }
            Some(ChatContext::Window { app_name, title, url }) => {
                let mut text = format!("Context — App: {app_name}, Window: {title}");
                if let Some(url) = url {
                    text.push_str(&format!(", URL: {url}"));
                }
                content.push(json!({ "type": "text", "text": text }));
            }
            None => {}
        }
    }
    content.push(json!({ "type": "text", "text": query }));

    chat.push(json!({ "role": "user", "content": content }));

    // The server-side web search tool is Anthropic's alone; other providers
    // reject the block, so the chat goes without it rather than dying on it.
    let is_anthropic = api_base.contains("anthropic.com");
    let mut body = json!({
        "model": model,
        "max_tokens": MAX_TOKENS,
        "system": SYSTEM_PROMPT,
        "messages": chat.snapshot(),
    });
    if is_anthropic {
        body["tools"] = json!([{ "type": "web_search_20260209", "name": "web_search", "max_uses": 5 }]);
        body["fallbacks"] = json!("default");
    }

    let response = match call(api_base, &key, &body).await {
        Ok(v) => v,
        Err(err) => {
            chat.pop(); // keep the history consistent with what the model saw
            return Err(err);
        }
    };

    // A policy decline comes back as HTTP 200 with stop_reason "refusal".
    if response.get("stop_reason").and_then(Value::as_str) == Some("refusal") {
        chat.pop();
        let why = response
            .get("stop_details")
            .and_then(|d| d.get("explanation"))
            .and_then(Value::as_str)
            .unwrap_or("Claude declined this one.");
        return Err(why.to_string());
    }

    let Some(blocks) = response.get("content").and_then(Value::as_array).cloned() else {
        chat.pop();
        return Err("Unexpected API response.".into());
    };

    // Store the whole content — tool_use / tool_result blocks included — so the
    // next turn has the right context.
    chat.push(json!({ "role": "assistant", "content": blocks.clone() }));

    let text = blocks
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|b| b.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();

    if text.is_empty() {
        return Err("No response text.".into());
    }
    Ok(ChatReply { text })
}

/// One turn through a CLI agent. The rolling transcript rides along as plain
/// text — the CLI is one-shot, so history is prompt, not protocol.
async fn cli_turn(chat: &Chat, agent: &str, query: &str) -> Result<ChatReply, String> {
    let mut prompt = String::new();
    {
        let turns = chat.turns.lock().unwrap();
        let kept = turns.iter().rev().take(6).collect::<Vec<_>>().into_iter().rev();
        if turns.len() > 6 || turns.is_empty() {
            // nothing to note
        }
        let _ = kept.clone().count();
        if !turns.is_empty() {
            prompt.push_str("Conversation so far:\n");
            for (q, a) in kept {
                prompt.push_str(&format!("User: {q}\n{agent}: {a}\n"));
            }
            prompt.push('\n');
        }
    }
    prompt.push_str(&format!("User: {query}\n\nReply concisely, plain text, no markdown."));

    let out_path = std::env::temp_dir().join(format!("atlas-chat-{agent}-{}", std::process::id()));
    let use_file = agent == "codex";

    let mut cmd = std::process::Command::new(agent);
    cmd.stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if use_file {
        // -C - keeps it out of any project's git rules; the file takes the
        // final message so the transcript never parses progress noise.
        cmd.args(["exec", "--skip-git-repo-check", "--sandbox", "read-only", "-C", "-", "-o"])
            .arg(&out_path)
            .arg(&prompt)
            .stdout(std::process::Stdio::null());
    } else {
        cmd.arg("-p").arg(&prompt).stdout(std::process::Stdio::piped());
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Could not run {agent}: {e}. Is it installed and on PATH?"))?;

    // A live agent takes a while; wait off the runtime thread, reading any
    // stdout to its end first (claude -p writes the reply there).
    let result = tokio::task::spawn_blocking(move || -> std::io::Result<(std::process::ExitStatus, String)> {
        let mut text = String::new();
        if let Some(mut out) = child.stdout.take() {
            use std::io::Read;
            let _ = out.read_to_string(&mut text);
        }
        let status = child.wait()?;
        Ok((status, text))
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("{agent} failed: {e}"))?;

    let (status, stdout_text) = result;
    if !status.success() {
        return Err(format!("{agent} exited unsuccessfully ({}). Is it logged in?", status));
    }
    let text = if use_file {
        std::fs::read_to_string(&out_path).unwrap_or_default()
    } else {
        stdout_text
    };
    let _ = std::fs::remove_file(&out_path);
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err(format!("{agent} returned nothing."));
    }
    chat.turns.lock().unwrap().push((query.to_string(), text.clone()));
    Ok(ChatReply { text })
}

async fn call(api_base: &str, key: &str, body: &Value) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(90))
        .build()
        .map_err(|e| e.to_string())?;

    let mut request = client
        .post(format!("{api_base}/v1/messages"))
        // Both auth spellings, the same key: x-api-key is the Anthropic header,
        // Bearer is what Z.AI's compatible endpoint reads.
        .header("x-api-key", key)
        .header("authorization", format!("Bearer {key}"))
        .header("anthropic-version", ANTHROPIC_VERSION)
        .header("content-type", "application/json");
    if api_base.contains("anthropic.com") {
        request = request.header("anthropic-beta", FALLBACK_BETA);
    }

    let response = request
        .json(body)
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    let status = response.status();
    let text = response.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        // Surface the API's own message, which is what makes a bad key obvious.
        let detail = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| {
                v.get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| text.chars().take(200).collect());
        return Err(format!("Chat API {status}: {detail}"));
    }
    serde_json::from_str(&text).map_err(|e| format!("Bad API response: {e}"))
}

/// PDF → document block, image → image block, text/code → inline text.
/// Mirrors readFileAsBlock() in ClaudeService.swift.
fn file_block(path: &str) -> Option<Value> {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let media_type = match ext.as_str() {
        "pdf" => Some(("document", "application/pdf")),
        "jpg" | "jpeg" => Some(("image", "image/jpeg")),
        "png" => Some(("image", "image/png")),
        "gif" => Some(("image", "image/gif")),
        "webp" => Some(("image", "image/webp")),
        _ => None,
    };

    if let Some((block_type, media)) = media_type {
        let bytes = std::fs::read(path).ok()?;
        return Some(json!({
            "type": block_type,
            "source": { "type": "base64", "media_type": media, "data": base64(&bytes) },
        }));
    }

    let len = std::fs::metadata(path).ok()?.len();
    if len > MAX_INLINE_TEXT {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    Some(json!({ "type": "text", "text": format!("File contents:\n{text}") }))
}

/// Small standalone base64 encoder — not worth another dependency.
/// Also used for Stripe's basic auth.
pub(crate) fn base64_for(bytes: &[u8]) -> String {
    base64(bytes)
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::base64;

    #[test]
    fn base64_matches_rfc4648_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
