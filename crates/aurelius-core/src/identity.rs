use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::OnceLock;

/// A person's durable, cross-project identity: name + contact identifier.
/// Configured once per machine at `~/.config/aurelius/identity.toml`,
/// independent of any single project (mirrors the `~/.config/aurelius/brave.key`
/// pattern used by the Brave Search client).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    pub name: String,
    pub email: String,
}

impl Identity {
    /// Git-style `"Name <email>"`, stamped onto every node/edge this person
    /// creates or updates.
    pub fn as_author(&self) -> String {
        format!("{} <{}>", self.name, self.email)
    }

    fn config_path() -> Result<PathBuf> {
        let dir = dirs_next::config_dir()
            .ok_or_else(|| anyhow::anyhow!("could not determine config directory"))?
            .join("aurelius");
        Ok(dir.join("identity.toml"))
    }

    /// Loads the identity config, if any. Returns `Ok(None)` when unset
    /// (not an error — most local-only usage never configures this), and
    /// `Err` only when the file exists but can't be read or parsed.
    pub fn load() -> Result<Option<Self>> {
        let path = Self::config_path()?;
        if !path.exists() {
            return Ok(None);
        }
        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let identity: Identity = toml::from_str(&contents)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        Ok(Some(identity))
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::config_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let contents = toml::to_string_pretty(self)?;
        std::fs::write(&path, contents)
            .with_context(|| format!("failed to write {}", path.display()))?;
        Ok(())
    }
}

static CURRENT: OnceLock<Option<Identity>> = OnceLock::new();

/// Cached lookup of the local identity config, read from disk at most once
/// per process. Returns `None` if identity has not been configured — callers
/// fall back to unattributed local operation rather than failing.
pub fn current() -> Option<Identity> {
    CURRENT
        .get_or_init(|| Identity::load().unwrap_or(None))
        .clone()
}
