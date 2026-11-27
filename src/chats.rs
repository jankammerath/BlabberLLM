use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use gtk::glib;

use crate::ui::message::MessageRole;

const MAX_TEXT_ATTACHMENT_BYTES: usize = 256 * 1024;
const MAX_IMAGE_ATTACHMENT_BYTES: usize = 10 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatAttachment {
    pub name: String,
    pub path: String,
    pub mime_type: String,
    pub is_image: bool,
    /// UTF-8 content for text files, or base64-encoded bytes for images.
    pub data: String,
}

impl ChatAttachment {
    pub fn from_path(path: &Path) -> Result<Self, String> {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("attachment")
            .to_string();
        let path_str = path.to_string_lossy().to_string();
        let bytes = fs::read(path).map_err(|err| format!("Failed to read {name}: {err}"))?;

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default();

        let image_mime = match ext.as_str() {
            "png" => Some("image/png"),
            "jpg" | "jpeg" => Some("image/jpeg"),
            "webp" => Some("image/webp"),
            "gif" => Some("image/gif"),
            _ => None,
        };

        if let Some(mime) = image_mime {
            if bytes.len() > MAX_IMAGE_ATTACHMENT_BYTES {
                return Err(format!(
                    "{name} is too large (max {} MB for images).",
                    MAX_IMAGE_ATTACHMENT_BYTES / (1024 * 1024)
                ));
            }
            return Ok(Self {
                name,
                path: path_str,
                mime_type: mime.to_string(),
                is_image: true,
                data: encode_base64(&bytes),
            });
        }

        let text = String::from_utf8(bytes)
            .map_err(|_| format!("{name} is not a supported image or UTF-8 text file."))?;
        let data = if text.len() > MAX_TEXT_ATTACHMENT_BYTES {
            let mut end = MAX_TEXT_ATTACHMENT_BYTES;
            while end > 0 && !text.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}\n… [file truncated]", &text[..end])
        } else {
            text
        };

        Ok(Self {
            name,
            path: path_str,
            mime_type: "text/plain".to_string(),
            is_image: false,
            data,
        })
    }
}

fn encode_base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((triple >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

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
    pub attachments: Vec<ChatAttachment>,
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
    pub tool_use_enabled: Option<bool>,
    pub messages: Vec<ChatMessage>,
}

impl ChatSession {
    pub fn new(
        first_prompt: &str,
        last_account_id: Option<String>,
        last_model: Option<String>,
        tool_use_enabled: Option<bool>,
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
            tool_use_enabled,
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
        tool_use_enabled: Option<bool>,
    ) -> String {
        let chat = ChatSession::new(first_prompt, account_id, model, tool_use_enabled);
        let id = chat.id.clone();
        self.chats.insert(0, chat);
        let _ = self.save_chat(&id);
        id
    }

    pub fn set_chat_tool_use(&mut self, chat_id: &str, enabled: bool) {
        let Some(chat) = self.chats.iter_mut().find(|c| c.id == chat_id) else {
            return;
        };
        if chat.tool_use_enabled == Some(enabled) {
            return;
        }
        chat.tool_use_enabled = Some(enabled);
        let _ = self.save_chat(chat_id);
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

    #[cfg(test)]
    pub fn append_message(&mut self, chat_id: &str, role: MessageRole, content: String) {
        self.append_message_full(chat_id, role, content, Vec::new(), Vec::new(), None);
    }

    pub fn append_message_with_attachments(
        &mut self,
        chat_id: &str,
        role: MessageRole,
        content: String,
        attachments: Vec<ChatAttachment>,
    ) {
        self.append_message_full(chat_id, role, content, attachments, Vec::new(), None);
    }

    pub fn append_message_with_tools(
        &mut self,
        chat_id: &str,
        role: MessageRole,
        content: String,
        tool_calls: Vec<ToolCallRecord>,
        duration_ms: Option<u64>,
    ) {
        self.append_message_full(chat_id, role, content, Vec::new(), tool_calls, duration_ms);
    }

    fn append_message_full(
        &mut self,
        chat_id: &str,
        role: MessageRole,
        content: String,
        attachments: Vec<ChatAttachment>,
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
            attachments,
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
        if let Some(enabled) = chat.tool_use_enabled {
            key_file.set_boolean("chat", "tool_use_enabled", enabled);
        }
        key_file.set_uint64("chat", "message_count", chat.messages.len() as u64);

        for (idx, msg) in chat.messages.iter().enumerate() {
            let group = format!("message.{idx}");
            key_file.set_string(&group, "role", msg.role.as_str());
            key_file.set_string(&group, "content", &msg.content);
            if let Some(ms) = msg.duration_ms {
                key_file.set_uint64(&group, "duration_ms", ms);
            }
            if !msg.attachments.is_empty() {
                key_file.set_uint64(&group, "attachment_count", msg.attachments.len() as u64);
                for (a_idx, att) in msg.attachments.iter().enumerate() {
                    let att_group = format!("message.{idx}.attachment.{a_idx}");
                    key_file.set_string(&att_group, "name", &att.name);
                    key_file.set_string(&att_group, "path", &att.path);
                    key_file.set_string(&att_group, "mime_type", &att.mime_type);
                    key_file.set_boolean(&att_group, "is_image", att.is_image);
                    key_file.set_string(&att_group, "data", &att.data);
                }
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
    let tool_use_enabled = key_file.boolean("chat", "tool_use_enabled").ok();
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

        let attachment_count = key_file.uint64(&group, "attachment_count").unwrap_or(0) as usize;
        let mut attachments = Vec::with_capacity(attachment_count);
        for a_idx in 0..attachment_count {
            let att_group = format!("message.{idx}.attachment.{a_idx}");
            if !key_file.has_group(&att_group) {
                continue;
            }
            let name = key_file
                .string(&att_group, "name")
                .map(|s| s.to_string())
                .unwrap_or_default();
            let path = key_file
                .string(&att_group, "path")
                .map(|s| s.to_string())
                .unwrap_or_default();
            let mime_type = key_file
                .string(&att_group, "mime_type")
                .map(|s| s.to_string())
                .unwrap_or_else(|_| "text/plain".to_string());
            let is_image = key_file.boolean(&att_group, "is_image").unwrap_or(false);
            let data = key_file
                .string(&att_group, "data")
                .map(|s| s.to_string())
                .unwrap_or_default();
            attachments.push(ChatAttachment {
                name,
                path,
                mime_type,
                is_image,
                data,
            });
        }

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
            attachments,
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
        tool_use_enabled,
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
            Some(true),
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
        store.set_chat_tool_use(&id, false);

        let reloaded = ChatStore::load_from_dir(dir.clone());
        assert_eq!(reloaded.chats().len(), 1);
        let chat = reloaded.get(&id).expect("chat persisted");
        assert_eq!(chat.title, "Hello");
        assert_eq!(chat.last_account_id.as_deref(), Some("provider-2"));
        assert_eq!(chat.last_model.as_deref(), Some("gpt-4o"));
        assert_eq!(chat.tool_use_enabled, Some(false));
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

    #[test]
    fn encodes_base64_correctly() {
        assert_eq!(encode_base64(b""), "");
        assert_eq!(encode_base64(b"f"), "Zg==");
        assert_eq!(encode_base64(b"fo"), "Zm8=");
        assert_eq!(encode_base64(b"foo"), "Zm9v");
        assert_eq!(encode_base64(b"Hello!"), "SGVsbG8h");
    }
}
