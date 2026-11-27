use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::{json, Value};

use crate::chats::ChatMessage;
use crate::settings::{ProviderAccount, ProviderKind};
use crate::ui::message::MessageRole;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelOption {
    pub account_id: String,
    pub account_name: String,
    pub kind: ProviderKind,
    pub model: String,
}

impl ModelOption {
    pub fn display_label(&self) -> String {
        format!("{} · {}", self.model, self.account_name)
    }
}

pub fn fallback_models(kind: ProviderKind) -> &'static [&'static str] {
    match kind {
        ProviderKind::Claude => &[
            "claude-sonnet-4-5",
            "claude-opus-4-1",
            "claude-3-7-sonnet-latest",
            "claude-3-5-haiku-latest",
        ],
        ProviderKind::Gemini => &[
            "gemini-2.5-flash",
            "gemini-2.5-pro",
            "gemini-2.0-flash",
        ],
        ProviderKind::Grok => &["grok-3", "grok-3-mini", "grok-2-latest"],
        ProviderKind::OpenAi => &["gpt-4o", "gpt-4o-mini", "o3-mini"],
        ProviderKind::OpenRouter => &[
            "anthropic/claude-sonnet-4",
            "openai/gpt-4o",
            "google/gemini-2.5-flash",
        ],
        ProviderKind::Ollama => &[],
    }
}

fn build_agent(timeout_secs: u64) -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(timeout_secs)))
        .http_status_as_error(false)
        .build();
    config.into()
}

pub fn fetch_models(account: &ProviderAccount) -> Result<Vec<String>, String> {
    let base = account.endpoint.trim().trim_end_matches('/');
    if base.is_empty() {
        return Err("Endpoint URL is empty".to_string());
    }

    let agent = build_agent(12);
    let mut response = match account.kind {
        ProviderKind::OpenAi | ProviderKind::Grok | ProviderKind::OpenRouter => {
            let url = format!("{base}/models");
            let mut req = agent.get(&url);
            if !account.api_key.trim().is_empty() {
                req = req.header("Authorization", &format!("Bearer {}", account.api_key.trim()));
            }
            req.call()
        }
        ProviderKind::Claude => {
            let url = format!("{base}/models");
            agent
                .get(&url)
                .header("x-api-key", account.api_key.trim())
                .header("anthropic-version", "2023-06-01")
                .call()
        }
        ProviderKind::Gemini => {
            let url = format!("{base}/models");
            agent
                .get(&url)
                .header("x-goog-api-key", account.api_key.trim())
                .call()
        }
        ProviderKind::Ollama => {
            let url = format!("{base}/api/tags");
            let mut req = agent.get(&url);
            if !account.api_key.trim().is_empty() {
                req = req.header("Authorization", &format!("Bearer {}", account.api_key.trim()));
            }
            req.call()
        }
    }
    .map_err(|err| format!("Network error: {err}"))?;

    let status = response.status().as_u16();
    let body_text = response
        .body_mut()
        .read_to_string()
        .map_err(|err| format!("Failed to read response: {err}"))?;

    if !(200..300).contains(&status) {
        return Err(extract_error_message(status, &body_text));
    }

    let value: Value =
        serde_json::from_str(&body_text).map_err(|err| format!("Invalid JSON response: {err}"))?;

    let mut models = match account.kind {
        ProviderKind::OpenAi | ProviderKind::Grok | ProviderKind::OpenRouter | ProviderKind::Claude => {
            value
                .get("data")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|item| item.get("id").and_then(Value::as_str))
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        }
        ProviderKind::Gemini => value
            .get("models")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter(|item| {
                        item.get("supportedGenerationMethods")
                            .and_then(Value::as_array)
                            .map(|methods| {
                                methods
                                    .iter()
                                    .any(|m| m.as_str() == Some("generateContent"))
                            })
                            .unwrap_or(true)
                    })
                    .filter_map(|item| item.get("name").and_then(Value::as_str))
                    .map(|name| name.strip_prefix("models/").unwrap_or(name).to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
        ProviderKind::Ollama => value
            .get("models")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| item.get("name").and_then(Value::as_str))
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
    };

    models.sort();
    models.dedup();
    Ok(models)
}

pub fn stream_chat<F>(
    account: &ProviderAccount,
    model: &str,
    messages: &[ChatMessage],
    cancel: &AtomicBool,
    mut on_delta: F,
) -> Result<String, String>
where
    F: FnMut(&str),
{
    let base = account.endpoint.trim().trim_end_matches('/');
    if base.is_empty() {
        return Err("Provider endpoint URL is empty.".to_string());
    }
    if !account.kind.api_key_optional() && account.api_key.trim().is_empty() {
        return Err(format!(
            "API key is missing for {}. Configure it in Settings.",
            account.name
        ));
    }

    let agent = build_agent(300);
    let mut response = match account.kind {
        ProviderKind::OpenAi | ProviderKind::Grok | ProviderKind::OpenRouter => {
            let url = format!("{base}/chat/completions");
            let payload = json!({
                "model": model,
                "stream": true,
                "messages": messages.iter().map(|m| json!({
                    "role": m.role.as_str(),
                    "content": m.content,
                })).collect::<Vec<_>>(),
            });
            let mut req = agent
                .post(&url)
                .header("Authorization", &format!("Bearer {}", account.api_key.trim()))
                .header("Content-Type", "application/json");
            if account.kind == ProviderKind::OpenRouter {
                req = req
                    .header("HTTP-Referer", "https://blabberllm.org")
                    .header("X-Title", "BlabberLLM");
            }
            req.send(payload.to_string())
        }
        ProviderKind::Claude => {
            let url = format!("{base}/messages");
            let payload = json!({
                "model": model,
                "max_tokens": 4096,
                "stream": true,
                "messages": messages.iter().map(|m| json!({
                    "role": m.role.as_str(),
                    "content": m.content,
                })).collect::<Vec<_>>(),
            });
            agent
                .post(&url)
                .header("x-api-key", account.api_key.trim())
                .header("anthropic-version", "2023-06-01")
                .header("Content-Type", "application/json")
                .send(payload.to_string())
        }
        ProviderKind::Gemini => {
            let url = format!("{base}/models/{model}:streamGenerateContent?alt=sse");
            let payload = json!({
                "contents": messages.iter().map(|m| json!({
                    "role": match m.role {
                        MessageRole::User => "user",
                        MessageRole::Assistant => "model",
                    },
                    "parts": [{ "text": m.content }],
                })).collect::<Vec<_>>(),
            });
            agent
                .post(&url)
                .header("x-goog-api-key", account.api_key.trim())
                .header("Content-Type", "application/json")
                .send(payload.to_string())
        }
        ProviderKind::Ollama => {
            let url = format!("{base}/api/chat");
            let payload = json!({
                "model": model,
                "stream": true,
                "messages": messages.iter().map(|m| json!({
                    "role": m.role.as_str(),
                    "content": m.content,
                })).collect::<Vec<_>>(),
            });
            let mut req = agent
                .post(&url)
                .header("Content-Type", "application/json");
            if !account.api_key.trim().is_empty() {
                req = req.header("Authorization", &format!("Bearer {}", account.api_key.trim()));
            }
            req.send(payload.to_string())
        }
    }
    .map_err(|err| format!("Request failed: {err}"))?;

    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        let body = response
            .body_mut()
            .read_to_string()
            .unwrap_or_else(|_| String::new());
        return Err(extract_error_message(status, &body));
    }

    let reader = BufReader::new(response.into_body().into_reader());
    let mut full_text = String::new();

    for line_result in reader.lines() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let line = match line_result {
            Ok(line) => line,
            Err(err) => {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                return Err(format!("Stream interrupted: {err}"));
            }
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        match parse_stream_line(account.kind, trimmed) {
            StreamEvent::Delta(text) => {
                if !text.is_empty() {
                    full_text.push_str(&text);
                    on_delta(&text);
                }
            }
            StreamEvent::Done => break,
            StreamEvent::Error(err) => return Err(err),
            StreamEvent::Ignore => {}
        }
    }

    Ok(full_text)
}

#[derive(Debug, PartialEq, Eq)]
enum StreamEvent {
    Delta(String),
    Done,
    Error(String),
    Ignore,
}

fn parse_stream_line(kind: ProviderKind, line: &str) -> StreamEvent {
    if kind == ProviderKind::Ollama {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return StreamEvent::Ignore;
        };
        if let Some(err) = value.get("error").and_then(Value::as_str) {
            return StreamEvent::Error(err.to_string());
        }
        if value.get("done").and_then(Value::as_bool) == Some(true) {
            if let Some(text) = value
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(Value::as_str)
            {
                if !text.is_empty() {
                    return StreamEvent::Delta(text.to_string());
                }
            }
            return StreamEvent::Done;
        }
        if let Some(text) = value
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(Value::as_str)
        {
            return StreamEvent::Delta(text.to_string());
        }
        return StreamEvent::Ignore;
    }

    let Some(data) = line.strip_prefix("data:") else {
        return StreamEvent::Ignore;
    };
    let data = data.trim();
    if data.is_empty() {
        return StreamEvent::Ignore;
    }
    if data == "[DONE]" {
        return StreamEvent::Done;
    }

    let Ok(value) = serde_json::from_str::<Value>(data) else {
        return StreamEvent::Ignore;
    };

    if let Some(err_msg) = value
        .get("error")
        .and_then(|e| e.get("message").and_then(Value::as_str).or_else(|| e.as_str()))
    {
        return StreamEvent::Error(err_msg.to_string());
    }

    match kind {
        ProviderKind::OpenAi | ProviderKind::Grok | ProviderKind::OpenRouter => {
            if let Some(text) = value
                .get("choices")
                .and_then(Value::as_array)
                .and_then(|c| c.first())
                .and_then(|c| c.get("delta"))
                .and_then(|d| d.get("content"))
                .and_then(Value::as_str)
            {
                StreamEvent::Delta(text.to_string())
            } else {
                StreamEvent::Ignore
            }
        }
        ProviderKind::Claude => match value.get("type").and_then(Value::as_str) {
            Some("content_block_delta") => {
                if let Some(text) = value
                    .get("delta")
                    .and_then(|d| d.get("text"))
                    .and_then(Value::as_str)
                {
                    StreamEvent::Delta(text.to_string())
                } else {
                    StreamEvent::Ignore
                }
            }
            Some("message_stop") => StreamEvent::Done,
            _ => StreamEvent::Ignore,
        },
        ProviderKind::Gemini => {
            let text = value
                .get("candidates")
                .and_then(Value::as_array)
                .and_then(|c| c.first())
                .and_then(|c| c.get("content"))
                .and_then(|c| c.get("parts"))
                .and_then(Value::as_array)
                .map(|parts| {
                    parts
                        .iter()
                        .filter_map(|p| p.get("text").and_then(Value::as_str))
                        .collect::<String>()
                })
                .unwrap_or_default();
            if text.is_empty() {
                StreamEvent::Ignore
            } else {
                StreamEvent::Delta(text)
            }
        }
        ProviderKind::Ollama => unreachable!(),
    }
}

fn extract_error_message(status: u16, body: &str) -> String {
    if let Ok(value) = serde_json::from_str::<Value>(body) {
        if let Some(msg) = value
            .get("error")
            .and_then(|e| e.get("message").and_then(Value::as_str).or_else(|| e.as_str()))
            .or_else(|| value.get("message").and_then(Value::as_str))
        {
            return format!("HTTP {status}: {msg}");
        }
    }

    let snippet = body.trim();
    if snippet.is_empty() {
        format!("HTTP {status}")
    } else {
        let short: String = snippet.chars().take(240).collect();
        format!("HTTP {status}: {short}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_openai_sse_lines() {
        let line = r#"data: {"choices":[{"delta":{"content":"Hello"}}]}"#;
        assert_eq!(
            parse_stream_line(ProviderKind::OpenAi, line),
            StreamEvent::Delta("Hello".into())
        );
        assert_eq!(
            parse_stream_line(ProviderKind::OpenAi, "data: [DONE]"),
            StreamEvent::Done
        );
    }

    #[test]
    fn parses_claude_sse_lines() {
        let line = r#"data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"Hi!"}}"#;
        assert_eq!(
            parse_stream_line(ProviderKind::Claude, line),
            StreamEvent::Delta("Hi!".into())
        );
        let stop = r#"data: {"type":"message_stop"}"#;
        assert_eq!(
            parse_stream_line(ProviderKind::Claude, stop),
            StreamEvent::Done
        );
    }

    #[test]
    fn parses_gemini_sse_lines() {
        let line = r#"data: {"candidates":[{"content":{"parts":[{"text":"Gemini reply"}]}}]}"#;
        assert_eq!(
            parse_stream_line(ProviderKind::Gemini, line),
            StreamEvent::Delta("Gemini reply".into())
        );
    }

    #[test]
    fn parses_ollama_ndjson_lines() {
        let line = r#"{"message":{"role":"assistant","content":"Local token"},"done":false}"#;
        assert_eq!(
            parse_stream_line(ProviderKind::Ollama, line),
            StreamEvent::Delta("Local token".into())
        );
        let done = r#"{"message":{"role":"assistant","content":""},"done":true}"#;
        assert_eq!(
            parse_stream_line(ProviderKind::Ollama, done),
            StreamEvent::Done
        );
    }

    #[test]
    fn extracts_json_error_messages() {
        let body = r#"{"error":{"message":"Invalid API key"}}"#;
        assert_eq!(extract_error_message(401, body), "HTTP 401: Invalid API key");
    }
}
