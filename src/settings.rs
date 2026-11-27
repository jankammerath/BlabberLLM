use std::collections::BTreeMap;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use gtk::glib;

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

    pub fn description(self) -> &'static str {
        match self {
            ProviderKind::Claude => "Anthropic Messages API",
            ProviderKind::Gemini => "Google Gemini API",
            ProviderKind::Grok => "xAI Grok API",
            ProviderKind::OpenAi => "OpenAI API",
            ProviderKind::OpenRouter => "OpenRouter unified model gateway",
            ProviderKind::Ollama => "Local or remote Ollama server (API key optional)",
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
pub struct ProviderConfig {
    pub endpoint: String,
    pub api_key: String,
}

impl ProviderConfig {
    pub fn default_for(kind: ProviderKind) -> Self {
        Self {
            endpoint: kind.default_endpoint().to_string(),
            api_key: String::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct AppSettings {
    providers: BTreeMap<ProviderKind, ProviderConfig>,
}

impl Default for AppSettings {
    fn default() -> Self {
        let mut providers = BTreeMap::new();
        for kind in ProviderKind::ALL {
            providers.insert(kind, ProviderConfig::default_for(kind));
        }
        Self { providers }
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

            settings
                .providers
                .insert(kind, ProviderConfig { endpoint, api_key });
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
        for kind in ProviderKind::ALL {
            let config = self.provider(kind);
            let group = kind.key();
            key_file.set_string(group, "endpoint", &config.endpoint);
            key_file.set_string(group, "api_key", &config.api_key);
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

    pub fn provider(&self, kind: ProviderKind) -> &ProviderConfig {
        self.providers
            .get(&kind)
            .expect("all ProviderKind variants are initialized")
    }

    pub fn set_endpoint(&mut self, kind: ProviderKind, endpoint: String) {
        if let Some(config) = self.providers.get_mut(&kind) {
            config.endpoint = endpoint;
        }
    }

    pub fn set_api_key(&mut self, kind: ProviderKind, api_key: String) {
        if let Some(config) = self.providers.get_mut(&kind) {
            config.api_key = api_key;
        }
    }

    pub fn configured_provider_count(&self) -> usize {
        ProviderKind::ALL
            .into_iter()
            .filter(|&kind| {
                let cfg = self.provider(kind);
                if kind.api_key_optional() {
                    !cfg.endpoint.trim().is_empty()
                } else {
                    !cfg.endpoint.trim().is_empty() && !cfg.api_key.trim().is_empty()
                }
            })
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_settings_file() {
        let dir = std::env::temp_dir().join(format!(
            "blabber-llm-settings-test-{}",
            std::process::id()
        ));
        let path = dir.join("settings.conf");

        let mut settings = AppSettings::default();
        settings.set_endpoint(ProviderKind::OpenAi, "https://custom.openai.example/v1".into());
        settings.set_api_key(ProviderKind::OpenAi, "sk-test-123".into());
        settings.set_api_key(ProviderKind::Claude, "sk-ant-456".into());
        settings.set_endpoint(ProviderKind::Ollama, "http://127.0.0.1:11434".into());

        settings.save_to_path(&path).expect("save settings");
        let loaded = AppSettings::load_from_path(&path);

        assert_eq!(
            loaded.provider(ProviderKind::OpenAi).endpoint,
            "https://custom.openai.example/v1"
        );
        assert_eq!(loaded.provider(ProviderKind::OpenAi).api_key, "sk-test-123");
        assert_eq!(loaded.provider(ProviderKind::Claude).api_key, "sk-ant-456");
        assert_eq!(
            loaded.provider(ProviderKind::Gemini).endpoint,
            ProviderKind::Gemini.default_endpoint()
        );
        assert_eq!(
            loaded.provider(ProviderKind::Ollama).endpoint,
            "http://127.0.0.1:11434"
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
