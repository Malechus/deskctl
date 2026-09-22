//! Systemd user-unit installation for the wallpaper-cycling timer.
//!
//! LEARNING NOTE: `include_str!("../systemd/deskctl-wallpaper.service")` is a
//! compile-time macro that reads a file *at build time* and embeds its
//! contents directly into the compiled binary as a `&'static str`. This is
//! very different from reading a file at runtime (`std::fs::read_to_string`)
//! - there's no file I/O happening when `--enable` runs, and no dependency
//!   on the `systemd/` directory existing on disk (which it wouldn't, if
//!   someone installed deskctl via `cargo install` rather than cloning the
//!   repo). Think of it as similar to a C#/Java "embedded resource", but
//!   resolved by the compiler rather than bundled into an assembly/jar at
//!   packaging time.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

const SERVICE_UNIT: &str = include_str!("../systemd/deskctl-wallpaper.service");
const TIMER_UNIT: &str = include_str!("../systemd/deskctl-wallpaper.timer");

const SERVICE_NAME: &str = "deskctl-wallpaper.service";
const TIMER_NAME: &str = "deskctl-wallpaper.timer";

/// Install deskctl's systemd user units and enable+start the timer, so the
/// caller doesn't have to copy/paste the `mkdir`/`cp`/`systemctl` steps from
/// the README by hand.
///
/// `interval` is a systemd time span (e.g. "2h", "30min") sourced from the
/// user's config file (`[timer].interval`, see `config::TimerConfig`) - it's
/// substituted into the embedded timer unit's `OnUnitActiveSec=` line before
/// writing it out, so the on-disk unit always reflects the current config
/// rather than whatever value happens to be hardcoded in the template.
pub fn enable(interval: &str) -> Result<PathBuf> {
    let unit_dir = user_unit_dir()?;

    std::fs::create_dir_all(&unit_dir)
        .with_context(|| format!("failed to create systemd user unit directory {}", unit_dir.display()))?;

    write_unit(&unit_dir, SERVICE_NAME, SERVICE_UNIT)?;
    write_unit(&unit_dir, TIMER_NAME, &render_timer_unit(interval))?;

    // Unit files changed on disk, so systemd needs to be told to re-read
    // them before `enable` will see the new/updated timer.
    run_systemctl(&["daemon-reload"])?;
    // `enable --now` both creates the WantedBy symlink (so the timer starts
    // automatically on future logins) and starts it immediately.
    run_systemctl(&["enable", "--now", TIMER_NAME])?;

    Ok(unit_dir)
}

/// Take the embedded `deskctl-wallpaper.timer` template and rewrite its
/// `OnUnitActiveSec=` line to use `interval` instead of the template's own
/// hardcoded default.
///
/// LEARNING NOTE: `.lines()` returns an iterator over `&str` slices, one per
/// line (without the trailing `\n`). `.map()` transforms each line through a
/// closure (`|line| ...`, Rust's lambda syntax - like a Java lambda or C#
/// `Func<>`, but the compiler infers the parameter type here from context).
/// `.collect::<Vec<_>>()` then gathers the resulting `String`s back into a
/// `Vec`, and `.join("\n")` stitches them into one string again - a common
/// Rust idiom for "transform every line, then reassemble the text" that
/// avoids writing a manual loop with a mutable accumulator.
fn render_timer_unit(interval: &str) -> String {
    let body = TIMER_UNIT
        .lines()
        .map(|line| {
            if line.starts_with("OnUnitActiveSec=") {
                format!("OnUnitActiveSec={interval}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    // `.lines()` drops the file's trailing newline, so add one back to keep
    // the written-out unit file well-formed (POSIX text files should end
    // with a newline).
    body + "\n"
}

/// Resolve `$XDG_CONFIG_HOME/systemd/user`, falling back to
/// `~/.config/systemd/user` per the XDG Base Directory spec - the same
/// fallback logic `config::resolve_path` uses for the config file.
fn user_unit_dir() -> Result<PathBuf> {
    let config_home = match std::env::var("XDG_CONFIG_HOME") {
        Ok(v) if !v.is_empty() => PathBuf::from(v),
        _ => {
            let home = std::env::var("HOME").context("HOME environment variable is not set")?;
            PathBuf::from(home).join(".config")
        }
    };

    Ok(config_home.join("systemd").join("user"))
}

fn write_unit(dir: &Path, filename: &str, contents: &str) -> Result<()> {
    let path = dir.join(filename);
    std::fs::write(&path, contents)
        .with_context(|| format!("failed to write systemd unit file to {}", path.display()))?;
    Ok(())
}

/// Run `systemctl --user <args>`, treating a non-zero exit status as an
/// error (`Command::status` alone only tells us *whether* it ran, not
/// whether systemctl considered the operation successful).
fn run_systemctl(args: &[&str]) -> Result<()> {
    let status = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .status()
        .with_context(|| format!("failed to run `systemctl --user {}` - is systemd installed?", args.join(" ")))?;

    if !status.success() {
        anyhow::bail!("`systemctl --user {}` exited with {status}", args.join(" "));
    }

    Ok(())
}

// LEARNING NOTE: same `#[cfg(test)]` convention as `wallpaper.rs` - these
// tests only exercise `render_timer_unit`, which is pure string manipulation
// with no filesystem or `systemctl` calls, so they run instantly and need no
// mocking.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutes_configured_interval() {
        let rendered = render_timer_unit("45min");
        assert!(rendered.contains("OnUnitActiveSec=45min"));
        assert!(!rendered.contains("OnUnitActiveSec=2h"));
    }

    #[test]
    fn preserves_other_lines_unchanged() {
        let rendered = render_timer_unit("1h");
        assert!(rendered.contains("[Timer]"));
        assert!(rendered.contains("OnBootSec=5min"));
        assert!(rendered.contains("WantedBy=timers.target"));
    }

    #[test]
    fn ends_with_trailing_newline() {
        let rendered = render_timer_unit("2h");
        assert!(rendered.ends_with('\n'));
    }
}
