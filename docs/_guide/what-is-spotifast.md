---
title: What is Resonix?
description: A native Rust desktop YouTube Music client built on Spotifast's egui interface and Sunder's YouTube backend.
nav_order: 1
permalink: /what-is-spotifast/
---

Resonix is a fast, lightweight native music player for Linux, macOS, and Windows written in Rust — with no browser engine and no Electron.

## Two open-source foundations

- **[Spotifast](https://github.com/crmne/spotifast)** (Carmine Paolino, MIT) provides the pure `egui` + `fastframe` UI, Winamp `.wsz` and WMP `.wmz` skin engine, MilkDrop `libprojectM` visualizer, 10-band equalizer, LRCLIB lyrics view, custom theme / Omarchy support, and desktop tray/MPRIS integration.
- **[Sunder](https://github.com/FrogSnot/Sunder)** (AGPL-3.0) provides the YouTube & YouTube Music dual-source search, `yt-dlp` stream extraction, offline download manager, and local SQLite persistence.

## Why Resonix?

- **Fast & low memory:** Native GPU rendering via `eframe` / `glow` instead of a Chromium web view.
- **No external playback account gates:** Search and listen immediately using YouTube / YouTube Music.
- **Rich desktop extras:** Discord Rich Presence, classic Winamp/WMP mini player (`Resonix Tall` & `Resonix Wide`), MilkDrop visuals, and CLI control (`resonix --help`).
