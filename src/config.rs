//! Configuration loading (and generation) for deskctl.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::{display, wallpaper};

/// The root of our config.toml file.
#[derive(Deserialize, Serialize, Debug)]
pub struct Config {
    /// Ordered named xrandr display profiles. The order determines how
    /// `deskctl display next` cycles through them.
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_display_profiles")]
    pub displays: Vec<DisplayProfile>,

    /// Ordered wallpaper profiles, each explicitly linked to one or more
    /// display profile names.
    #[serde(default)]
    pub wallpapers: Vec<WallpaperProfile>,

    /// Settings for the systemd timer that `deskctl --enable` installs.
    #[serde(default)]
    pub timer: TimerConfig,
}

/// Settings for the systemd timer that `deskctl --enable` installs.
#[derive(Deserialize, Serialize, Debug)]
pub struct TimerConfig {
    /// How often the wallpaper timer fires, in systemd time span syntax
    /// (e.g. "30min", "2h", "1h30min") - this becomes the `OnUnitActiveSec=`
    /// value in the generated `deskctl-wallpaper.timer` unit.
    #[serde(default = "default_timer_interval")]
    pub interval: String,
}

/// The default timer interval used both when generating a new config
/// (`--genconfig`) and when an existing config's `[timer]` table (or the
/// `interval` field within it) is missing.
pub fn default_timer_interval() -> String {
    "2h".to_string()
}

impl Default for TimerConfig {
    fn default() -> Self {
        Self {
            interval: default_timer_interval(),
        }
    }
}

/// A single named xrandr profile: a list of raw arguments to pass to xrandr.
/// Keeping this as raw args (rather than trying to model every xrandr flag)
/// keeps the tool flexible - you write the xrandr invocation you'd type by
/// hand, and deskctl just replays it. Profiles in the `[[displays]]` array
/// retain their config-file order for `deskctl display next`.
#[derive(Deserialize, Serialize, Debug)]
pub struct DisplayProfile {
    /// Profile name, used with `deskctl display apply <name>`.
    pub name: String,
    /// Full argument list passed to `xrandr`, e.g.
    /// ["--output", "eDP-1", "--mode", "1920x1080", "--output", "HDMI-1", "--off"]
    pub args: Vec<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum DisplayProfilesConfig {
    Ordered(Vec<DisplayProfile>),
    Legacy(BTreeMap<String, LegacyDisplayProfile>),
}

#[derive(Deserialize)]
struct LegacyDisplayProfile {
    args: Vec<String>,
}

fn deserialize_display_profiles<'de, D>(
    deserializer: D,
) -> std::result::Result<Vec<DisplayProfile>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let profiles = match DisplayProfilesConfig::deserialize(deserializer)? {
        DisplayProfilesConfig::Ordered(profiles) => profiles,
        DisplayProfilesConfig::Legacy(profiles) => profiles
            .into_iter()
            .map(|(name, profile)| DisplayProfile {
                name,
                args: profile.args,
            })
            .collect(),
    };

    let mut names = std::collections::HashSet::new();
    for profile in &profiles {
        if !names.insert(&profile.name) {
            return Err(serde::de::Error::custom(format!(
                "duplicate display profile name '{}'",
                profile.name
            )));
        }
    }

    Ok(profiles)
}

/// A single named wallpaper profile: an explicit, ordered list of image
/// files - one per screen, in xrandr output order - mirroring how the
/// original `wallswitcher.sh` script hardcoded `feh --bg-fill img1 img2
/// img3 img4`. There is no randomness here: the same profile always sets
/// the same images, in the same screen order, every time it's applied.
///
/// Profiles are declared as TOML array-of-tables (`[[wallpapers]]`); their
/// order in the file determines the cycling order within each linked display.
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct WallpaperProfile {
    /// Profile name, used with `deskctl wallpaper apply <name>` and shown in
    /// notifications/state tracking.
    pub name: String,
    /// Display profile names this wallpaper profile is compatible with.
    pub displays: Vec<String>,
    /// Explicit image paths, one per screen, in the same order xrandr
    /// reports connected outputs (see `deskctl display list`).
    pub images: Vec<PathBuf>,

    /// Optional image to attach to the desktop notification sent when this
    /// profile is applied (via `notify-send -i <path>`). `None` (the field
    /// is omitted entirely from the TOML file) or an empty string both mean
    /// "send the notification without an image."
    #[serde(default)]
    pub notification_image: Option<PathBuf>,
}

impl Config {
    /// Iterate over wallpaper profiles linked to `display_name`, preserving
    /// their order from the config file.
    pub fn wallpapers_for_display(
        &self,
        display_name: &str,
    ) -> impl Iterator<Item = &WallpaperProfile> {
        self.wallpapers
            .iter()
            .filter(move |profile| profile.displays.iter().any(|name| name == display_name))
    }

    /// Load config from an explicit path, or fall back to the default
    /// `$XDG_CONFIG_HOME/deskctl/config.toml` location.
    pub fn load(explicit_path: Option<&str>) -> Result<Self> {
        let path = resolve_path(explicit_path)?;

        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read config file at {}", path.display()))?;

        let mut config: Config = toml::from_str(&raw)
            .with_context(|| format!("failed to parse config file at {}", path.display()))?;

        config.validate()?;

        for profile in &mut config.wallpapers {
            for image in &mut profile.images {
                *image = expand_path(image)?;
            }
            if let Some(image) = &mut profile.notification_image {
                *image = expand_path(image)?;
            }
        }

        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        let mut wallpaper_names = std::collections::HashSet::new();

        for profile in &self.wallpapers {
            if !wallpaper_names.insert(&profile.name) {
                bail!("duplicate wallpaper profile name '{}'", profile.name);
            }
            if profile.displays.is_empty() {
                bail!(
                    "wallpaper profile '{}' must list at least one compatible display in `displays`",
                    profile.name
                );
            }

            for display_name in &profile.displays {
                let Some(display_profile) = self
                    .displays
                    .iter()
                    .find(|display| display.name == *display_name)
                else {
                    bail!(
                        "wallpaper profile '{}' references unknown display profile '{}'",
                        profile.name,
                        display_name
                    );
                };

                let screen_count = display::active_output_count(display_profile);
                if profile.images.len() != screen_count {
                    log::warn!(
                        "wallpaper profile '{}' has {} image(s), but linked display profile '{}' enables {} screen(s)",
                        profile.name,
                        profile.images.len(),
                        display_name,
                        screen_count
                    );
                }
            }
        }

        Ok(())
    }
}

/// Expand a leading `~`/`~/` and any `$VAR` / `${VAR}` references in a path.
/// Shells do this before a program sees its arguments, but not for paths read
/// from the config file or passed as `--opt=~/x`. `~user` is left alone, a
/// `$` not followed by a variable name stays literal, and referencing an
/// unset variable is an error so typos don't silently become wrong paths.
pub fn expand_path(path: &Path) -> Result<PathBuf> {
    expand_path_with(path, |name| std::env::var(name).ok())
}

fn expand_path_with(path: &Path, lookup: impl Fn(&str) -> Option<String>) -> Result<PathBuf> {
    let Some(text) = path.to_str() else {
        return Ok(path.to_path_buf());
    };

    let mut rest = text;
    let mut out = String::new();
    if rest == "~" || rest.starts_with("~/") {
        out.push_str(&lookup("HOME").context("cannot expand '~': HOME is not set")?);
        rest = &rest[1..];
    }

    while let Some(pos) = rest.find('$') {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + 1..];

        let (name, consumed) = if let Some(braced) = after.strip_prefix('{') {
            match braced.find('}') {
                Some(end) => (&braced[..end], end + 2),
                None => bail!("unterminated '${{' in path '{text}'"),
            }
        } else {
            let len = after
                .char_indices()
                .find(|&(i, c)| !(c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit())))
                .map_or(after.len(), |(i, _)| i);
            (&after[..len], len)
        };

        if name.is_empty() {
            out.push('$');
            rest = after;
            continue;
        }

        let value = lookup(name)
            .with_context(|| format!("environment variable '{name}' (used in path '{text}') is not set"))?;
        out.push_str(&value);
        rest = &after[consumed..];
    }
    out.push_str(rest);

    Ok(PathBuf::from(out))
}

/// Resolve the config path we should use: the explicit path if one was
/// given (and non-empty), otherwise `$XDG_CONFIG_HOME/deskctl/config.toml`,
/// falling back to `~/.config/deskctl/config.toml` if `XDG_CONFIG_HOME` isn't
/// set (this fallback is what the XDG Base Directory spec itself mandates).
fn resolve_path(explicit_path: Option<&str>) -> Result<PathBuf> {
    if let Some(p) = explicit_path
        && !p.is_empty()
    {
        return expand_path(Path::new(p));
    }

    let config_home = match std::env::var("XDG_CONFIG_HOME") {
        Ok(v) if !v.is_empty() => PathBuf::from(v),
        _ => {
            let home = std::env::var("HOME").context("HOME environment variable is not set")?;
            Path::new(&home).join(".config")
        }
    };

    Ok(config_home.join("deskctl").join("config.toml"))
}

/// Generate a starter config file at the resolved path (explicit path, or
/// the default `$XDG_CONFIG_HOME` location) and return where it was written.
/// Refuses to overwrite an existing file so you don't accidentally clobber a
/// config you've already customized.
pub fn generate(explicit_path: Option<&str>, wallpaper_dir: Option<&Path>) -> Result<PathBuf> {
    let path = resolve_path(explicit_path)?;

    if path.exists() {
        bail!(
            "config file already exists at {} - remove it first or pass a different path with --genconfig=<path>",
            path.display()
        );
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config directory {}", parent.display()))?;
    }

    let config = build_config(wallpaper_dir)?;

    let contents = toml::to_string_pretty(&config).context("failed to serialize generated config to TOML")?;

    std::fs::write(&path, contents)
        .with_context(|| format!("failed to write config file to {}", path.display()))?;

    Ok(path)
}

/// The config content written by `--genconfig`. The `displays` list always
/// reflects the machine deskctl is actually running on right now (built
/// from a live `xrandr --query`, via `display::build_detected_profile`) -
/// applying it immediately after generation is always safe. The
/// `wallpapers` list depends on whether a wallpaper image directory was
/// supplied:
/// - `Some(dir)`: real images found in `dir` are split into two profiles
///   ("set-1", "set-2"), each assigning one image per currently connected
///   screen and linked to the generated "detected" display profile.
/// - `None`: writes placeholder profiles linked to "detected" that you
///   edit by hand.
fn build_config(wallpaper_dir: Option<&Path>) -> Result<Config> {
    let monitors = display::list_monitors()
        .context("failed to query xrandr to build a display profile for --genconfig")?;
    let screen_count = monitors.iter().filter(|m| m.connected).count();

    let displays = vec![display::build_detected_profile(&monitors)];

    let wallpapers = match wallpaper_dir {
        Some(dir) => real_wallpaper_profiles(dir, "detected", screen_count)?,
        None => placeholder_wallpaper_profiles("detected", screen_count),
    };

    let config = Config {
        displays,
        wallpapers,
        timer: TimerConfig::default(),
    };
    config.validate()?;
    Ok(config)
}

/// Build two wallpaper profiles ("set-1" and "set-2") from real image files
/// found in `dir`, each assigning `screen_count` images (one per currently
/// connected screen). If `dir` contains fewer than `2 * screen_count`
/// images, we cycle back through the list with `%` rather than failing -
/// good enough for a generated starting point, since you can always edit
/// the file afterward.
fn real_wallpaper_profiles(dir: &Path, display_name: &str, screen_count: usize) -> Result<Vec<WallpaperProfile>> {
    let images = wallpaper::list_images(dir)
        .with_context(|| format!("failed to list images in --wallpaper-dir {}", dir.display()))?;

    if images.is_empty() {
        bail!("no image files (.png/.jpg/.jpeg/.bmp) found in {}", dir.display());
    }

    // Keep generated profiles non-empty even when xrandr reports no connected monitors.
    let per_profile = screen_count.max(1);

    let build_profile = |name: &str, start: usize| WallpaperProfile {
        name: name.to_string(),
        displays: vec![display_name.to_string()],
        images: (0..per_profile)
            .map(|i| images[(start + i) % images.len()].clone())
            .collect(),
        notification_image: images.first().cloned(),
    };

    Ok(vec![
        build_profile("set-1", 0),
        build_profile("set-2", per_profile),
    ])
}

/// Placeholder wallpaper profiles used when `--genconfig` is run without a
/// `--wallpaper-dir` - example profiles you edit by hand.
fn placeholder_wallpaper_profiles(display_name: &str, screen_count: usize) -> Vec<WallpaperProfile> {
    let paths = [
        "/home/CHANGE_ME/Pictures/Wallpapers/nature/001.jpg",
        "/home/CHANGE_ME/Pictures/Wallpapers/nature/002.jpg",
    ];
    let images = (0..screen_count.max(1))
        .map(|index| PathBuf::from(paths[index % paths.len()]))
        .collect::<Vec<_>>();
    vec![
        WallpaperProfile {
            name: "nature".to_string(),
            displays: vec![display_name.to_string()],
            images: images.clone(),
            notification_image: images.first().cloned(),
        },
        WallpaperProfile {
            name: "dark".to_string(),
            displays: vec![display_name.to_string()],
            images,
            notification_image: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_display_profiles_keep_config_order() {
        let config: Config = toml::from_str(
            r#"
            [[displays]]
            name = "work"
            args = ["--output", "DP-1", "--auto"]

            [[displays]]
            name = "home"
            args = ["--output", "HDMI-1", "--auto"]
            "#,
        )
        .unwrap();

        let names: Vec<_> = config.displays.iter().map(|profile| profile.name.as_str()).collect();
        assert_eq!(names, ["work", "home"]);
    }

    #[test]
    fn legacy_display_tables_are_still_loaded() {
        let config: Config = toml::from_str(
            r#"
            [displays.work]
            args = ["--output", "DP-1", "--auto"]

            [displays.home]
            args = ["--output", "HDMI-1", "--auto"]
            "#,
        )
        .unwrap();

        let names: Vec<_> = config.displays.iter().map(|profile| profile.name.as_str()).collect();
        assert_eq!(names, ["home", "work"]);
    }

    #[test]
    fn duplicate_display_names_are_rejected() {
        let parsed = toml::from_str::<Config>(
            r#"
            [[displays]]
            name = "same"
            args = ["--output", "DP-1", "--auto"]

            [[displays]]
            name = "same"
            args = ["--output", "HDMI-1", "--auto"]
            "#,
        );

        assert!(parsed.is_err());
    }

    #[test]
    fn wallpaper_profile_must_reference_configured_displays() {
        let config: Config = toml::from_str(
            r#"
            [[displays]]
            name = "work"
            args = ["--output", "DP-1", "--auto"]

            [[wallpapers]]
            name = "work-wallpaper"
            displays = ["missing"]
            images = ["one.jpg"]
            "#,
        )
        .unwrap();

        assert!(config.validate().is_err());
    }

    #[test]
    fn wallpaper_profile_requires_display_links() {
        let config: Config = toml::from_str(
            r#"
            [[displays]]
            name = "work"
            args = ["--output", "DP-1", "--auto"]

            [[wallpapers]]
            name = "unlinked"
            displays = []
            images = ["one.jpg"]
            "#,
        )
        .unwrap();

        assert!(config.validate().is_err());
    }

    #[test]
    fn wallpaper_profiles_can_link_to_multiple_displays() {
        let config: Config = toml::from_str(
            r#"
            [[displays]]
            name = "work"
            args = ["--output", "DP-1", "--auto"]

            [[displays]]
            name = "personal"
            args = ["--output", "HDMI-1", "--auto"]

            [[wallpapers]]
            name = "shared"
            displays = ["work", "personal"]
            images = ["one.jpg"]
            "#,
        )
        .unwrap();

        config.validate().unwrap();
        assert_eq!(config.wallpapers_for_display("work").count(), 1);
        assert_eq!(config.wallpapers_for_display("personal").count(), 1);
    }

    #[test]
    fn example_config_has_valid_display_links() {
        let config: Config = toml::from_str(include_str!("../config.example.toml")).unwrap();
        config.validate().unwrap();
    }

    #[test]
    fn image_count_mismatch_is_a_warning_not_a_config_error() {
        let config: Config = toml::from_str(
            r#"
            [[displays]]
            name = "work"
            args = ["--output", "DP-1", "--auto", "--output", "DP-2", "--auto"]

            [[wallpapers]]
            name = "mismatched"
            displays = ["work"]
            images = ["one.jpg"]
            "#,
        )
        .unwrap();

        assert!(config.validate().is_ok());
    }

    fn expand(p: &str) -> Result<PathBuf> {
        expand_path_with(Path::new(p), |name| match name {
            "HOME" => Some("/home/me".to_string()),
            "WALLS" => Some("/data/walls".to_string()),
            _ => None,
        })
    }

    #[test]
    fn expands_tilde() {
        assert_eq!(expand("~").unwrap(), PathBuf::from("/home/me"));
        assert_eq!(expand("~/a.jpg").unwrap(), PathBuf::from("/home/me/a.jpg"));
    }

    #[test]
    fn expands_variables() {
        assert_eq!(expand("$WALLS/a.jpg").unwrap(), PathBuf::from("/data/walls/a.jpg"));
        assert_eq!(expand("${WALLS}x/a.jpg").unwrap(), PathBuf::from("/data/wallsx/a.jpg"));
        assert_eq!(expand("~/$WALLS").unwrap(), PathBuf::from("/home/me//data/walls"));
    }

    #[test]
    fn leaves_other_text_alone() {
        for p in ["/abs/~/x", "rel/x", "~user/x", "a~b", "cost$", "a$/b", "$1x"] {
            assert_eq!(expand(p).unwrap(), PathBuf::from(p));
        }
    }

    #[test]
    fn unset_or_malformed_variable_is_an_error() {
        assert!(expand("$NOPE/x").is_err());
        assert!(expand("${NOPE}").is_err());
        assert!(expand("${WALLS").is_err());
    }
}
