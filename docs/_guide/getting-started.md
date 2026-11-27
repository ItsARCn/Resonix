---
title: Getting Started
description: Install runtime tools, build or launch Resonix, and start listening on your desktop.
nav_order: 2
---

## Runtime dependencies

Resonix streams and downloads audio from YouTube and YouTube Music using two standard tools available on `PATH`:

- **[yt-dlp](https://github.com/yt-dlp/yt-dlp)** — resolves search results, streams, metadata, and audio downloads
- **[ffmpeg](https://ffmpeg.org)** — handles audio extraction and container conversion

## Launch Resonix

If building from source:

```sh
cargo run
```

To build without the MilkDrop C++ (`libprojectM`) visualizer:

```sh
cargo run --no-default-features
```

## Basics

- **No Spotify account needed.** Open Resonix, search any song, album, artist, or playlist, and double-click to play immediately.
- **Closing the window keeps playing in the tray** when enabled in Settings; reopen from the tray icon or quit via **Ctrl+Q** / tray menu.
- **Custom title bar** is enabled by default on Linux and Windows. You can switch between the custom bar and your desktop's native frame in **Settings → Appearance**.
- **Keyboard shortcuts:** `Space` plays/pauses, `Ctrl+F` or `/` searches, `Q` toggles the queue panel, `Ctrl+M` switches to the Winamp / WMP mini player, and `Ctrl+/` opens the shortcuts sheet.
- **Discord Rich Presence:** Enabled by default under **Settings → Playback on this computer**, showing your active track, artwork, progress timer, **Listen on YouTube**, and **Get Resonix** links over local Discord / Vesktop IPC.
- **Upstream credits:** Built on [Spotifast](https://github.com/crmne/spotifast) (`egui` + `fastframe` UI) and [Sunder](https://github.com/FrogSnot/Sunder) (YouTube backend).
