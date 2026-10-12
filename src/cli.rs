//! Command-line interface definition for deskctl.

use clap::{Parser, Subcommand};

/// Command-line options for deskctl.
#[derive(Parser)]
#[command(name = "deskctl", version, about = "Manage wallpapers and display layouts via feh and xrandr")]
pub struct Cli {
    /// Path to an alternate config file (defaults to $XDG_CONFIG_HOME/deskctl/config.toml)
    #[arg(short, long)]
    pub config: Option<String>,

    /// Generate a starter config file and exit. Optionally pass a path
    /// (`--genconfig=/some/path.toml`); with no path, writes to
    /// $XDG_CONFIG_HOME/deskctl/config.toml (falling back to ~/.config/...).
    #[arg(long, num_args = 0..=1, default_missing_value = "")]
    pub genconfig: Option<String>,

    /// Directory of wallpaper images to use when building the generated
    /// config's wallpaper profiles (only meaningful together with
    /// `--genconfig`). If omitted, `--genconfig` falls back to writing
    /// placeholder wallpaper profiles that you edit by hand afterward.
    #[arg(long, requires = "genconfig")]
    pub wallpaper_dir: Option<String>,

    /// Install deskctl's systemd user units (service + timer) to
    /// `$XDG_CONFIG_HOME/systemd/user` and enable+start the timer, then
    /// exit. Equivalent to manually copying the files from `systemd/` and
    /// running `systemctl --user daemon-reload && systemctl --user enable
    /// --now deskctl-wallpaper.timer` yourself. The timer's interval comes
    /// from `[timer].interval` in your config file (default: "2h" if the
    /// config, or just that field, is missing).
    #[arg(long)]
    pub enable: bool,

    /// The subcommand to run (wallpaper, display, ...). Optional so that
    /// `deskctl --genconfig` works without also requiring a subcommand.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Top-level command groups.
#[derive(Subcommand)]
pub enum Command {
    /// Wallpaper-related actions (set via feh)
    Wallpaper {
        #[command(subcommand)]
        action: WallpaperAction,
    },
    /// Display layout actions (set via xrandr)
    Display {
        #[command(subcommand)]
        action: DisplayAction,
    },
}

#[derive(Subcommand)]
pub enum WallpaperAction {
    /// List wallpaper profiles available for the current number of connected screens
    List,
    /// Advance to the next wallpaper profile (in configured order) matching the current screen count and apply it
    Next,
    /// Apply a specific named wallpaper profile (must match the current screen count)
    Apply { profile: String },
    /// Set a specific wallpaper file directly on every screen (bypasses profiles)
    Set { path: String },
}

#[derive(Subcommand)]
pub enum DisplayAction {
    /// List configured display profiles and currently connected monitors
    List,
    /// Advance to the next display profile in configured order and apply it
    Next,
    /// Apply a named display profile from the config file
    Apply { profile: String },
}
