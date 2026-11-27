# Contributing to Resonix

Resonix is a native desktop YouTube Music client written in Rust with `egui` and
`fastframe`, combining Spotifast's native desktop interface with Sunder's YouTube
audio and library backend. Changes should improve the native app without adding
a browser engine, telemetry, or a hosted backend.

## Before opening an issue

Search open and closed issues first. For a bug, include logs
(`resonix.log` from your state directory) and clear steps to reproduce it.

For a feature, explain the user problem and discuss large changes in an issue
before writing code.

## Project guidelines

- **Native-first:** Do not embed a browser engine/WebView or Electron shell.
- **Privacy:** Do not add telemetry, analytics, or credentials logging.
- **No regressions:** Run `cargo fmt --all -- --check`, `cargo check --all-targets`,
  and the relevant unit/integration tests before submitting changes.
- **Upstream credits:** Resonix builds on [Spotifast](https://github.com/crmne/spotifast)
  (MIT) and [Sunder](https://github.com/FrogSnot/Sunder) (AGPL-3.0). Keep
  upstream attributions and license notices intact.

## Development quick start

```sh
cargo run
cargo run --features demo -- --demo
cargo fmt --all -- --check
cargo check --all-targets
cargo test --features demo --test branding --test localization --test omarchy --test packaging_icons --test startup_diagnostics --test tray --test update_launch
```

Translations live in `assets/i18n/`; see
[Translating Resonix](docs/_reference/translating.md). Release packaging notes
are in [PACKAGING.md](PACKAGING.md).
