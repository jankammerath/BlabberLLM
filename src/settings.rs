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

#[derive(Clone, Debug, Default)]
pub struct AppSettings {
    accounts: Vec<ProviderAccount>,
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
        true
    }

    pub fn configured_provider_count(&self) -> usize {
        self.accounts.iter().filter(|a| a.is_configured()).count()
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

        settings.save_to_path(&path).expect("save settings");
        let loaded = AppSettings::load_from_path(&path);

        assert_eq!(loaded.accounts().len(), 3);
        assert_eq!(loaded.configured_provider_count(), 3);
        assert_eq!(loaded.get(&work_openai.id), Some(&work_openai));
        assert_eq!(loaded.get(&personal_openai.id), Some(&personal_openai));
        assert_eq!(loaded.get(&ollama.id), Some(&ollama));

        let _ = fs::remove_dir_all(&dir);
    }
}
