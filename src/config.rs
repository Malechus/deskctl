//! Configuration loading (and generation) for deskctl.
//!
//! LEARNING NOTE: `serde` is Rust's de-facto serialization framework. Instead
//! of writing a parser by hand (or using reflection like Java/C# JSON
//! libraries often do), we `#[derive(Deserialize, Serialize)]` on plain
//! structs and serde generates both directions of conversion at compile
//! time. `Deserialize` lets us parse TOML into these structs; `Serialize`
//! (needed for `--genconfig`) lets us go the other way and turn a struct
//! back into TOML text.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::{display, wallpaper};

/// The root of our config.toml file.
///
/// LEARNING NOTE: we use `BTreeMap` here rather than `HashMap`. Both map
/// string keys to values (like `Dictionary<string, T>` in C# or
/// `HashMap<String, T>` in Java), but `BTreeMap` keeps its keys in sorted
/// order. That doesn't matter for *looking up* a profile by name, but it
/// does matter when we *write* the config back out for `--genconfig`: a
/// `BTreeMap` always serializes in the same (sorted) order, whereas a
/// `HashMap`'s iteration order is unspecified and can vary between runs.
#[derive(Deserialize, Serialize, Debug)]
pub struct Config {
    /// Named xrandr display profiles, e.g. "docked" -> DisplayProfile.
    #[serde(default)]
    pub displays: BTreeMap<String, DisplayProfile>,

    /// Wallpaper profiles, grouped by *number of connected screens*.
    /// The map key is the screen count as a string (TOML table keys are
    /// always strings, e.g. `[[wallpapers.2]]`), and the value is an
    /// *ordered* list of named profiles for that screen count. `deskctl
    /// wallpaper next` walks through this list in order (wrapping back to
    /// the start), remembering its position in a state file - it never
    /// picks randomly.
    #[serde(default)]
    pub wallpapers: BTreeMap<String, Vec<WallpaperProfile>>,
}

/// A single named xrandr profile: a list of raw arguments to pass to xrandr.
/// Keeping this as raw args (rather than trying to model every xrandr flag)
/// keeps the tool flexible - you write the xrandr invocation you'd type by
/// hand, and deskctl just replays it.
#[derive(Deserialize, Serialize, Debug)]
pub struct DisplayProfile {
    /// Full argument list passed to `xrandr`, e.g.
    /// ["--output", "eDP-1", "--mode", "1920x1080", "--output", "HDMI-1", "--off"]
    pub args: Vec<String>,
}

/// A single named wallpaper profile: an explicit, ordered list of image
/// files - one per screen, in xrandr output order - mirroring how the
/// original `wallswitcher.sh` script hardcoded `feh --bg-fill img1 img2
/// img3 img4`. There is no randomness here: the same profile always sets
/// the same images, in the same screen order, every time it's applied.
///
/// LEARNING NOTE: this struct lives inside a `Vec<WallpaperProfile>` (see
/// `Config::wallpapers` above), so in TOML it's written using "array of
/// tables" syntax: `[[wallpapers.2]]` starts a new entry in the list for the
/// "2 screens" key. Each `[[wallpapers.2]]` block becomes one
/// `WallpaperProfile` in that `Vec`, and `Vec` preserves the order entries
/// appeared in the file - which is exactly what lets `wallpaper next` cycle
/// through them in a predictable, repeatable sequence.
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct WallpaperProfile {
    /// Profile name, used with `deskctl wallpaper apply <name>` and shown in
    /// notifications/state tracking.
    pub name: String,
    /// Explicit image paths, one per screen, in the same order xrandr
    /// reports connected outputs (see `deskctl display list`).
    pub images: Vec<PathBuf>,

    /// Optional image to attach to the desktop notification sent when this
    /// profile is applied (via `notify-send -i <path>`). `None` (the field
    /// is omitted entirely from the TOML file) or an empty string both mean
    /// "send the notification without an image."
    ///
    /// LEARNING NOTE: `#[serde(default)]` on an `Option<T>` field means the
    /// field is allowed to be missing from the TOML file entirely, in which
    /// case serde fills in `None` automatically - without it, serde would
    /// require every profile to explicitly write `notification_image = ""`
    /// or similar, which would be tedious for the common case of "no icon."
    #[serde(default)]
    pub notification_image: Option<PathBuf>,
}

impl Config {
    /// Load config from an explicit path, or fall back to the default
    /// `$XDG_CONFIG_HOME/deskctl/config.toml` location.
    ///
    /// LEARNING NOTE: `Option<String>` here mirrors what we saw in cli.rs -
    /// the caller may or may not have passed `--config`. `Result<Self>` means
    /// this function can fail (missing file, bad TOML); Rust forces callers to
    /// handle that explicitly, there is no silent null or uncaught exception.
    pub fn load(explicit_path: Option<&str>) -> Result<Self> {
        let path = resolve_path(explicit_path)?;

        // `with_context` from anyhow lets us attach a human-readable message
        // to any error as it bubbles up, without losing the original cause.
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read config file at {}", path.display()))?;

        let config: Config = toml::from_str(&raw)
            .with_context(|| format!("failed to parse config file at {}", path.display()))?;

        Ok(config)
    }
}

/// Resolve the config path we should use: the explicit path if one was
/// given (and non-empty), otherwise `$XDG_CONFIG_HOME/deskctl/config.toml`,
/// falling back to `~/.config/deskctl/config.toml` if `XDG_CONFIG_HOME` isn't
/// set (this fallback is what the XDG Base Directory spec itself mandates).
fn resolve_path(explicit_path: Option<&str>) -> Result<PathBuf> {
    // LEARNING NOTE: `if let Some(p) = x && condition` is "let-chains" - a
    // relatively new Rust feature that lets you combine a pattern match with
    // an extra boolean condition in one `if`, avoiding a nested `if` block.
    // clippy (Rust's built-in linter) suggested this collapse automatically.
    if let Some(p) = explicit_path
        && !p.is_empty()
    {
        return Ok(PathBuf::from(p));
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

    // `toml::to_string_pretty` is the mirror image of `toml::from_str`: it
    // serializes our struct back into human-readable TOML text.
    let contents = toml::to_string_pretty(&config).context("failed to serialize generated config to TOML")?;

    std::fs::write(&path, contents)
        .with_context(|| format!("failed to write config file to {}", path.display()))?;

    Ok(path)
}

/// The config content written by `--genconfig`. The `displays` map always
/// reflects the machine deskctl is actually running on right now (built
/// from a live `xrandr --query`, via `display::build_detected_profile`) -
/// applying it immediately after generation is always safe. The
/// `wallpapers` map depends on whether a wallpaper image directory was
/// supplied:
/// - `Some(dir)`: real images found in `dir` are split into two profiles
///   ("set-1", "set-2"), each assigning one image per *currently connected*
///   screen, keyed under the real detected screen count.
/// - `None`: falls back to the previous placeholder behavior - example
///   profiles under screen counts 1 and 2 with `CHANGE_ME` paths you edit
///   by hand.
fn build_config(wallpaper_dir: Option<&Path>) -> Result<Config> {
    let monitors = display::list_monitors()
        .context("failed to query xrandr to build a display profile for --genconfig")?;
    let screen_count = monitors.iter().filter(|m| m.connected).count();

    let mut displays = BTreeMap::new();
    displays.insert("detected".to_string(), display::build_detected_profile(&monitors));

    let wallpapers = match wallpaper_dir {
        Some(dir) => real_wallpaper_profiles(dir, screen_count)?,
        None => placeholder_wallpaper_profiles(),
    };

    Ok(Config {
        displays,
        wallpapers,
    })
}

/// Build two wallpaper profiles ("set-1" and "set-2") from real image files
/// found in `dir`, each assigning `screen_count` images (one per currently
/// connected screen). If `dir` contains fewer than `2 * screen_count`
/// images, we cycle back through the list with `%` rather than failing -
/// good enough for a generated starting point, since you can always edit
/// the file afterward.
fn real_wallpaper_profiles(dir: &Path, screen_count: usize) -> Result<BTreeMap<String, Vec<WallpaperProfile>>> {
    let images = wallpaper::list_images(dir)
        .with_context(|| format!("failed to list images in --wallpaper-dir {}", dir.display()))?;

    if images.is_empty() {
        bail!("no image files (.png/.jpg/.jpeg/.bmp) found in {}", dir.display());
    }

    // LEARNING NOTE: `.max(1)` guards against `screen_count` being 0 (e.g.
    // xrandr reporting no connected monitors at all) - we still want to
    // generate at least a one-image-per-profile config rather than an
    // empty, useless one.
    let per_profile = screen_count.max(1);

    let build_profile = |name: &str, start: usize| WallpaperProfile {
        name: name.to_string(),
        images: (0..per_profile)
            .map(|i| images[(start + i) % images.len()].clone())
            .collect(),
        notification_image: images.first().cloned(),
    };

    let mut wallpapers = BTreeMap::new();
    wallpapers.insert(
        screen_count.to_string(),
        vec![
            build_profile("set-1", 0),
            build_profile("set-2", per_profile),
        ],
    );
    Ok(wallpapers)
}

/// Placeholder wallpaper profiles used when `--genconfig` is run without a
/// `--wallpaper-dir` - the previous, hand-edit-required behavior.
fn placeholder_wallpaper_profiles() -> BTreeMap<String, Vec<WallpaperProfile>> {
    let mut wallpapers = BTreeMap::new();
    wallpapers.insert(
        "1".to_string(),
        vec![
            WallpaperProfile {
                name: "single-nature".to_string(),
                images: vec![PathBuf::from("/home/CHANGE_ME/Pictures/Wallpapers/nature/001.jpg")],
                notification_image: Some(PathBuf::from(
                    "/home/CHANGE_ME/Pictures/Wallpapers/nature/001.jpg",
                )),
            },
            WallpaperProfile {
                name: "single-dark".to_string(),
                images: vec![PathBuf::from("/home/CHANGE_ME/Pictures/Wallpapers/dark/001.jpg")],
                notification_image: None,
            },
        ],
    );
    wallpapers.insert(
        "2".to_string(),
        vec![WallpaperProfile {
            name: "dual-nature".to_string(),
            images: vec![
                PathBuf::from("/home/CHANGE_ME/Pictures/Wallpapers/nature/001.jpg"),
                PathBuf::from("/home/CHANGE_ME/Pictures/Wallpapers/nature/002.jpg"),
            ],
            notification_image: None,
        }],
    );
    wallpapers
}

