use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use gtk::glib;

static NEXT_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProviderKind {
    Claude,
    Gemini,
    Grok,
    OpenAi,
    OpenRouter,
    Ollama,
}

impl ProviderKind {
    pub const ALL: [ProviderKind; 6] = [
        ProviderKind::Claude,
        ProviderKind::Gemini,
        ProviderKind::Grok,
        ProviderKind::OpenAi,
        ProviderKind::OpenRouter,
        ProviderKind::Ollama,
    ];

    pub fn key(self) -> &'static str {
        match self {
            ProviderKind::Claude => "claude",
            ProviderKind::Gemini => "gemini",
            ProviderKind::Grok => "grok",
            ProviderKind::OpenAi => "openai",
            ProviderKind::OpenRouter => "openrouter",
            ProviderKind::Ollama => "ollama",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.key() == key)
    }

    pub fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|&kind| kind == self)
            .unwrap_or(0)
    }

    pub fn from_index(index: usize) -> Self {
        Self::ALL.get(index).copied().unwrap_or(ProviderKind::Claude)
    }

    pub fn display_name(self) -> &'static str {
        match self {
            ProviderKind::Claude => "Claude",
            ProviderKind::Gemini => "Gemini",
            ProviderKind::Grok => "Grok",
            ProviderKind::OpenAi => "OpenAI",
            ProviderKind::OpenRouter => "OpenRouter",
            ProviderKind::Ollama => "Ollama",
        }
    }

    pub fn default_endpoint(self) -> &'static str {
        match self {
            ProviderKind::Claude => "https://api.anthropic.com/v1",
            ProviderKind::Gemini => "https://generativelanguage.googleapis.com/v1beta",
            ProviderKind::Grok => "https://api.x.ai/v1",
            ProviderKind::OpenAi => "https://api.openai.com/v1",
            ProviderKind::OpenRouter => "https://openrouter.ai/api/v1",
            ProviderKind::Ollama => "http://localhost:11434",
        }
    }

    pub fn api_key_optional(self) -> bool {
        matches!(self, ProviderKind::Ollama)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderAccount {
    pub id: String,
    pub kind: ProviderKind,
    pub name: String,
    pub endpoint: String,
    pub api_key: String,
}

impl ProviderAccount {
    pub fn new(kind: ProviderKind, name: String, endpoint: String, api_key: String) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let seq = NEXT_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
        Self {
            id: format!("provider-{nanos}-{seq}"),
            kind,
            name,
            endpoint,
            api_key,
        }
    }

    pub fn is_configured(&self) -> bool {
        if self.kind.api_key_optional() {
            !self.endpoint.trim().is_empty()
        } else {
            !self.endpoint.trim().is_empty() && !self.api_key.trim().is_empty()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ShellPermissionMode {
    AlwaysAsk,
    #[default]
    AskWhenDangerous,
    NeverAsk,
}

impl ShellPermissionMode {
    pub const ALL: [ShellPermissionMode; 3] = [
        ShellPermissionMode::AlwaysAsk,
        ShellPermissionMode::AskWhenDangerous,
        ShellPermissionMode::NeverAsk,
    ];

    pub fn key(self) -> &'static str {
        match self {
            ShellPermissionMode::AlwaysAsk => "always_ask",
            ShellPermissionMode::AskWhenDangerous => "ask_when_dangerous",
            ShellPermissionMode::NeverAsk => "never_ask",
        }
    }

    pub fn from_key(key: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|m| m.key() == key)
            .unwrap_or(ShellPermissionMode::AskWhenDangerous)
    }

    pub fn display_name(self) -> &'static str {
        match self {
            ShellPermissionMode::AlwaysAsk => "Always ask",
            ShellPermissionMode::AskWhenDangerous => "Only ask when dangerous",
            ShellPermissionMode::NeverAsk => "Never ask",
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|&m| m == self).unwrap_or(1)
    }

    pub fn from_index(idx: usize) -> Self {
        Self::ALL
            .get(idx)
            .copied()
            .unwrap_or(ShellPermissionMode::AskWhenDangerous)
    }
}

pub const DEFAULT_MAX_TOOL_STEPS: usize = 10;
pub const MIN_MAX_TOOL_STEPS: usize = 1;
pub const MAX_MAX_TOOL_STEPS: usize = 100;

#[derive(Clone, Debug)]
pub struct AppSettings {
    accounts: Vec<ProviderAccount>,
    default_account_id: Option<String>,
    default_model: Option<String>,
    shell_tool_enabled: bool,
    shell_permission_mode: ShellPermissionMode,
    shell_sudo_enabled: bool,
    max_tool_steps: usize,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            accounts: Vec::new(),
            default_account_id: None,
            default_model: None,
            shell_tool_enabled: false,
            shell_permission_mode: ShellPermissionMode::default(),
            shell_sudo_enabled: false,
            max_tool_steps: DEFAULT_MAX_TOOL_STEPS,
        }
    }
}

impl AppSettings {
    pub fn config_path() -> PathBuf {
        glib::user_config_dir()
            .join("blabber-llm")
            .join("settings.conf")
    }

    pub fn load() -> Self {
        Self::load_from_path(&Self::config_path())
    }

    pub fn load_from_path(path: &PathBuf) -> Self {
        let mut settings = Self::default();
        let key_file = glib::KeyFile::new();

        if key_file
            .load_from_file(path, glib::KeyFileFlags::NONE)
            .is_err()
        {
            return settings;
        }

        if key_file.has_group("defaults") {
            settings.default_account_id = key_file
                .string("defaults", "account_id")
                .ok()
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty());
            settings.default_model = key_file
                .string("defaults", "model")
                .ok()
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty());
        } else if key_file.has_group("selection") {
            settings.default_account_id = key_file
                .string("selection", "account_id")
                .ok()
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty());
            settings.default_model = key_file
                .string("selection", "model")
                .ok()
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty());
        }

        if key_file.has_group("tools") {
            settings.shell_tool_enabled = key_file.boolean("tools", "shell_enabled").unwrap_or(false);
            if let Ok(mode) = key_file.string("tools", "shell_permission") {
                settings.shell_permission_mode = ShellPermissionMode::from_key(mode.as_str());
            }
            settings.shell_sudo_enabled = key_file.boolean("tools", "sudo_enabled").unwrap_or(false);
            if let Ok(steps) = key_file.integer("tools", "max_steps") {
                settings.set_max_tool_steps(steps.max(1) as usize);
            }
        }

        if key_file.has_group("providers") {
            if let Ok(raw_ids) = key_file.string("providers", "ids") {
                for id_str in raw_ids.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    let group = format!("provider.{id_str}");
                    if !key_file.has_group(&group) {
                        continue;
                    }
                    let kind_str = key_file
                        .string(&group, "kind")
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    let Some(kind) = ProviderKind::from_key(&kind_str) else {
                        continue;
                    };
                    let name = key_file
                        .string(&group, "name")
                        .map(|s| s.to_string())
                        .unwrap_or_else(|_| kind.display_name().to_string());
                    let endpoint = key_file
                        .string(&group, "endpoint")
                        .map(|s| s.to_string())
                        .unwrap_or_else(|_| kind.default_endpoint().to_string());
                    let api_key = key_file
                        .string(&group, "api_key")
                        .map(|s| s.to_string())
                        .unwrap_or_default();

                    settings.accounts.push(ProviderAccount {
                        id: id_str.to_string(),
                        kind,
                        name,
                        endpoint,
                        api_key,
                    });
                }
            }
            return settings;
        }

        // Migrate any legacy single-account-per-provider sections if present.
        for kind in ProviderKind::ALL {
            let group = kind.key();
            if !key_file.has_group(group) {
                continue;
            }
            let endpoint = key_file
                .string(group, "endpoint")
                .map(|s| s.to_string())
                .unwrap_or_else(|_| kind.default_endpoint().to_string());
            let api_key = key_file
                .string(group, "api_key")
                .map(|s| s.to_string())
                .unwrap_or_default();

            let customized_endpoint = endpoint.trim() != kind.default_endpoint();
            if !api_key.trim().is_empty() || customized_endpoint {
                settings.accounts.push(ProviderAccount::new(
                    kind,
                    kind.display_name().to_string(),
                    endpoint,
                    api_key,
                ));
            }
        }

        settings
    }

    pub fn save(&self) -> Result<(), String> {
        self.save_to_path(&Self::config_path())
    }

    pub fn save_to_path(&self, path: &PathBuf) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("Failed to create config directory: {err}"))?;
        }

        let key_file = glib::KeyFile::new();
        if let Some(account_id) = &self.default_account_id {
            key_file.set_string("defaults", "account_id", account_id);
        }
        if let Some(model) = &self.default_model {
            key_file.set_string("defaults", "model", model);
        }
        key_file.set_boolean("tools", "shell_enabled", self.shell_tool_enabled);
        key_file.set_string("tools", "shell_permission", self.shell_permission_mode.key());
        key_file.set_boolean("tools", "sudo_enabled", self.shell_sudo_enabled);
        key_file.set_integer("tools", "max_steps", self.max_tool_steps as i32);
        let ids: Vec<&str> = self.accounts.iter().map(|a| a.id.as_str()).collect();
        key_file.set_string("providers", "ids", &ids.join(","));

        for account in &self.accounts {
            let group = format!("provider.{}", account.id);
            key_file.set_string(&group, "kind", account.kind.key());
            key_file.set_string(&group, "name", &account.name);
            key_file.set_string(&group, "endpoint", &account.endpoint);
            key_file.set_string(&group, "api_key", &account.api_key);
        }

        let data = key_file.to_data();
        fs::write(path, data.as_bytes())
            .map_err(|err| format!("Failed to write settings file: {err}"))?;

        #[cfg(unix)]
        {
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
        }

        Ok(())
    }

    pub fn accounts(&self) -> &[ProviderAccount] {
        &self.accounts
    }

    pub fn get(&self, id: &str) -> Option<&ProviderAccount> {
        self.accounts.iter().find(|a| a.id == id)
    }

    pub fn suggested_name(&self, kind: ProviderKind) -> String {
        let base = kind.display_name();
        let count = self.accounts.iter().filter(|a| a.kind == kind).count();
        if count == 0 {
            base.to_string()
        } else {
            format!("{base} {}", count + 1)
        }
    }

    pub fn add_account(&mut self, account: ProviderAccount) {
        self.accounts.push(account);
    }

    pub fn update_account(&mut self, updated: ProviderAccount) {
        if let Some(existing) = self.accounts.iter_mut().find(|a| a.id == updated.id) {
            *existing = updated;
        }
    }

    pub fn remove_account(&mut self, id: &str) -> bool {
        let Some(pos) = self.accounts.iter().position(|a| a.id == id) else {
            return false;
        };
        self.accounts.remove(pos);
        if self.default_account_id.as_deref() == Some(id) {
            self.default_account_id = None;
            self.default_model = None;
        }
        true
    }

    pub fn configured_provider_count(&self) -> usize {
        self.accounts.iter().filter(|a| a.is_configured()).count()
    }

    pub fn default_model(&self) -> Option<(&str, &str)> {
        match (
            self.default_account_id.as_deref(),
            self.default_model.as_deref(),
        ) {
            (Some(account_id), Some(model)) => Some((account_id, model)),
            _ => None,
        }
    }

    pub fn set_default_model(&mut self, account_id: Option<String>, model: Option<String>) {
        self.default_account_id = account_id;
        self.default_model = model;
    }

    pub fn shell_tool_enabled(&self) -> bool {
        self.shell_tool_enabled
    }

    pub fn set_shell_tool_enabled(&mut self, enabled: bool) {
        self.shell_tool_enabled = enabled;
    }

    pub fn shell_permission_mode(&self) -> ShellPermissionMode {
        self.shell_permission_mode
    }

    pub fn set_shell_permission_mode(&mut self, mode: ShellPermissionMode) {
        self.shell_permission_mode = mode;
    }

    pub fn shell_sudo_enabled(&self) -> bool {
        self.shell_sudo_enabled
    }

    pub fn set_shell_sudo_enabled(&mut self, enabled: bool) {
        self.shell_sudo_enabled = enabled;
    }

    pub fn max_tool_steps(&self) -> usize {
        self.max_tool_steps
    }

    pub fn set_max_tool_steps(&mut self, steps: usize) {
        self.max_tool_steps = steps.clamp(MIN_MAX_TOOL_STEPS, MAX_MAX_TOOL_STEPS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_multiple_accounts_per_provider() {
        let dir = std::env::temp_dir().join(format!(
            "blabber-llm-settings-test-{}",
            std::process::id()
        ));
        let path = dir.join("settings.conf");

        let mut settings = AppSettings::default();
        let work_openai = ProviderAccount::new(
            ProviderKind::OpenAi,
            "OpenAI (Work)".into(),
            "https://work.openai.example/v1".into(),
            "sk-work-123".into(),
        );
        let personal_openai = ProviderAccount::new(
            ProviderKind::OpenAi,
            "OpenAI (Personal)".into(),
            ProviderKind::OpenAi.default_endpoint().into(),
            "sk-personal-456".into(),
        );
        let ollama = ProviderAccount::new(
            ProviderKind::Ollama,
            "Local Ollama".into(),
            "http://127.0.0.1:11434".into(),
            String::new(),
        );

        settings.add_account(work_openai.clone());
        settings.add_account(personal_openai.clone());
        settings.add_account(ollama.clone());
        settings.set_default_model(Some(work_openai.id.clone()), Some("gpt-4o".into()));

        settings.save_to_path(&path).expect("save settings");
        let loaded = AppSettings::load_from_path(&path);

        assert_eq!(loaded.accounts().len(), 3);
        assert_eq!(loaded.configured_provider_count(), 3);
        assert_eq!(loaded.get(&work_openai.id), Some(&work_openai));
        assert_eq!(loaded.get(&personal_openai.id), Some(&personal_openai));
        assert_eq!(loaded.get(&ollama.id), Some(&ollama));
        assert_eq!(
            loaded.default_model(),
            Some((work_openai.id.as_str(), "gpt-4o"))
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
