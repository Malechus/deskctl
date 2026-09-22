//! Wallpaper management via `feh`.
//!
//! LEARNING NOTE: This module shows how Rust calls out to external programs.
//! `std::process::Command` is a builder: each method call (`.arg(...)`) returns
//! `&mut Self`, so we can chain calls fluently, similar to how you might chain
//! `ProcessStartInfo` configuration in C#, but built into the standard library
//! with no extra dependency needed.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::WallpaperProfile;

/// List image files directly inside `dir` (not recursive - kept simple),
/// sorted alphabetically so results are deterministic and repeatable (a
/// `std::fs::read_dir` iterator's order is not guaranteed by the OS).
/// Used by `config::generate` to build real wallpaper profiles from a
/// user-supplied `--wallpaper-dir`.
pub fn list_images(dir: &Path) -> Result<Vec<PathBuf>> {
    let entries = std::fs::read_dir(dir)
        .with_context(|| format!("failed to read wallpaper directory {}", dir.display()))?;

    let mut images = Vec::new();
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            let ext = ext.to_lowercase();
            if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "bmp") {
                images.push(path);
            }
        }
    }
    images.sort();
    Ok(images)
}

/// Given the list of profiles configured for the current screen count and
/// the name of the profile that was last applied (if any), return the
/// *next* profile to apply, cycling forward in the order profiles appear in
/// the config file and wrapping back to the start at the end.
///
/// This is a deliberately pure function (no I/O, no side effects) - it just
/// computes an index. That makes it trivial to unit test (see the `tests`
/// module below) without needing a real config file or `feh` installed.
///
/// LEARNING NOTE: the return type `&WallpaperProfile` borrows from the
/// `profiles` slice passed in, rather than cloning it. The lifetime of that
/// borrow is tied to `profiles` by Rust's borrow checker - the compiler will
/// refuse to compile code that tries to use the returned reference after
/// `profiles` goes out of scope. This is the "no null, no dangling pointer"
/// guarantee ownership/borrowing gives you, enforced at compile time rather
/// than caught at runtime like a Java/C# `NullReferenceException` might be.
pub fn next_profile<'a>(
    profiles: &'a [WallpaperProfile],
    current_name: Option<&str>,
) -> Result<&'a WallpaperProfile> {
    if profiles.is_empty() {
        bail!("wallpaper profile list is empty");
    }

    let next_index = match current_name.and_then(|name| profiles.iter().position(|p| p.name == name)) {
        // Found the current profile in the list - advance to the next one,
        // wrapping around with `%` once we pass the last index.
        Some(current_index) => (current_index + 1) % profiles.len(),
        // No recorded current profile (first run), or it no longer exists
        // in the config (e.g. renamed/removed) - start from the beginning.
        None => 0,
    };

    Ok(&profiles[next_index])
}

/// Apply a wallpaper profile: set its explicit image list via `feh`, one
/// image per screen, in the order they're listed in the config.
pub fn apply_profile(profile: &WallpaperProfile) -> Result<()> {
    if profile.images.is_empty() {
        bail!("wallpaper profile '{}' has no images configured", profile.name);
    }

    set_multiple(&profile.images)?;
    notify(&profile.name, profile.notification_image.as_deref());
    Ok(())
}

/// Set wallpapers directly via `feh --bg-fill`, one image per screen, in
/// the given order.
///
/// LEARNING NOTE: `feh` applies each `--bg-fill <path>` argument to the next
/// connected output in xrandr's reported order, so passing N images sets N
/// different wallpapers across N monitors in one call. We build the
/// `Command` dynamically with a loop rather than a fixed set of `.arg()`
/// calls, since we don't know the image count until runtime. `--no-fehbg`
/// matches the original `wallswitcher.sh` script: it tells feh not to write
/// a `~/.fehbg` restore script, since deskctl's config file is already the
/// source of truth for restoring wallpapers.
pub fn set_multiple(paths: &[PathBuf]) -> Result<()> {
    if paths.is_empty() {
        bail!("no wallpaper paths given to set");
    }

    log::info!("setting {} wallpaper(s) via feh: {:?}", paths.len(), paths);

    let mut cmd = Command::new("feh");
    for path in paths {
        cmd.arg("--bg-fill").arg(path);
    }
    cmd.arg("--no-fehbg");

    let status = cmd
        .status()
        .context("failed to launch `feh` - is it installed and on PATH?")?;

    if !status.success() {
        bail!("feh exited with a non-zero status: {status}");
    }

    Ok(())
}

/// Best-effort desktop notification, mirroring the `notify-send` call in
/// `wallswitcher.sh`. Deliberately does not return `Result`/fail the overall
/// command: a missing `notify-send` binary or a failed notification
/// shouldn't stop the wallpaper itself from having been set successfully.
/// We just log a warning and move on.
///
/// `image` is the profile's optional `notification_image`: `Some(path)`
/// attaches it via `notify-send -i <path>`, while `None` (or, defensively,
/// an empty path) skips the `-i` flag entirely rather than passing
/// `notify-send` a broken/empty icon argument.
///
/// LEARNING NOTE: `Option<&Path>` parameter (rather than `Option<PathBuf>`)
/// lets the caller pass a *borrowed* view of the profile's field without
/// transferring ownership - `profile.notification_image.as_deref()` in
/// `apply_profile` does exactly this conversion from `&Option<PathBuf>` to
/// `Option<&Path>`.
fn notify(profile_name: &str, image: Option<&Path>) {
    let mut cmd = Command::new("notify-send");
    cmd.arg("-t").arg("2000");

    if let Some(image_path) = image
        && !image_path.as_os_str().is_empty()
    {
        cmd.arg("-i").arg(image_path);
    }

    cmd.arg("Wallpaper Change")
        .arg(format!("Setting wallpaper to {profile_name}"));

    if let Err(err) = cmd.status() {
        log::warn!("failed to send desktop notification (notify-send not installed?): {err}");
    }
}

// LEARNING NOTE: `#[cfg(test)]` marks this module as test-only code - it's
// compiled and run by `cargo test` but stripped out of normal builds
// entirely, so it adds zero size/cost to the real `deskctl` binary. This is
// Rust's built-in unit testing convention: tests live alongside the code
// they test, in the same file, rather than a separate test project like
// you'd typically see in Java/C#.
#[cfg(test)]
mod tests {
    use super::*;

    fn profile(name: &str) -> WallpaperProfile {
        WallpaperProfile {
            name: name.to_string(),
            images: vec![PathBuf::from(format!("{name}.png"))],
            notification_image: None,
        }
    }

    #[test]
    fn first_run_picks_first_profile() {
        let profiles = vec![profile("a"), profile("b"), profile("c")];
        let next = next_profile(&profiles, None).unwrap();
        assert_eq!(next.name, "a");
    }

    #[test]
    fn advances_to_next_profile_in_order() {
        let profiles = vec![profile("a"), profile("b"), profile("c")];
        let next = next_profile(&profiles, Some("a")).unwrap();
        assert_eq!(next.name, "b");
    }

    #[test]
    fn wraps_around_after_last_profile() {
        let profiles = vec![profile("a"), profile("b"), profile("c")];
        let next = next_profile(&profiles, Some("c")).unwrap();
        assert_eq!(next.name, "a");
    }

    #[test]
    fn falls_back_to_first_profile_if_current_name_unknown() {
        let profiles = vec![profile("a"), profile("b")];
        let next = next_profile(&profiles, Some("renamed-or-deleted")).unwrap();
        assert_eq!(next.name, "a");
    }

    #[test]
    fn empty_profile_list_is_an_error() {
        let profiles: Vec<WallpaperProfile> = vec![];
        assert!(next_profile(&profiles, None).is_err());
    }
}
