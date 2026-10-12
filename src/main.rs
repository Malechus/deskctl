//! deskctl - a small CLI tool for managing wallpapers (feh) and display
//! layouts (xrandr), intended to be invoked directly or from a systemd timer.
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

fn main() -> Result<()> {
    // Initializes logging so `log::info!` etc. actually print. Controlled by
    // the RUST_LOG env var, e.g. `RUST_LOG=info deskctl wallpaper next`.
    // Under systemd, anything printed to stdout/stderr is captured by
    // journald automatically - no extra logging setup needed for that.
    env_logger::init();

    let cli = Cli::parse();
    cli.exit_if_help_requested();

    // Handle `--genconfig` first, and return immediately - it deliberately
    // does not require (or attempt to load) an existing config file, since
    // its whole purpose is to create one.
    if let Some(raw_path) = cli.genconfig.as_deref() {
        let explicit = if raw_path.is_empty() { None } else { Some(raw_path) };
        let wallpaper_dir = cli
            .wallpaper_dir
            .as_deref()
            .map(|d| config::expand_path(std::path::Path::new(d)))
            .transpose()?;
        let path = config::generate(explicit, wallpaper_dir.as_deref())?;
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

    // With no subcommand, behave like `deskctl wallpaper next`.
    let command = cli.command.unwrap_or(Command::Wallpaper {
        action: WallpaperAction::Next,
    });

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
            let display_name = active_display_name(config)?;
            let profiles: Vec<_> = config.wallpapers_for_display(&display_name).collect();
            if profiles.is_empty() {
                println!("no wallpaper profiles configured for display '{display_name}'");
            } else {
                println!(
                    "wallpaper profiles compatible with display '{display_name}', in cycle order:"
                );
                for profile in profiles {
                    let images: Vec<String> = profile
                        .images
                        .iter()
                        .map(|image| image.display().to_string())
                        .collect();
                    println!("  {} -> {}", profile.name, images.join(", "));
                }
            }
        }
        WallpaperAction::Next => {
            let display_name = active_display_name(config)?;
            let profiles: Vec<_> = config.wallpapers_for_display(&display_name).collect();
            let applicable: Vec<_> = profiles.into_iter().cloned().collect();
            if applicable.is_empty() {
                anyhow::bail!(
                    "no wallpaper profiles configured for display '{display_name}' - add a [[wallpapers]] entry with displays = [\"{display_name}\"]"
                );
            }

            let current_name = state::read_current_profile(&display_name)?;
            let next = wallpaper::next_profile(&applicable, current_name.as_deref())?;
            let next_name = next.name.clone();

            wallpaper::apply_profile(next)?;
            state::write_current_profile(&display_name, &next_name)?;

            println!("applied wallpaper profile '{next_name}' for display '{display_name}'");
        }
        WallpaperAction::Apply { profile } => {
            let display_name = active_display_name(config)?;
            let profiles: Vec<_> = config.wallpapers_for_display(&display_name).collect();
            let Some(p) = profiles.iter().find(|p| p.name == profile) else {
                let available: Vec<&str> = profiles.iter().map(|p| p.name.as_str()).collect();
                anyhow::bail!(
                    "no wallpaper profile named '{profile}' compatible with display '{display_name}' (available: {available:?})"
                );
            };

            wallpaper::apply_profile(p)?;
            state::write_current_profile(&display_name, &profile)?;

            println!("applied wallpaper profile '{profile}' for display '{display_name}'");
        }
        WallpaperAction::Set { path } => {
            let path = config::expand_path(std::path::Path::new(&path))?;
            wallpaper::set_multiple(std::slice::from_ref(&path))?;
            println!("wallpaper set: {}", path.display());
        }
    }
    Ok(())
}

fn active_display_name(config: &Config) -> Result<String> {
    let display_name = match state::read_current_display_profile()? {
        Some(name) => name,
        None => {
            let monitors = display::list_monitors()?;
            let detected = display::detect_profile(&config.displays, &monitors)
                .context("no display profile is active and none matches the current xrandr layout - run `deskctl display apply <profile>` first")?;
            log::info!("detected active display profile '{}' from xrandr", detected.name);
            state::write_current_display_profile(&detected.name)?;
            detected.name.clone()
        }
    };
    if !config.displays.iter().any(|profile| profile.name == display_name) {
        anyhow::bail!(
            "active display profile '{display_name}' is not configured - apply a configured display profile first"
        );
    }
    Ok(display_name)
}

fn handle_display(action: DisplayAction, config: &Config) -> Result<()> {
    match action {
        DisplayAction::List => {
            if config.displays.is_empty() {
                println!("no display profiles configured");
            } else {
                println!("display profiles in cycle order:");
                for profile in &config.displays {
                    println!("  {}", profile.name);
                }
            }

            println!("connected monitors:");
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
        DisplayAction::Next => {
            let current_name = state::read_current_display_profile()?;
            let next = display::next_profile(&config.displays, current_name.as_deref())
                .with_context(|| {
                    "no display profiles configured - add [[displays]] entries to your config"
                })?;
            let next_name = next.name.clone();

            display::apply_profile(&next_name, next)?;
            state::write_current_display_profile(&next_name)?;

            println!("applied display profile: {next_name}");
        }
        DisplayAction::Apply { profile } => {
            let Some(p) = config.displays.iter().find(|p| p.name == profile) else {
                anyhow::bail!(
                    "no display profile named '{profile}' found in config (available: {:?})",
                    config.displays.iter().map(|p| &p.name).collect::<Vec<_>>()
                );
            };
            display::apply_profile(&profile, p)?;
            state::write_current_display_profile(&profile)?;
            println!("applied display profile: {profile}");
        }
    }
    Ok(())
}
