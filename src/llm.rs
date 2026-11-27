use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::{json, Value};

use crate::chats::ChatMessage;
use crate::settings::{ProviderAccount, ProviderKind, ShellPermissionMode};
use crate::tools::{self, is_dangerous_command};
use crate::ui::message::MessageRole;

const SHELL_TOOL_NAME: &str = "run_shell_command";
const MAX_TOOL_TURNS: usize = 10;

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

#[derive(Clone, Debug)]
pub struct ShellToolConfig {
    pub enabled: bool,
    pub permission_mode: ShellPermissionMode,
    pub sudo_enabled: bool,
}

#[derive(Clone, Debug)]
pub struct ShellToolRequest {
    pub command: String,
    pub working_directory: Option<String>,
    pub dangerous: bool,
    pub elevated: bool,
}

#[derive(Clone, Debug)]
pub enum ChatStreamEvent {
    Delta(String),
    ToolStarted {
        command: String,
        working_directory: Option<String>,
    },
    ToolFinished {
        command: String,
        exit_code: Option<i32>,
        output: String,
        denied: bool,
    },
}

pub fn fallback_models(kind: ProviderKind) -> &'static [&'static str] {
    match kind {
        ProviderKind::Claude => &[
            "claude-sonnet-4-5",
            "claude-opus-4-1",
            "claude-3-7-sonnet-latest",
            "claude-3-5-haiku-latest",
        ],
        ProviderKind::Gemini => &["gemini-2.5-flash", "gemini-2.5-pro", "gemini-2.0-flash"],
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
        ProviderKind::OpenAi
        | ProviderKind::Grok
        | ProviderKind::OpenRouter
        | ProviderKind::Claude => value
            .get("data")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| item.get("id").and_then(Value::as_str))
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
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

#[derive(Clone, Debug)]
struct PendingToolCall {
    id: String,
    name: String,
    arguments_json: String,
    arguments_value: Option<Value>,
}

impl PendingToolCall {
    fn parsed_args(&self) -> Value {
        if let Some(val) = &self.arguments_value {
            return val.clone();
        }
        serde_json::from_str::<Value>(&self.arguments_json).unwrap_or_else(|_| json!({}))
    }
}

pub fn stream_chat_with_tools<FEvent, FApprove>(
    account: &ProviderAccount,
    model: &str,
    messages: &[ChatMessage],
    tool_config: &ShellToolConfig,
    cancel: &AtomicBool,
    mut on_event: FEvent,
    mut request_approval: FApprove,
) -> Result<String, String>
where
    FEvent: FnMut(ChatStreamEvent),
    FApprove: FnMut(&ShellToolRequest) -> bool,
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
    let mut conv = build_initial_conversation(account.kind, messages);
    let mut accumulated_text = String::new();

    for turn in 0..MAX_TOOL_TURNS {
        if cancel.load(Ordering::Relaxed) {
            break;
        }

        let mut turn_text = String::new();
        let mut emitted_separator = false;

        let tool_calls = run_single_turn(
            &agent,
            account,
            base,
            model,
            &conv,
            tool_config,
            cancel,
            |delta| {
                if !emitted_separator && turn > 0 && !accumulated_text.is_empty() {
                    accumulated_text.push_str("\n\n");
                    on_event(ChatStreamEvent::Delta("\n\n".to_string()));
                    emitted_separator = true;
                }
                turn_text.push_str(delta);
                accumulated_text.push_str(delta);
                on_event(ChatStreamEvent::Delta(delta.to_string()));
            },
        )?;

        if tool_calls.is_empty() || !tool_config.enabled || cancel.load(Ordering::Relaxed) {
            break;
        }

        let mut tool_results: Vec<(PendingToolCall, String)> = Vec::new();
        for call in tool_calls {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let result_text = execute_tool_call(
                &call,
                tool_config,
                &mut on_event,
                &mut request_approval,
            );
            tool_results.push((call, result_text));
        }

        if cancel.load(Ordering::Relaxed) {
            break;
        }

        append_tool_turn(account.kind, &mut conv, &turn_text, &tool_results);
    }

    Ok(accumulated_text)
}

fn execute_tool_call<FEvent, FApprove>(
    call: &PendingToolCall,
    tool_config: &ShellToolConfig,
    on_event: &mut FEvent,
    request_approval: &mut FApprove,
) -> String
where
    FEvent: FnMut(ChatStreamEvent),
    FApprove: FnMut(&ShellToolRequest) -> bool,
{
    if call.name != SHELL_TOOL_NAME {
        return format!("Unknown tool: {}", call.name);
    }

    let args = call.parsed_args();
    let raw_command = args
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let working_directory = args
        .get("working_directory")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);
    let arg_sudo = args.get("sudo").and_then(Value::as_bool).unwrap_or(false);

    if raw_command.is_empty() {
        return "Error: missing required 'command' parameter.".to_string();
    }

    let stripped_sudo = tools::strip_leading_sudo(&raw_command).map(ToString::to_string);
    let wants_sudo = arg_sudo || stripped_sudo.is_some();
    let exec_command = stripped_sudo.unwrap_or_else(|| raw_command.clone());
    let display_command = if wants_sudo {
        format!("sudo {exec_command}")
    } else {
        exec_command.clone()
    };

    if wants_sudo && !tool_config.sudo_enabled {
        on_event(ChatStreamEvent::ToolFinished {
            command: display_command,
            exit_code: None,
            output: "Administrator (sudo) commands are disabled in Settings.".to_string(),
            denied: true,
        });
        return "Elevated (sudo) commands are disabled in BlabberLLM Settings. Ask the user to enable 'Allow Elevated Commands (sudo)' in Settings if administrator privileges are needed.".to_string();
    }

    let dangerous = wants_sudo || is_dangerous_command(&exec_command);
    let needs_approval = match tool_config.permission_mode {
        ShellPermissionMode::AlwaysAsk => true,
        ShellPermissionMode::AskWhenDangerous => dangerous,
        ShellPermissionMode::NeverAsk => false,
    };

    if needs_approval {
        let req = ShellToolRequest {
            command: display_command.clone(),
            working_directory: working_directory.clone(),
            dangerous,
            elevated: wants_sudo,
        };
        if !request_approval(&req) {
            on_event(ChatStreamEvent::ToolFinished {
                command: display_command,
                exit_code: None,
                output: "Permission denied by user.".to_string(),
                denied: true,
            });
            return "User denied permission to run this shell command.".to_string();
        }
    }

    on_event(ChatStreamEvent::ToolStarted {
        command: display_command.clone(),
        working_directory: working_directory.clone(),
    });

    let exec_result =
        tools::execute_shell_command(&exec_command, working_directory.as_deref(), wants_sudo);
    on_event(ChatStreamEvent::ToolFinished {
        command: display_command,
        exit_code: Some(exec_result.exit_code),
        output: exec_result.combined_preview(),
        denied: false,
    });

    exec_result.format_for_llm()
}

fn build_initial_conversation(kind: ProviderKind, messages: &[ChatMessage]) -> Vec<Value> {
    match kind {
        ProviderKind::Gemini => messages
            .iter()
            .map(|m| {
                json!({
                    "role": match m.role {
                        MessageRole::User => "user",
                        MessageRole::Assistant => "model",
                    },
                    "parts": [{ "text": m.content }],
                })
            })
            .collect(),
        _ => messages
            .iter()
            .map(|m| {
                json!({
                    "role": m.role.as_str(),
                    "content": m.content,
                })
            })
            .collect(),
    }
}

fn append_tool_turn(
    kind: ProviderKind,
    conv: &mut Vec<Value>,
    turn_text: &str,
    tool_results: &[(PendingToolCall, String)],
) {
    match kind {
        ProviderKind::OpenAi | ProviderKind::Grok | ProviderKind::OpenRouter => {
            let tool_calls_json: Vec<Value> = tool_results
                .iter()
                .map(|(call, _)| {
                    json!({
                        "id": call.id,
                        "type": "function",
                        "function": {
                            "name": call.name,
                            "arguments": if call.arguments_json.is_empty() {
                                call.parsed_args().to_string()
                            } else {
                                call.arguments_json.clone()
                            },
                        }
                    })
                })
                .collect();
            conv.push(json!({
                "role": "assistant",
                "content": if turn_text.is_empty() { Value::Null } else { Value::String(turn_text.to_string()) },
                "tool_calls": tool_calls_json,
            }));
            for (call, result) in tool_results {
                conv.push(json!({
                    "role": "tool",
                    "tool_call_id": call.id,
                    "content": result,
                }));
            }
        }
        ProviderKind::Claude => {
            let mut content_blocks = Vec::new();
            if !turn_text.is_empty() {
                content_blocks.push(json!({
                    "type": "text",
                    "text": turn_text,
                }));
            }
            for (call, _) in tool_results {
                content_blocks.push(json!({
                    "type": "tool_use",
                    "id": call.id,
                    "name": call.name,
                    "input": call.parsed_args(),
                }));
            }
            conv.push(json!({
                "role": "assistant",
                "content": content_blocks,
            }));

            let result_blocks: Vec<Value> = tool_results
                .iter()
                .map(|(call, result)| {
                    json!({
                        "type": "tool_result",
                        "tool_use_id": call.id,
                        "content": result,
                    })
                })
                .collect();
            conv.push(json!({
                "role": "user",
                "content": result_blocks,
            }));
        }
        ProviderKind::Gemini => {
            let mut model_parts = Vec::new();
            if !turn_text.is_empty() {
                model_parts.push(json!({ "text": turn_text }));
            }
            for (call, _) in tool_results {
                model_parts.push(json!({
                    "functionCall": {
                        "name": call.name,
                        "args": call.parsed_args(),
                    }
                }));
            }
            conv.push(json!({
                "role": "model",
                "parts": model_parts,
            }));

            let response_parts: Vec<Value> = tool_results
                .iter()
                .map(|(call, result)| {
                    json!({
                        "functionResponse": {
                            "name": call.name,
                            "response": { "output": result },
                        }
                    })
                })
                .collect();
            conv.push(json!({
                "role": "user",
                "parts": response_parts,
            }));
        }
        ProviderKind::Ollama => {
            let tool_calls_json: Vec<Value> = tool_results
                .iter()
                .map(|(call, _)| {
                    json!({
                        "function": {
                            "name": call.name,
                            "arguments": call.parsed_args(),
                        }
                    })
                })
                .collect();
            conv.push(json!({
                "role": "assistant",
                "content": turn_text,
                "tool_calls": tool_calls_json,
            }));
            for (_, result) in tool_results {
                conv.push(json!({
                    "role": "tool",
                    "content": result,
                }));
            }
        }
    }
}

fn openai_shell_tool_schema(sudo_enabled: bool) -> Value {
    let mut properties = json!({
        "command": {
            "type": "string",
            "description": "The bash command to execute."
        },
        "working_directory": {
            "type": "string",
            "description": "Optional working directory path."
        }
    });
    let description = if sudo_enabled {
        properties["sudo"] = json!({
            "type": "boolean",
            "description": "Set to true to execute with administrator (root) privileges via system authentication (pkexec)."
        });
        "Execute a shell command using bash on the user's Linux system and return exit code, stdout, and stderr. Set sudo=true when administrator privileges are required."
    } else {
        "Execute a shell command using bash on the user's Linux system and return exit code, stdout, and stderr."
    };

    json!({
        "type": "function",
        "function": {
            "name": SHELL_TOOL_NAME,
            "description": description,
            "parameters": {
                "type": "object",
                "properties": properties,
                "required": ["command"]
            }
        }
    })
}

fn claude_shell_tool_schema(sudo_enabled: bool) -> Value {
    let mut properties = json!({
        "command": {
            "type": "string",
            "description": "The bash command to execute."
        },
        "working_directory": {
            "type": "string",
            "description": "Optional working directory path."
        }
    });
    let description = if sudo_enabled {
        properties["sudo"] = json!({
            "type": "boolean",
            "description": "Set to true to execute with administrator (root) privileges via system authentication (pkexec)."
        });
        "Execute a shell command using bash on the user's Linux system and return exit code, stdout, and stderr. Set sudo=true when administrator privileges are required."
    } else {
        "Execute a shell command using bash on the user's Linux system and return exit code, stdout, and stderr."
    };

    json!({
        "name": SHELL_TOOL_NAME,
        "description": description,
        "input_schema": {
            "type": "object",
            "properties": properties,
            "required": ["command"]
        }
    })
}

fn gemini_shell_tool_schema(sudo_enabled: bool) -> Value {
    let mut properties = json!({
        "command": {
            "type": "STRING",
            "description": "The bash command to execute."
        },
        "working_directory": {
            "type": "STRING",
            "description": "Optional working directory path."
        }
    });
    let description = if sudo_enabled {
        properties["sudo"] = json!({
            "type": "BOOLEAN",
            "description": "Set to true to execute with administrator (root) privileges via system authentication (pkexec)."
        });
        "Execute a shell command using bash on the user's Linux system and return exit code, stdout, and stderr. Set sudo=true when administrator privileges are required."
    } else {
        "Execute a shell command using bash on the user's Linux system and return exit code, stdout, and stderr."
    };

    json!({
        "name": SHELL_TOOL_NAME,
        "description": description,
        "parameters": {
            "type": "OBJECT",
            "properties": properties,
            "required": ["command"]
        }
    })
}

#[allow(clippy::too_many_arguments)]
fn run_single_turn<F>(
    agent: &ureq::Agent,
    account: &ProviderAccount,
    base: &str,
    model: &str,
    conv: &[Value],
    tool_config: &ShellToolConfig,
    cancel: &AtomicBool,
    mut on_delta: F,
) -> Result<Vec<PendingToolCall>, String>
where
    F: FnMut(&str),
{
    let tools_enabled = tool_config.enabled;
    let sudo_enabled = tool_config.sudo_enabled;
    let mut response = match account.kind {
        ProviderKind::OpenAi | ProviderKind::Grok | ProviderKind::OpenRouter => {
            let url = format!("{base}/chat/completions");
            let mut payload = json!({
                "model": model,
                "stream": true,
                "messages": conv,
            });
            if tools_enabled {
                payload["tools"] = json!([openai_shell_tool_schema(sudo_enabled)]);
            }
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
            let mut payload = json!({
                "model": model,
                "max_tokens": 4096,
                "stream": true,
                "messages": conv,
            });
            if tools_enabled {
                payload["tools"] = json!([claude_shell_tool_schema(sudo_enabled)]);
            }
            agent
                .post(&url)
                .header("x-api-key", account.api_key.trim())
                .header("anthropic-version", "2023-06-01")
                .header("Content-Type", "application/json")
                .send(payload.to_string())
        }
        ProviderKind::Gemini => {
            let url = format!("{base}/models/{model}:streamGenerateContent?alt=sse");
            let mut payload = json!({
                "contents": conv,
            });
            if tools_enabled {
                payload["tools"] = json!([{
                    "functionDeclarations": [gemini_shell_tool_schema(sudo_enabled)]
                }]);
            }
            agent
                .post(&url)
                .header("x-goog-api-key", account.api_key.trim())
                .header("Content-Type", "application/json")
                .send(payload.to_string())
        }
        ProviderKind::Ollama => {
            let url = format!("{base}/api/chat");
            let mut payload = json!({
                "model": model,
                "stream": true,
                "messages": conv,
            });
            if tools_enabled {
                payload["tools"] = json!([openai_shell_tool_schema(sudo_enabled)]);
            }
            let mut req = agent.post(&url).header("Content-Type", "application/json");
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
    let mut indexed_calls: BTreeMap<usize, PendingToolCall> = BTreeMap::new();
    let mut direct_calls: Vec<PendingToolCall> = Vec::new();

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

        match parse_stream_line(account.kind, trimmed, &mut indexed_calls, &mut direct_calls) {
            StreamEvent::Delta(text) => {
                if !text.is_empty() {
                    on_delta(&text);
                }
            }
            StreamEvent::Done => break,
            StreamEvent::Error(err) => return Err(err),
            StreamEvent::Ignore => {}
        }
    }

    let mut all_calls: Vec<PendingToolCall> = indexed_calls.into_values().collect();
    all_calls.extend(direct_calls);
    Ok(all_calls)
}

#[derive(Debug, PartialEq, Eq)]
enum StreamEvent {
    Delta(String),
    Done,
    Error(String),
    Ignore,
}

fn parse_stream_line(
    kind: ProviderKind,
    line: &str,
    indexed_calls: &mut BTreeMap<usize, PendingToolCall>,
    direct_calls: &mut Vec<PendingToolCall>,
) -> StreamEvent {
    if kind == ProviderKind::Ollama {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return StreamEvent::Ignore;
        };
        if let Some(err) = value.get("error").and_then(Value::as_str) {
            return StreamEvent::Error(err.to_string());
        }
        if let Some(calls) = value
            .get("message")
            .and_then(|m| m.get("tool_calls"))
            .and_then(Value::as_array)
        {
            for (idx, call) in calls.iter().enumerate() {
                if let Some(func) = call.get("function") {
                    let name = func
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    if !name.is_empty() {
                        let args_val = func.get("arguments").cloned().unwrap_or_else(|| json!({}));
                        direct_calls.push(PendingToolCall {
                            id: format!("ollama-call-{idx}"),
                            name,
                            arguments_json: args_val.to_string(),
                            arguments_value: Some(args_val),
                        });
                    }
                }
            }
        }
        let text = value
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if value.get("done").and_then(Value::as_bool) == Some(true) {
            if !text.is_empty() {
                return StreamEvent::Delta(text.to_string());
            }
            return StreamEvent::Done;
        }
        if !text.is_empty() {
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
            if let Some(delta) = value
                .get("choices")
                .and_then(Value::as_array)
                .and_then(|c| c.first())
                .and_then(|c| c.get("delta"))
            {
                if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
                    for tc in tool_calls {
                        let idx = tc.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                        let entry = indexed_calls.entry(idx).or_insert_with(|| PendingToolCall {
                            id: format!("call-{idx}"),
                            name: String::new(),
                            arguments_json: String::new(),
                            arguments_value: None,
                        });
                        if let Some(id) = tc.get("id").and_then(Value::as_str) {
                            if !id.is_empty() {
                                entry.id = id.to_string();
                            }
                        }
                        if let Some(func) = tc.get("function") {
                            if let Some(name) = func.get("name").and_then(Value::as_str) {
                                entry.name.push_str(name);
                            }
                            if let Some(args) = func.get("arguments").and_then(Value::as_str) {
                                entry.arguments_json.push_str(args);
                            }
                        }
                    }
                }

                if let Some(text) = delta.get("content").and_then(Value::as_str) {
                    if !text.is_empty() {
                        return StreamEvent::Delta(text.to_string());
                    }
                }
            }
            StreamEvent::Ignore
        }
        ProviderKind::Claude => match value.get("type").and_then(Value::as_str) {
            Some("content_block_start") => {
                if let Some(block) = value.get("content_block") {
                    if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                        let idx = value.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                        let id = block
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let name = block
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        indexed_calls.insert(
                            idx,
                            PendingToolCall {
                                id,
                                name,
                                arguments_json: String::new(),
                                arguments_value: None,
                            },
                        );
                    }
                }
                StreamEvent::Ignore
            }
            Some("content_block_delta") => {
                let Some(delta) = value.get("delta") else {
                    return StreamEvent::Ignore;
                };
                match delta.get("type").and_then(Value::as_str) {
                    Some("text_delta") => delta
                        .get("text")
                        .and_then(Value::as_str)
                        .map(|t| StreamEvent::Delta(t.to_string()))
                        .unwrap_or(StreamEvent::Ignore),
                    Some("input_json_delta") => {
                        let idx = value.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                        if let Some(partial) = delta.get("partial_json").and_then(Value::as_str) {
                            if let Some(entry) = indexed_calls.get_mut(&idx) {
                                entry.arguments_json.push_str(partial);
                            }
                        }
                        StreamEvent::Ignore
                    }
                    _ => StreamEvent::Ignore,
                }
            }
            Some("message_stop") => StreamEvent::Done,
            _ => StreamEvent::Ignore,
        },
        ProviderKind::Gemini => {
            let mut text = String::new();
            if let Some(parts) = value
                .get("candidates")
                .and_then(Value::as_array)
                .and_then(|c| c.first())
                .and_then(|c| c.get("content"))
                .and_then(|c| c.get("parts"))
                .and_then(Value::as_array)
            {
                for part in parts {
                    if let Some(t) = part.get("text").and_then(Value::as_str) {
                        text.push_str(t);
                    }
                    if let Some(fc) = part.get("functionCall") {
                        let name = fc
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        if !name.is_empty() {
                            let args_val = fc.get("args").cloned().unwrap_or_else(|| json!({}));
                            direct_calls.push(PendingToolCall {
                                id: format!("gemini-call-{}", direct_calls.len()),
                                name,
                                arguments_json: args_val.to_string(),
                                arguments_value: Some(args_val),
                            });
                        }
                    }
                }
            }
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
    fn parses_openai_sse_lines_and_tool_calls() {
        let mut indexed = BTreeMap::new();
        let mut direct = Vec::new();

        let line = r#"data: {"choices":[{"delta":{"content":"Hello"}}]}"#;
        assert_eq!(
            parse_stream_line(ProviderKind::OpenAi, line, &mut indexed, &mut direct),
            StreamEvent::Delta("Hello".into())
        );

        let tc1 = r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"run_shell_command","arguments":"{\"command\":"}}]}}]}"#;
        let tc2 = r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"ls\"}"}}]}}]}"#;
        parse_stream_line(ProviderKind::OpenAi, tc1, &mut indexed, &mut direct);
        parse_stream_line(ProviderKind::OpenAi, tc2, &mut indexed, &mut direct);
        assert_eq!(indexed.len(), 1);
        assert_eq!(indexed[&0].name, "run_shell_command");
        assert_eq!(indexed[&0].parsed_args()["command"], "ls");

        assert_eq!(
            parse_stream_line(ProviderKind::OpenAi, "data: [DONE]", &mut indexed, &mut direct),
            StreamEvent::Done
        );
    }

    #[test]
    fn parses_claude_sse_lines_and_tool_use() {
        let mut indexed = BTreeMap::new();
        let mut direct = Vec::new();

        let line = r#"data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"Hi!"}}"#;
        assert_eq!(
            parse_stream_line(ProviderKind::Claude, line, &mut indexed, &mut direct),
            StreamEvent::Delta("Hi!".into())
        );

        let start = r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"run_shell_command"}}"#;
        let delta = r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"command\":\"pwd\"}"}}"#;
        parse_stream_line(ProviderKind::Claude, start, &mut indexed, &mut direct);
        parse_stream_line(ProviderKind::Claude, delta, &mut indexed, &mut direct);
        assert_eq!(indexed[&1].id, "toolu_1");
        assert_eq!(indexed[&1].parsed_args()["command"], "pwd");

        let stop = r#"data: {"type":"message_stop"}"#;
        assert_eq!(
            parse_stream_line(ProviderKind::Claude, stop, &mut indexed, &mut direct),
            StreamEvent::Done
        );
    }

    #[test]
    fn parses_gemini_sse_lines_and_function_call() {
        let mut indexed = BTreeMap::new();
        let mut direct = Vec::new();

        let line = r#"data: {"candidates":[{"content":{"parts":[{"text":"Gemini reply"}]}}]}"#;
        assert_eq!(
            parse_stream_line(ProviderKind::Gemini, line, &mut indexed, &mut direct),
            StreamEvent::Delta("Gemini reply".into())
        );

        let fc = r#"data: {"candidates":[{"content":{"parts":[{"functionCall":{"name":"run_shell_command","args":{"command":"date"}}}]}}]}"#;
        parse_stream_line(ProviderKind::Gemini, fc, &mut indexed, &mut direct);
        assert_eq!(direct.len(), 1);
        assert_eq!(direct[0].parsed_args()["command"], "date");
    }

    #[test]
    fn parses_ollama_ndjson_lines() {
        let mut indexed = BTreeMap::new();
        let mut direct = Vec::new();

        let line = r#"{"message":{"role":"assistant","content":"Local token"},"done":false}"#;
        assert_eq!(
            parse_stream_line(ProviderKind::Ollama, line, &mut indexed, &mut direct),
            StreamEvent::Delta("Local token".into())
        );
        let done = r#"{"message":{"role":"assistant","content":""},"done":true}"#;
        assert_eq!(
            parse_stream_line(ProviderKind::Ollama, done, &mut indexed, &mut direct),
            StreamEvent::Done
        );
    }

    #[test]
    fn extracts_json_error_messages() {
        let body = r#"{"error":{"message":"Invalid API key"}}"#;
        assert_eq!(extract_error_message(401, body), "HTTP 401: Invalid API key");
    }
}
