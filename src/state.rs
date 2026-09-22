//! Cycling state for `deskctl wallpaper next`.
//!
//! LEARNING NOTE: this mirrors what `wallswitcher.sh` did with
//! `$XDG_CACHE_HOME/wallSetting` - remembering which profile was last applied
//! so the *next* invocation knows where to advance to. We deliberately keep
//! this in `$XDG_CACHE_HOME` rather than `$XDG_CONFIG_HOME`: config is
//! something a user writes and edits by hand, while cache is
//! machine-generated bookkeeping that's safe to delete at any time (deskctl
//! will just start cycling from the first profile again).

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Resolve `$XDG_CACHE_HOME/deskctl`, falling back to `~/.cache/deskctl` if
/// `XDG_CACHE_HOME` isn't set (again, this fallback is mandated by the XDG
/// Base Directory spec itself).
fn cache_dir() -> Result<PathBuf> {
    let base = match std::env::var("XDG_CACHE_HOME") {
        Ok(v) if !v.is_empty() => PathBuf::from(v),
        _ => {
            let home = std::env::var("HOME").context("HOME environment variable is not set")?;
            Path::new(&home).join(".cache")
        }
    };
    Ok(base.join("deskctl"))
}

/// One state file per screen count, since the profile list (and therefore
/// "current position") differs depending on how many screens are connected.
fn state_file(screen_count: usize) -> Result<PathBuf> {
    Ok(cache_dir()?.join(format!("wallpaper-state-{screen_count}")))
}

/// Read the name of the last-applied wallpaper profile for `screen_count`,
/// if any has been recorded yet.
///
/// LEARNING NOTE: the return type is `Result<Option<String>>` - a `Result`
/// (this can fail, e.g. a permissions error) wrapping an `Option` (there
/// might legitimately be no prior state, which isn't an error - it just
/// means "first run"). This nested-wrapper pattern is common in Rust and
/// lets you distinguish "failed to check" from "checked, found nothing".
pub fn read_current_profile(screen_count: usize) -> Result<Option<String>> {
    let path = state_file(screen_count)?;
    if !path.exists() {
        return Ok(None);
    }

    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read wallpaper state file {}", path.display()))?;

    let trimmed = contents.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.to_string()))
    }
}

/// Persist `profile_name` as the current wallpaper profile for `screen_count`.
pub fn write_current_profile(screen_count: usize, profile_name: &str) -> Result<()> {
    let path = state_file(screen_count)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create cache directory {}", parent.display()))?;
    }

    std::fs::write(&path, profile_name)
        .with_context(|| format!("failed to write wallpaper state file {}", path.display()))?;

    Ok(())
}
