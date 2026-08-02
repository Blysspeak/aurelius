//! Resolves which data/config directory `au`/`aurelius` use: the
//! `AURELIUS_HOME` env var (one-off override — handy for a single test
//! command without disturbing anything persisted), else a directory
//! persisted via `au home use`, else the OS default data/config dirs (the
//! common case — nothing set here changes existing installs at all).
//!
//! The pointer file itself always lives at the *real* OS config dir,
//! never inside a home — it has to be findable before any home is known.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

fn pointer_path() -> Option<PathBuf> {
    Some(
        dirs_next::config_dir()?
            .join("aurelius")
            .join("active-home"),
    )
}

/// The currently active home override, if any. `None` means "use the OS
/// default data/config directories" — existing behavior, unchanged.
pub fn resolve() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("AURELIUS_HOME") {
        if !home.is_empty() {
            return Some(PathBuf::from(home));
        }
    }
    let contents = std::fs::read_to_string(pointer_path()?).ok()?;
    let trimmed = contents.trim();
    (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
}

/// Persists `path` as the active home — every subsequent `au`/`aurelius`
/// invocation on this machine uses it, from any shell, with no env var
/// needed, until `reset()` or an `AURELIUS_HOME` override takes over.
pub fn use_home(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path)
        .with_context(|| format!("failed to create {}", path.display()))?;
    let pointer =
        pointer_path().ok_or_else(|| anyhow::anyhow!("could not determine config directory"))?;
    if let Some(parent) = pointer.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    std::fs::write(&pointer, path.to_string_lossy().as_bytes())
        .with_context(|| format!("failed to write {}", pointer.display()))?;
    Ok(())
}

/// Clears the persisted active-home pointer, reverting to OS defaults.
/// Does not touch an `AURELIUS_HOME` env var, which still overrides.
pub fn reset() -> Result<()> {
    if let Some(pointer) = pointer_path() {
        if pointer.exists() {
            std::fs::remove_file(&pointer)
                .with_context(|| format!("failed to remove {}", pointer.display()))?;
        }
    }
    Ok(())
}
