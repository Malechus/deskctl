//! deskctl - a small CLI tool for managing wallpapers (feh) and display
//! layouts (xrandr), intended to be invoked directly or from a systemd timer.
//!
//! LEARNING NOTE: `mod` declarations below tell Rust which files make up our
//! crate (a "crate" is Rust's word for a compiled package/library/binary).
//! Unlike Java (where file location + package declaration define the module
//! tree) or C# (namespaces), Rust requires each module to be explicitly
//! declared with `mod name;`, which then looks for `src/name.rs` (or
//! `src/name/mod.rs`).
mod cli;
mod config;
mod display;
mod state;
mod systemd;
mod wallpaper;

use anyhow::{Context, Result};
use clap::Parser;
use cli::{Cli, Command, DisplayAction, WallpaperAction};
use config::Config;

/// LEARNING NOTE: `fn main() -> Result<()>` lets us use the `?` operator in
/// `main` itself. If any `?` produces an `Err`, Rust prints it via the
/// `Debug` implementation and exits with a non-zero status - this is how we
/// get clean error reporting without writing our own top-level try/catch.
fn main() -> Result<()> {
    // Initializes logging so `log::info!` etc. actually print. Controlled by
    // the RUST_LOG env var, e.g. `RUST_LOG=info deskctl wallpaper next`.
    // Under systemd, anything printed to stdout/stderr is captured by
    // journald automatically - no extra logging setup needed for that.
    env_logger::init();

    let cli = Cli::parse();

    // Handle `--genconfig` first, and return immediately - it deliberately
    // does not require (or attempt to load) an existing config file, since
    // its whole purpose is to create one.
    //
    // LEARNING NOTE: `as_deref()` turns `Option<String>` into `Option<&str>`
    // without consuming/moving the `String` out of `cli`. This is a common
    // Rust pattern: borrow a view into an `Option<String>` rather than
    // taking ownership, similar in spirit to passing a `string` by reference
    // in C# but enforced by the type system rather than by convention.
    if let Some(raw_path) = cli.genconfig.as_deref() {
        let explicit = if raw_path.is_empty() { None } else { Some(raw_path) };
        let wallpaper_dir = cli.wallpaper_dir.as_deref().map(std::path::Path::new);
        let path = config::generate(explicit, wallpaper_dir)?;
        println!("generated config file at {}", path.display());
        return Ok(());
    }

    // Handle `--enable` next, also returning immediately. Unlike
    // `--genconfig`, this *does* try to read the config file - so the
    // installed timer picks up your `[timer].interval` setting - but it
    // tolerates a missing/unparsable config by falling back to the same
    // "2h" default `TimerConfig` uses, rather than failing outright. That
    // way `deskctl --enable` still works standalone (e.g. before you've run
    // `--genconfig` at all).
    if cli.enable {
        let interval = match config::Config::load(cli.config.as_deref()) {
            Ok(config) => config.timer.interval,
            Err(err) => {
                log::warn!(
                    "could not load config ({err:#}), using default timer interval of {}",
                    config::default_timer_interval()
                );
                config::default_timer_interval()
            }
        };

        let unit_dir = systemd::enable(&interval)?;
        println!(
            "installed and enabled systemd units in {} (interval: {interval})",
            unit_dir.display()
        );
        println!("check status with: systemctl --user status deskctl-wallpaper.timer");
        return Ok(());
    }

    // LEARNING NOTE: `let ... else` (a "let-else" statement, stabilized in
    // Rust 1.65) lets us pattern-match on `Some(command)` and bind it, or
    // `bail!` (which is `anyhow`'s way of returning early with an error) if
    // it was `None`. It's a tidy alternative to a full `match` when there's
    // only one "failure" case to bail out of.
    let Some(command) = cli.command else {
        anyhow::bail!(
            "no command given - run `deskctl --help` for usage, or `deskctl --genconfig` to create a config file"
        );
    };

    let config = Config::load(cli.config.as_deref())?;

    match command {
        Command::Wallpaper { action } => handle_wallpaper(action, &config)?,
        Command::Display { action } => handle_display(action, &config)?,
    }

    Ok(())
}

fn handle_wallpaper(action: WallpaperAction, config: &Config) -> Result<()> {
    match action {
        WallpaperAction::List => {
            let count = display::count_connected_monitors()?;
            match config.wallpapers.get(&count.to_string()) {
                Some(profiles) => {
                    println!("wallpaper profiles for {count} connected screen(s), in cycle order:");
                    for p in profiles {
                        let images: Vec<String> =
                            p.images.iter().map(|i| i.display().to_string()).collect();
                        println!("  {} -> {}", p.name, images.join(", "));
                    }
                }
                None => println!("no wallpaper profiles configured for {count} connected screen(s)"),
            }
        }
        WallpaperAction::Next => {
            let count = display::count_connected_monitors()?;
            let profiles = config
                .wallpapers
                .get(&count.to_string())
                .with_context(|| {
                    format!(
                        "no wallpaper profiles configured for {count} connected screen(s) - \
                         add a [[wallpapers.{count}]] entry to your config"
                    )
                })?;

            // Look up where we left off last time (per screen count), work
            // out the next profile in sequence, apply it, then persist the
            // new position so the *following* `next` call continues from
            // here rather than restarting at the beginning.
            let current_name = state::read_current_profile(count)?;
            let next = wallpaper::next_profile(profiles, current_name.as_deref())?;
            let next_name = next.name.clone();

            wallpaper::apply_profile(next)?;
            state::write_current_profile(count, &next_name)?;

            println!("applied wallpaper profile '{next_name}' across {count} screen(s)");
        }
        WallpaperAction::Apply { profile } => {
            let count = display::count_connected_monitors()?;
            let profiles = config
                .wallpapers
                .get(&count.to_string())
                .with_context(|| format!("no wallpaper profiles configured for {count} connected screen(s)"))?;

            let Some(p) = profiles.iter().find(|p| p.name == profile) else {
                let available: Vec<&str> = profiles.iter().map(|p| p.name.as_str()).collect();
                anyhow::bail!(
                    "no wallpaper profile named '{profile}' for {count} connected screen(s) (available: {available:?})"
                );
            };

            wallpaper::apply_profile(p)?;
            // Manually applying a profile also updates the cycle position,
            // so a later `wallpaper next` continues on from wherever you
            // jumped to - matching how the original bash script's `case`
            // statement worked regardless of how `currentSetting` got set.
            state::write_current_profile(count, &profile)?;

            println!("applied wallpaper profile '{profile}' across {count} screen(s)");
        }
        WallpaperAction::Set { path } => {
            wallpaper::set_multiple(&[std::path::PathBuf::from(&path)])?;
            println!("wallpaper set: {path}");
        }
    }
    Ok(())
}

fn handle_display(action: DisplayAction, config: &Config) -> Result<()> {
    match action {
        DisplayAction::List => {
            let monitors = display::list_monitors()?;
            for m in monitors {
                let mode = m.current_mode.as_deref().unwrap_or("-");
                println!(
                    "{:<10} {:<12} mode={}",
                    m.name,
                    if m.connected { "connected" } else { "disconnected" },
                    mode
                );
            }
        }
        DisplayAction::Apply { profile } => {
            let Some(p) = config.displays.get(&profile) else {
                anyhow::bail!(
                    "no display profile named '{profile}' found in config (available: {:?})",
                    config.displays.keys().collect::<Vec<_>>()
                );
            };
            display::apply_profile(&profile, p)?;
            println!("applied display profile: {profile}");
        }
    }
    Ok(())
}
