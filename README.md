# deskctl

A small Rust CLI tool that manages desktop wallpapers (via `feh`) and display
layouts (via `xrandr`). Designed to be run directly from the command line or
triggered periodically by a systemd user timer.

Licensed under the MIT License.

## Requirements

- `feh` and `xrandr` installed and on `PATH`
- Rust toolchain (`cargo`/`rustc`) to build

## Third-party software and licenses

`deskctl` is distributed under the MIT License. The software it directly depends on is listed below for attribution, along with the relevant author or organization, license, and upstream link.

| Component | Author / Organization | License | Link |
| --- | --- | --- | --- |
| `anyhow` | David Tolnay | Apache-2.0 OR MIT | https://github.com/dtolnay/anyhow |
| `clap` | clap-rs contributors | Apache-2.0 OR MIT | https://github.com/clap-rs/clap |
| `env_logger` | rust-cli contributors | Apache-2.0 OR MIT | https://github.com/rust-cli/env_logger |
| `log` | The Rust Project Developers | Apache-2.0 OR MIT | https://github.com/rust-lang/log |
| `serde` | serde-rs contributors | Apache-2.0 OR MIT | https://github.com/serde-rs/serde |
| `toml` | toml-rs contributors | Apache-2.0 OR MIT | https://github.com/toml-rs/toml |
| `feh` | Tomáš Hubínek ("derf") | MIT-feh (MIT variant) | https://github.com/derf/feh/ |
| `xrandr` | X.Org / freedesktop.org | MIT License | https://www.x.org/ |

See `THIRD_PARTY_NOTICES.md` for the full attribution summary and additional notice text.

## Building

```sh
cargo build --release
# binary at target/release/deskctl
```

## Configuration

Generate a starter config automatically:

```sh
deskctl --genconfig                      # writes to $XDG_CONFIG_HOME/deskctl/config.toml
deskctl --genconfig=/custom/path.toml    # or write to a custom path
```

This always builds a `[displays.detected]` profile from a live `xrandr
--query` of *this machine's* current monitor layout (correct mode names and
rotations included), so it's safe to apply immediately after generating.

For wallpapers, optionally pass `--wallpaper-dir <dir>` alongside
`--genconfig`:

```sh
deskctl --genconfig --wallpaper-dir ~/Pictures/Wallpapers/horizontal
```

This builds two real, working wallpaper profiles ("set-1", "set-2") from the
image files actually found in that directory, keyed to your machine's
*current* connected screen count - each profile assigns one distinct image
per screen. Without `--wallpaper-dir`, `--genconfig` instead writes
placeholder wallpaper profiles (with `CHANGE_ME` paths) for you to edit by
hand.

(Fails if the target config file already exists, so it won't clobber a
config you've already customized.) Or copy `config.example.toml` yourself.
Either way, the resulting file has:

- `[displays.<name>]`: named `xrandr` argument lists, applied with
  `deskctl display apply <name>`
- `[[wallpapers.<screen-count>]]`: named wallpaper profiles, grouped by how
  many screens `xrandr` currently reports as connected. Each profile has an
  explicit, ordered `images` list - one path per screen, applied in that
  order (matching xrandr's output order). `deskctl wallpaper next` cycles
  through the profiles for your *current* screen count **in the order they
  appear in the config**, wrapping back to the first after the last -
  no randomness. It remembers its position in
  `$XDG_CACHE_HOME/deskctl/wallpaper-state-<count>` so repeated runs (e.g.
  from a systemd timer) keep advancing. `deskctl wallpaper apply <name>`
  jumps straight to a specific profile and also updates that saved position.
  An optional `notification_image` path can also be set per profile - it's
  passed to `notify-send -i` as the notification's icon; omit it for a
  plain text notification.

Image paths (and `--config`, `--wallpaper-dir`, `wallpaper set`) support a
leading `~` plus `$VAR` / `${VAR}` environment variables, e.g.
`images = ["$WALLPAPERS/nature/001.jpg"]`. Referencing an unset variable is an
error. Note that under a systemd user timer, only variables known to the
user manager are available (see `systemctl --user import-environment`).

## Usage

Running `deskctl` with no arguments is the same as `deskctl wallpaper next`.

```sh
deskctl wallpaper list             # show wallpaper profiles (and cycle order) for the current screen count
deskctl wallpaper next              # advance to the next profile in sequence and apply it
deskctl wallpaper apply <profile>  # jump straight to a specific named wallpaper profile
deskctl wallpaper set <path>        # set one literal image file on every screen
deskctl display list               # show connected monitors (xrandr --query)
deskctl display apply <profile>    # apply a named xrandr profile from config
```

## Running on a timer (systemd)

The easiest way to set this up is:

```sh
deskctl --enable
```

This writes the bundled `deskctl-wallpaper.service`/`.timer` units (embedded
in the binary at compile time, so this works even if you installed deskctl
via `cargo install` without a local checkout of this repo) to
`$XDG_CONFIG_HOME/systemd/user` (usually `~/.config/systemd/user`), then runs
`systemctl --user daemon-reload` and `systemctl --user enable --now
deskctl-wallpaper.timer` for you.

How often the timer fires is controlled by `[timer].interval` in your config
file (systemd time span syntax, e.g. "30min", "1h", "1h30min"). It defaults
to `"2h"` if omitted - both when `--genconfig` writes a new config file and
when `--enable` reads an existing one that predates this setting. Re-run
`deskctl --enable` any time you change `interval` to update the installed
timer.

Alternatively, unit files are provided in `systemd/` if you'd rather install
them yourself:

```sh
mkdir -p ~/.config/systemd/user
cp systemd/deskctl-wallpaper.service systemd/deskctl-wallpaper.timer ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now deskctl-wallpaper.timer
```

Check status/logs with:

```sh
systemctl --user status deskctl-wallpaper.timer
journalctl --user -u deskctl-wallpaper.service
```
