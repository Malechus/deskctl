//! Display layout management via `xrandr`.

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
/// Output headers are followed by indented mode lines. Track the current
/// output so its active mode can be assigned to the correct monitor.
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
    DisplayProfile {
        name: "detected".to_string(),
        args,
    }
}

/// Count outputs that are configured on in an xrandr argument list.
/// Each `--output NAME` starts a section that ends at the next `--output`;
/// sections containing `--off` do not contribute to the screen count.
pub fn active_output_count(profile: &DisplayProfile) -> usize {
    let mut active_outputs = 0;
    let mut reading_output = false;
    let mut output_is_off = false;

    for arg in &profile.args {
        if arg == "--output" {
            if reading_output && !output_is_off {
                active_outputs += 1;
            }
            reading_output = true;
            output_is_off = false;
        } else if reading_output && arg == "--off" {
            output_is_off = true;
        }
    }

    if reading_output && !output_is_off {
        active_outputs += 1;
    }

    active_outputs
}

/// What a profile's args ask of one output.
struct OutputSpec<'a> {
    name: &'a str,
    off: bool,
    mode: Option<&'a str>,
    rotation: &'a str,
}

fn parse_output_specs(profile: &DisplayProfile) -> Vec<OutputSpec<'_>> {
    let mut specs: Vec<OutputSpec> = Vec::new();
    let mut args = profile.args.iter().map(String::as_str);
    while let Some(arg) = args.next() {
        match arg {
            "--output" => {
                if let Some(name) = args.next() {
                    specs.push(OutputSpec {
                        name,
                        off: false,
                        mode: None,
                        rotation: "normal",
                    });
                }
            }
            "--off" => {
                if let Some(spec) = specs.last_mut() {
                    spec.off = true;
                }
            }
            "--mode" => {
                if let (Some(mode), Some(spec)) = (args.next(), specs.last_mut()) {
                    spec.mode = Some(mode);
                }
            }
            "--rotate" => {
                if let (Some(rotation), Some(spec)) = (args.next(), specs.last_mut()) {
                    spec.rotation = rotation;
                }
            }
            _ => {}
        }
    }
    specs
}

/// Whether the monitors' current state is what `profile` would produce.
/// Outputs the profile doesn't mention must be inactive; outputs it turns
/// off must be inactive; outputs it enables must be active with the
/// requested mode (if any) and rotation. Position is not compared.
fn profile_matches(profile: &DisplayProfile, monitors: &[Monitor]) -> bool {
    let specs = parse_output_specs(profile);
    if specs.is_empty() {
        return false;
    }
    let is_active = |m: &Monitor| m.connected && m.current_mode.is_some();

    let all_specs_hold = specs.iter().all(|spec| {
        let monitor = monitors.iter().find(|m| m.name == spec.name);
        if spec.off {
            return monitor.is_none_or(|m| !is_active(m));
        }
        monitor.is_some_and(|m| {
            is_active(m)
                && spec.mode.is_none_or(|mode| m.current_mode.as_deref() == Some(mode))
                && m.rotation.as_deref().unwrap_or("normal") == spec.rotation
        })
    });

    all_specs_hold
        && monitors
            .iter()
            .filter(|m| is_active(m))
            .all(|m| specs.iter().any(|spec| spec.name == m.name && !spec.off))
}

/// Find the first configured profile (in config order) matching the
/// monitor layout xrandr currently reports.
pub fn detect_profile<'a>(
    profiles: &'a [DisplayProfile],
    monitors: &[Monitor],
) -> Option<&'a DisplayProfile> {
    profiles.iter().find(|p| profile_matches(p, monitors))
}

/// Given the configured display profiles and the last profile applied,
/// return the next profile in config-file order, wrapping at the end.
pub fn next_profile<'a>(
    profiles: &'a [DisplayProfile],
    current_name: Option<&str>,
) -> Result<&'a DisplayProfile> {
    if profiles.is_empty() {
        bail!("display profile list is empty");
    }

    let next_index =
        match current_name.and_then(|name| profiles.iter().position(|p| p.name == name)) {
            Some(current_index) => (current_index + 1) % profiles.len(),
            None => 0,
        };

    Ok(&profiles[next_index])
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

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(name: &str) -> DisplayProfile {
        DisplayProfile {
            name: name.to_string(),
            args: vec![
                "--output".to_string(),
                "DP-1".to_string(),
                "--auto".to_string(),
            ],
        }
    }

    #[test]
    fn first_run_picks_first_profile() {
        let profiles = vec![profile("home"), profile("work")];
        assert_eq!(next_profile(&profiles, None).unwrap().name, "home");
    }

    #[test]
    fn advances_and_wraps_in_order() {
        let profiles = vec![profile("home"), profile("work")];
        assert_eq!(next_profile(&profiles, Some("home")).unwrap().name, "work");
        assert_eq!(next_profile(&profiles, Some("work")).unwrap().name, "home");
    }

    #[test]
    fn unknown_current_profile_restarts_at_first() {
        let profiles = vec![profile("home"), profile("work")];
        assert_eq!(
            next_profile(&profiles, Some("removed")).unwrap().name,
            "home"
        );
    }

    #[test]
    fn empty_profile_list_is_an_error() {
        assert!(next_profile(&[], None).is_err());
    }

    fn monitor(name: &str, mode: Option<&str>, rotation: Option<&str>) -> Monitor {
        Monitor {
            name: name.to_string(),
            connected: true,
            current_mode: mode.map(str::to_string),
            rotation: rotation.map(str::to_string),
        }
    }

    fn profile_with(name: &str, args: &[&str]) -> DisplayProfile {
        DisplayProfile {
            name: name.to_string(),
            args: args.iter().map(|a| a.to_string()).collect(),
        }
    }

    #[test]
    fn detects_matching_profile_by_mode_rotation_and_off_outputs() {
        let single = profile_with("single", &["--output", "DP-0", "--off", "--output", "HDMI-0", "--mode", "1920x1080"]);
        let dual = profile_with(
            "dual",
            &["--output", "DP-0", "--mode", "1080x1920", "--rotate", "left", "--output", "HDMI-0", "--mode", "1920x1080"],
        );
        let profiles = vec![single, dual];

        let one = vec![monitor("DP-0", None, None), monitor("HDMI-0", Some("1920x1080"), None)];
        assert_eq!(detect_profile(&profiles, &one).unwrap().name, "single");

        let two = vec![
            monitor("DP-0", Some("1080x1920"), Some("left")),
            monitor("HDMI-0", Some("1920x1080"), None),
        ];
        assert_eq!(detect_profile(&profiles, &two).unwrap().name, "dual");

        let wrong_rotation = vec![
            monitor("DP-0", Some("1080x1920"), None),
            monitor("HDMI-0", Some("1920x1080"), None),
        ];
        assert!(detect_profile(&profiles, &wrong_rotation).is_none());
    }

    #[test]
    fn counts_only_outputs_not_marked_off() {
        let profile = DisplayProfile {
            name: "work".to_string(),
            args: vec![
                "--output".to_string(),
                "DP-1".to_string(),
                "--mode".to_string(),
                "1920x1080".to_string(),
                "--output".to_string(),
                "DP-2".to_string(),
                "--off".to_string(),
                "--output".to_string(),
                "HDMI-1".to_string(),
                "--auto".to_string(),
            ],
        };
        assert_eq!(active_output_count(&profile), 2);
    }
}
