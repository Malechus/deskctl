//! Command-line interface definition for deskctl.
//!
//! LEARNING NOTE: In Rust, the `clap` crate lets us describe a CLI declaratively
//! using structs/enums annotated with `#[derive(...)]` macros. The derive macro
//! generates all the argument-parsing code for us at compile time - we never
//! hand-write a parser. This is very different from, say, C# where you'd often
//! reach for a library like System.CommandLine and configure it imperatively.

use clap::{Parser, Subcommand};

/// Top-level CLI struct. `#[derive(Parser)]` turns this into a full argument
/// parser. The doc comment above the struct becomes the `--help` description.
#[derive(Parser)]
#[command(name = "deskctl", version, about = "Manage wallpapers and display layouts via feh and xrandr")]
pub struct Cli {
    /// Path to an alternate config file (defaults to $XDG_CONFIG_HOME/deskctl/config.toml)
    #[arg(short, long)]
    pub config: Option<String>,

    /// Generate a starter config file and exit. Optionally pass a path
    /// (`--genconfig=/some/path.toml`); with no path, writes to
    /// $XDG_CONFIG_HOME/deskctl/config.toml (falling back to ~/.config/...).
    ///
    /// LEARNING NOTE: `num_args = 0..=1` tells clap this flag can appear with
    /// zero or one value. `default_missing_value` supplies a value ("",
    /// meaning "use the default location") when the flag is present but no
    /// value was typed. Without these, clap would require a value whenever
    /// `--genconfig` appears at all.
    #[arg(long, num_args = 0..=1, default_missing_value = "")]
    pub genconfig: Option<String>,

    /// Directory of wallpaper images to use when building the generated
    /// config's wallpaper profiles (only meaningful together with
    /// `--genconfig`). If omitted, `--genconfig` falls back to writing
    /// placeholder wallpaper profiles that you edit by hand afterward.
    ///
    /// LEARNING NOTE: `requires = "genconfig"` tells clap to reject the
    /// command line at parse time if `--wallpaper-dir` is given without
    /// `--genconfig` also being present - a validation rule expressed
    /// declaratively rather than something we'd have to check by hand in
    /// `main`.
    #[arg(long, requires = "genconfig")]
    pub wallpaper_dir: Option<String>,

    /// The subcommand to run (wallpaper, display, ...). Optional so that
    /// `deskctl --genconfig` works without also requiring a subcommand.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// `#[derive(Subcommand)]` on an enum means each variant becomes a CLI
/// subcommand, e.g. `deskctl wallpaper next`. Rust enums are more powerful than
/// C#/Java enums: each variant can carry its own data (here, nested subcommands).
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
    /// List currently connected monitors (via `xrandr --query`)
    List,
    /// Apply a named display profile from the config file
    Apply { profile: String },
}
