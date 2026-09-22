# Third-Party Notices

This project is licensed under the MIT License. The software below is used by or invoked by `deskctl` during normal operation, and its licenses are reproduced here for attribution and compliance.

## Rust dependencies

The project directly depends on the following crates:

| Component | Author / Organization | License | Link |
| --- | --- | --- | --- |
| `anyhow` | David Tolnay | Apache-2.0 OR MIT | https://github.com/dtolnay/anyhow |
| `clap` | clap-rs contributors | Apache-2.0 OR MIT | https://github.com/clap-rs/clap |
| `env_logger` | rust-cli contributors | Apache-2.0 OR MIT | https://github.com/rust-cli/env_logger |
| `log` | The Rust Project Developers | Apache-2.0 OR MIT | https://github.com/rust-lang/log |
| `serde` | serde-rs contributors | Apache-2.0 OR MIT | https://github.com/serde-rs/serde |
| `toml` | toml-rs contributors | Apache-2.0 OR MIT | https://github.com/toml-rs/toml |

These crates are permissive open-source libraries and are compatible with the project's MIT license.

## External runtime dependencies

`deskctl` shells out to the following external programs:

| Component | Author / Organization | License | Link |
| --- | --- | --- | --- |
| `feh` | Tomáš Hubínek ("derf") | MIT-feh (MIT variant) | https://github.com/derf/feh/ |
| `xrandr` | X.Org / freedesktop.org | MIT License | https://www.x.org/ |

These tools are Free and Open Source Software and are distributed under permissive licenses compatible with this project's MIT licensing.

## License notices

### MIT License

The MIT License applies to the `deskctl` project itself. See `LICENSE` for the full text.

### MIT-feh

`feh` is distributed under an MIT-style license. The complete license text is available in the upstream project sources and is included in the original distribution. Preserve the author attribution and copyright notice when redistributing feh or derivative works.

### X.Org / xrandr MIT

`xrandr` is distributed under the X.Org / freedesktop.org project licensing terms, which are MIT-compatible. Preserve the copyright and license notices when redistributing this software or derived works.
