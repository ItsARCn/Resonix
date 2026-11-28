# Copilot instructions for Resonix

Use `CONTRIBUTING.md` as the source of truth for every change and review.

## Scope & Review

- Keep Resonix a fast, native YouTube Music client written in Rust (`egui` + `fastframe`), without an embedded browser engine or telemetry.
- Require `yt-dlp` and `ffmpeg` only at runtime via external process execution (never bundled inside small installers unless explicitly approved).
- Check that behavioural changes include focused tests and that user-visible settings, files, or network access are documented.
- Dependencies must come from crates.io or approved forks in `Cargo.toml`.
- Keep review evidence (screenshots, recordings, comparisons) attached to pull requests rather than committed into the repository.

## Communication

- Keep issue and pull request replies concise, direct, and in plain language.
- Never include credentials or signing keys in commits, logs, or replies.
