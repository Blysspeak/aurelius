pub mod brave;
pub mod cache;
pub mod perplexity;

use serde::{Deserialize, Serialize};

/// A single web search result, normalized across providers so callers never
/// need to know which backend produced it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub description: String,
}

/// Which search backend `search_web` should hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Brave,
    Perplexity,
}

impl Provider {
    /// Case-insensitive parse of a provider name (`"brave"` / `"perplexity"`).
    /// Returns `None` for anything else so the caller can report the allowed
    /// values.
    pub fn parse(s: &str) -> Option<Provider> {
        match s.to_ascii_lowercase().as_str() {
            "brave" => Some(Provider::Brave),
            "perplexity" => Some(Provider::Perplexity),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Provider::Brave => "brave",
            Provider::Perplexity => "perplexity",
        }
    }
}

/// Resolve an API key: environment variable first, then a config file under
/// `~/.config/aurelius/<file_name>`. Shared by every provider so the lookup
/// order and error message stay identical across them.
pub(crate) fn resolve_api_key(env_var: &str, file_name: &str) -> anyhow::Result<String> {
    if let Ok(key) = std::env::var(env_var) {
        return Ok(key);
    }
    // Fallback: read from config file
    let config_path = dirs_next::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join("aurelius")
        .join(file_name);
    if config_path.exists() {
        let key = std::fs::read_to_string(&config_path)?.trim().to_owned();
        if !key.is_empty() {
            return Ok(key);
        }
    }
    anyhow::bail!("{env_var} not set and ~/.config/aurelius/{file_name} not found")
}
