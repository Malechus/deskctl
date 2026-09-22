//! Display layout management via `xrandr`.
//!
//! LEARNING NOTE: Here we parse the *output* of an external command
//! (`xrandr --query`) as plain text. Rust has no built-in regex; the `regex`
//! crate comes in transitively via `env_logger`, but we deliberately avoid
//! depending on that directly and instead use simple string methods
//! (`.contains`, `.split_whitespace`) - this keeps the parsing easy to read
//! while you're still learning the language.

use anyhow::{bail, Context, Result};
use std::process::Command;

use crate::config::DisplayProfile;

/// A connected monitor as reported by `xrandr --query`.
#[derive(Debug)]
pub struct Monitor {
    pub name: String,
    pub connected: bool,
    /// The currently active mode *name* (e.g. "1920x1080"), taken from the
    /// mode list (the line marked with `*`) - this is the value xrandr's
    /// `--mode` flag expects. It intentionally is NOT the same as the
    /// geometry shown on the header line (`WxH+X+Y`), which reflects the
    /// screen's effective size *after* rotation is applied and is not a
    /// valid `--mode` argument for rotated outputs (see `rotation` below).
    pub current_mode: Option<String>,
    /// The output's current rotation, if not "normal" - one of "left",
    /// "right", or "inverted". xrandr only prints this token when it's
    /// non-default, so `None` here means normal/unrotated.
    pub rotation: Option<String>,
}

/// Run `xrandr --query` and parse the connected monitor list.
///
/// LEARNING NOTE: `xrandr --query` output alternates between "header" lines
/// (one per output, starting at column 0 - e.g. `DP-0 connected ...`) and
/// indented "mode" lines listing that output's supported resolutions, with
/// the currently active one marked by an `*`. To parse this correctly we
/// need to remember *which output* we're currently looking at as we walk
/// through its indented mode lines - `current` (an `Option<usize>` index
/// into `monitors`) is exactly that piece of running state.
pub fn list_monitors() -> Result<Vec<Monitor>> {
    let output = Command::new("xrandr")
        .arg("--query")
        .output()
        .context("failed to launch `xrandr` - is it installed and on PATH?")?;

    if !output.status.success() {
        bail!("xrandr exited with a non-zero status: {}", output.status);
    }

    // `output.stdout` is `Vec<u8>` (raw bytes). `String::from_utf8_lossy`
    // converts it to text, replacing any invalid UTF-8 rather than erroring -
    // fine here since xrandr output is expected to be plain ASCII/UTF-8.
    let text = String::from_utf8_lossy(&output.stdout);

    let mut monitors: Vec<Monitor> = Vec::new();
    // Index into `monitors` of the output block we're currently reading mode
    // lines for, or `None` before the first header line / while skipping
    // the "Screen 0: ..." summary line.
    let mut current: Option<usize> = None;

    for line in text.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            // An indented mode line, e.g. "   1920x1080     60.00*+  ...".
            // Only the currently-active mode (marked with '*') matters to
            // us; everything else is a resolution we're not using.
            if let Some(index) = current
                && line.contains('*')
                && let Some(mode_name) = line.split_whitespace().next()
            {
                monitors[index].current_mode = Some(mode_name.to_string());
            }
            continue;
        }

        // The first line of `xrandr --query` output is a summary line like
        // "Screen 0: minimum 8 x 8, current ...", not a monitor - skip it,
        // and make sure we don't treat later mode lines as belonging to it.
        if line.starts_with("Screen ") {
            current = None;
            continue;
        }

        // We've reached a new output header line, e.g.:
        //   "DP-0 connected 1080x1920+3014+1080 left (normal left ...) 521mm x 293mm"
        //   "HDMI-0 connected primary 1920x1080+1094+1080 (normal ...) 597mm x 336mm"
        //   "DP-1 disconnected (normal left inverted right x axis y axis)"
        let mut parts = line.split_whitespace().peekable();
        let Some(name) = parts.next() else { continue };
        let Some(state) = parts.next() else { continue };
        let connected = state == "connected";

        // Skip the optional "primary" token.
        if parts.peek() == Some(&"primary") {
            parts.next();
        }

        // If there's an active mode, the next token is geometry
        // (`WxH+X+Y`) - we only use its presence as a signal, not its
        // value, since the real mode name comes from the mode list below.
        if let Some(next) = parts.peek()
            && next.contains('x')
            && next.chars().next().is_some_and(|c| c.is_ascii_digit())
        {
            parts.next();
        }

        // Any remaining token before the `(properties...)` block is the
        // rotation - xrandr omits this entirely for "normal" rotation.
        let rotation = parts
            .peek()
            .filter(|t| !t.starts_with('('))
            .map(|t| t.to_string());

        monitors.push(Monitor {
            name: name.to_string(),
            connected,
            current_mode: None,
            rotation,
        });
        current = Some(monitors.len() - 1);
    }

    Ok(monitors)
}

/// Count how many monitors are currently reported as "connected" by xrandr.
/// This is what drives screen-count-based wallpaper profile selection - e.g.
/// docking a laptop (2 screens) vs. undocked (1 screen) picks a different
/// set of wallpaper profiles.
pub fn count_connected_monitors() -> Result<usize> {
    let monitors = list_monitors()?;
    Ok(monitors.iter().filter(|m| m.connected).count())
}

/// Build a `DisplayProfile` that reproduces the monitor layout xrandr
/// currently reports: each connected monitor keeps its current mode *and*
/// rotation (or `--auto`, letting xrandr pick a sensible default, if no
/// mode was detected), and every disconnected output is explicitly turned
/// off. This profile is always valid to apply immediately after
/// generation, since it's built from what's actually plugged in right now.
pub fn build_detected_profile(monitors: &[Monitor]) -> DisplayProfile {
    let mut args = Vec::new();
    for monitor in monitors {
        args.push("--output".to_string());
        args.push(monitor.name.clone());
        if monitor.connected {
            match &monitor.current_mode {
                Some(mode) => {
                    args.push("--mode".to_string());
                    args.push(mode.clone());
                }
                None => args.push("--auto".to_string()),
            }
            if let Some(rotation) = &monitor.rotation {
                args.push("--rotate".to_string());
                args.push(rotation.clone());
            }
        } else {
            args.push("--off".to_string());
        }
    }
    DisplayProfile { args }
}



pub fn apply_profile(name: &str, profile: &DisplayProfile) -> Result<()> {
    log::info!("applying display profile '{name}': xrandr {:?}", profile.args);

    let status = Command::new("xrandr")
        .args(&profile.args)
        .status()
        .context("failed to launch `xrandr` - is it installed and on PATH?")?;

    if !status.success() {
        bail!("xrandr exited with a non-zero status: {}", status);
    }

    Ok(())
}
