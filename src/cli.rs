//! Command-line interface definition for deskctl.

use clap::{CommandFactory, Parser, Subcommand};

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

impl Cli {
    /// If a subcommand's free-form string argument is the literal word
    /// `help`, print that subcommand's help (as `--help` would) and exit.
    pub fn exit_if_help_requested(&self) {
        let path: [&str; 2] = match &self.command {
            Some(Command::Wallpaper { action: WallpaperAction::Apply { profile } })
                if profile == "help" => ["wallpaper", "apply"],
            Some(Command::Wallpaper { action: WallpaperAction::Set { path } })
                if path == "help" => ["wallpaper", "set"],
            Some(Command::Display { action: DisplayAction::Apply { profile } })
                if profile == "help" => ["display", "apply"],
            _ => return,
        };

        // `build()` propagates parent names so usage reads `deskctl wallpaper apply ...`.
        let mut root = <Cli as CommandFactory>::command();
        root.build();
        if let Some(sub) = root
            .find_subcommand(path[0])
            .and_then(|group| group.find_subcommand(path[1]))
        {
            let mut sub = sub.clone();
            let _ = sub.print_long_help();
            std::process::exit(0);
        }
    }
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
    /// List wallpaper profiles compatible with the selected display profile
    List,
    /// Advance to the next wallpaper profile compatible with the selected display profile
    Next,
    /// Apply a named wallpaper profile compatible with the selected display profile
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
