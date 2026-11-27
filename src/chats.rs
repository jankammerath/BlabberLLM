use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use gtk::glib;

use crate::ui::message::MessageRole;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCallRecord {
    pub command: String,
    pub working_directory: Option<String>,
    pub exit_code: Option<i32>,
    pub output: String,
    pub denied: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
    pub tool_calls: Vec<ToolCallRecord>,
    pub duration_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatSession {
    pub id: String,
    pub title: String,
    pub updated_at: i64,
    pub last_account_id: Option<String>,
    pub last_model: Option<String>,
    pub messages: Vec<ChatMessage>,
}

impl ChatSession {
    pub fn new(
        first_prompt: &str,
        last_account_id: Option<String>,
        last_model: Option<String>,
    ) -> Self {
        let now = unix_timestamp_secs();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        Self {
            id: format!("chat-{nanos}"),
            title: title_from_prompt(first_prompt),
            updated_at: now,
            last_account_id,
            last_model,
            messages: Vec::new(),
        }
    }

    pub fn subtitle(&self) -> String {
        let count = self.messages.len();
        if count == 1 {
            "1 message".to_string()
        } else {
            format!("{count} messages")
        }
    }
}

pub struct ChatStore {
    dir: PathBuf,
    chats: Vec<ChatSession>,
}

impl ChatStore {
    pub fn default_dir() -> PathBuf {
        glib::user_data_dir().join("blabber-llm").join("chats")
    }

    pub fn load() -> Self {
        Self::load_from_dir(Self::default_dir())
    }

    pub fn load_from_dir(dir: PathBuf) -> Self {
        let mut chats = Vec::new();

        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("chat") {
                    continue;
                }
                if let Some(chat) = load_chat_file(&path) {
                    chats.push(chat);
                }
            }
        }

        chats.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then_with(|| b.id.cmp(&a.id)));

        Self { dir, chats }
    }

    pub fn chats(&self) -> &[ChatSession] {
        &self.chats
    }

    pub fn get(&self, id: &str) -> Option<&ChatSession> {
        self.chats.iter().find(|c| c.id == id)
    }

    pub fn create_chat(
        &mut self,
        first_prompt: &str,
        account_id: Option<String>,
        model: Option<String>,
    ) -> String {
        let chat = ChatSession::new(first_prompt, account_id, model);
        let id = chat.id.clone();
        self.chats.insert(0, chat);
        let _ = self.save_chat(&id);
        id
    }

    pub fn set_chat_model(&mut self, chat_id: &str, account_id: String, model: String) {
        let Some(chat) = self.chats.iter_mut().find(|c| c.id == chat_id) else {
            return;
        };
        if chat.last_account_id.as_deref() == Some(account_id.as_str())
            && chat.last_model.as_deref() == Some(model.as_str())
        {
            return;
        }
        chat.last_account_id = Some(account_id);
        chat.last_model = Some(model);
        let _ = self.save_chat(chat_id);
    }

    pub fn append_message(&mut self, chat_id: &str, role: MessageRole, content: String) {
        self.append_message_with_tools(chat_id, role, content, Vec::new(), None);
    }

    pub fn append_message_with_tools(
        &mut self,
        chat_id: &str,
        role: MessageRole,
        content: String,
        tool_calls: Vec<ToolCallRecord>,
        duration_ms: Option<u64>,
    ) {
        let Some(pos) = self.chats.iter().position(|c| c.id == chat_id) else {
            return;
        };

        let mut chat = self.chats.remove(pos);
        chat.messages.push(ChatMessage {
            role,
            content,
            tool_calls,
            duration_ms,
        });
        chat.updated_at = unix_timestamp_secs();
        self.chats.insert(0, chat);

        let _ = self.save_chat(chat_id);
    }

    pub fn delete_chat(&mut self, chat_id: &str) -> bool {
        let Some(pos) = self.chats.iter().position(|c| c.id == chat_id) else {
            return false;
        };
        self.chats.remove(pos);
        let path = self.chat_path(chat_id);
        let _ = fs::remove_file(path);
        true
    }

    fn chat_path(&self, chat_id: &str) -> PathBuf {
        self.dir.join(format!("{chat_id}.chat"))
    }

    fn save_chat(&self, chat_id: &str) -> Result<(), String> {
        let Some(chat) = self.get(chat_id) else {
            return Ok(());
        };
        fs::create_dir_all(&self.dir)
            .map_err(|err| format!("Failed to create chats directory: {err}"))?;

        let key_file = glib::KeyFile::new();
        key_file.set_string("chat", "id", &chat.id);
        key_file.set_string("chat", "title", &chat.title);
        key_file.set_int64("chat", "updated_at", chat.updated_at);
        if let Some(account_id) = &chat.last_account_id {
            key_file.set_string("chat", "last_account_id", account_id);
        }
        if let Some(model) = &chat.last_model {
            key_file.set_string("chat", "last_model", model);
        }
        key_file.set_uint64("chat", "message_count", chat.messages.len() as u64);

        for (idx, msg) in chat.messages.iter().enumerate() {
            let group = format!("message.{idx}");
            key_file.set_string(&group, "role", msg.role.as_str());
            key_file.set_string(&group, "content", &msg.content);
            if let Some(ms) = msg.duration_ms {
                key_file.set_uint64(&group, "duration_ms", ms);
            }
            if !msg.tool_calls.is_empty() {
                key_file.set_uint64(&group, "tool_count", msg.tool_calls.len() as u64);
                for (t_idx, tool) in msg.tool_calls.iter().enumerate() {
                    let tool_group = format!("message.{idx}.tool.{t_idx}");
                    key_file.set_string(&tool_group, "command", &tool.command);
                    if let Some(dir) = &tool.working_directory {
                        key_file.set_string(&tool_group, "working_directory", dir);
                    }
                    if let Some(code) = tool.exit_code {
                        key_file.set_integer(&tool_group, "exit_code", code);
                    }
                    key_file.set_string(&tool_group, "output", &tool.output);
                    key_file.set_boolean(&tool_group, "denied", tool.denied);
                }
            }
        }

        let data = key_file.to_data();
        fs::write(self.chat_path(chat_id), data.as_bytes())
            .map_err(|err| format!("Failed to write chat file: {err}"))
    }
}

fn load_chat_file(path: &Path) -> Option<ChatSession> {
    let key_file = glib::KeyFile::new();
    key_file
        .load_from_file(path, glib::KeyFileFlags::NONE)
        .ok()?;

    let id = key_file.string("chat", "id").ok()?.to_string();
    let title = key_file
        .string("chat", "title")
        .map(|s| s.to_string())
        .unwrap_or_else(|_| "Untitled Chat".to_string());
    let updated_at = key_file.int64("chat", "updated_at").unwrap_or(0);
    let last_account_id = key_file
        .string("chat", "last_account_id")
        .ok()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty());
    let last_model = key_file
        .string("chat", "last_model")
        .ok()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty());
    let message_count = key_file.uint64("chat", "message_count").unwrap_or(0) as usize;

    let mut messages = Vec::with_capacity(message_count);
    for idx in 0..message_count {
        let group = format!("message.{idx}");
        if !key_file.has_group(&group) {
            continue;
        }
        let role_str = key_file
            .string(&group, "role")
            .map(|s| s.to_string())
            .unwrap_or_default();
        let Some(role) = MessageRole::from_str(&role_str) else {
            continue;
        };
        let content = key_file
            .string(&group, "content")
            .map(|s| s.to_string())
            .unwrap_or_default();
        let duration_ms = key_file.uint64(&group, "duration_ms").ok();

        let tool_count = key_file.uint64(&group, "tool_count").unwrap_or(0) as usize;
        let mut tool_calls = Vec::with_capacity(tool_count);
        for t_idx in 0..tool_count {
            let tool_group = format!("message.{idx}.tool.{t_idx}");
            if !key_file.has_group(&tool_group) {
                continue;
            }
            let command = key_file
                .string(&tool_group, "command")
                .map(|s| s.to_string())
                .unwrap_or_default();
            let working_directory = key_file
                .string(&tool_group, "working_directory")
                .ok()
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty());
            let exit_code = key_file.integer(&tool_group, "exit_code").ok();
            let output = key_file
                .string(&tool_group, "output")
                .map(|s| s.to_string())
                .unwrap_or_default();
            let denied = key_file.boolean(&tool_group, "denied").unwrap_or(false);
            tool_calls.push(ToolCallRecord {
                command,
                working_directory,
                exit_code,
                output,
                denied,
            });
        }

        messages.push(ChatMessage {
            role,
            content,
            tool_calls,
            duration_ms,
        });
    }

    Some(ChatSession {
        id,
        title,
        updated_at,
        last_account_id,
        last_model,
        messages,
    })
}

fn title_from_prompt(prompt: &str) -> String {
    let first_line = prompt
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("New Chat");

    let mut chars = first_line.chars();
    let truncated: String = chars.by_ref().take(48).collect();
    if chars.next().is_some() {
        format!("{}…", truncated.trim_end())
    } else {
        truncated
    }
}

fn unix_timestamp_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_chat_store() {
        let dir = std::env::temp_dir().join(format!(
            "blabber-llm-chats-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);

        let mut store = ChatStore::load_from_dir(dir.clone());
        let id = store.create_chat(
            "Hello\nsecond line",
            Some("provider-1".into()),
            Some("claude-sonnet-4-5".into()),
        );
        store.append_message(&id, MessageRole::User, "Hello\nsecond line".into());
        store.append_message_with_tools(
            &id,
            MessageRole::Assistant,
            "Hi there!".into(),
            vec![ToolCallRecord {
                command: "ls -la".into(),
                working_directory: Some("/tmp".into()),
                exit_code: Some(0),
                output: "file1\nfile2".into(),
                denied: false,
            }],
            Some(4250),
        );
        store.set_chat_model(&id, "provider-2".into(), "gpt-4o".into());

        let reloaded = ChatStore::load_from_dir(dir.clone());
        assert_eq!(reloaded.chats().len(), 1);
        let chat = reloaded.get(&id).expect("chat persisted");
        assert_eq!(chat.title, "Hello");
        assert_eq!(chat.last_account_id.as_deref(), Some("provider-2"));
        assert_eq!(chat.last_model.as_deref(), Some("gpt-4o"));
        assert_eq!(chat.messages.len(), 2);
        assert_eq!(chat.messages[0].role, MessageRole::User);
        assert_eq!(chat.messages[0].content, "Hello\nsecond line");
        assert_eq!(chat.messages[1].role, MessageRole::Assistant);
        assert_eq!(chat.messages[1].content, "Hi there!");
        assert_eq!(chat.messages[1].duration_ms, Some(4250));
        assert_eq!(chat.messages[1].tool_calls.len(), 1);
        assert_eq!(chat.messages[1].tool_calls[0].command, "ls -la");
        assert_eq!(chat.messages[1].tool_calls[0].exit_code, Some(0));
        assert_eq!(chat.messages[1].tool_calls[0].output, "file1\nfile2");

        let _ = fs::remove_dir_all(&dir);
    }
}
