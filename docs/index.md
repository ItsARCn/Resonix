---
layout: home
title: Resonix
description: Resonix is a fast, lightweight YouTube Music client for Linux, macOS, and Windows, written in Rust with no browser engine.
seo:
  title: Native YouTube Music Client for Linux, macOS and Windows
permalink: /
hero:
  name: Resonix
  text: YouTube Music, native and fast
  tagline: A lightweight native music player in Rust for Linux, macOS, and Windows. Powered by Spotifast's egui UI and Sunder's YouTube backend.
  actions:
    - theme: brand
      text: Releases
      link: https://github.com/ItsARCn/Resonix/releases
    - theme: alt
      text: Getting Started
      link: /getting-started/
    - theme: alt
      text: GitHub
      link: https://github.com/ItsARCn/Resonix
  image:
    src: /screenshot.png
    alt: "Resonix Home with library, queue, and music player visible"
    width: 2018
    height: 1198

features:
  - icon: ⚡
    title: Lightweight
    details: Pure native Rust desktop UI with no Electron or embedded browser engine.
  - icon: 🎵
    title: YouTube & YouTube Music
    details: Search YouTube Music and YouTube simultaneously, stream locally via yt-dlp and ffmpeg/rodio, and save tracks offline.
  - icon: 💬
    title: Discord Rich Presence
    details: Share your playing track, artist, cover art, progress bar, and Listen on YouTube / Get Resonix buttons via local Discord IPC.
  - icon: 🎨
    title: Themes
    details: Light, dark, follow-system, custom palettes, album-art accents, and Omarchy desktop theme support.
  - icon: 📻
    title: Skins
    details: Wear Resonix Tall and Resonix Wide, or load classic Winamp (.wsz) and Windows Media Player (.wmz) skins.
    link: /winamp/
    link_text: See it in action
  - icon: 🌀
    title: MilkDrop
    details: Watch music-reactive visuals through libprojectM in a dedicated window or fullscreen.
    link: /milkdrop/
    link_text: Open the guide
  - icon: ⌨️
    title: Desktop controls
    details: Keyboard shortcuts, CLI remote control, media keys, and background system tray playback.
  - icon: 🤝
    title: Credits
    details: Built by combining Carmine Paolino's Spotifast UI with FrogSnot's Sunder YouTube backend.
    link: https://github.com/ItsARCn/Resonix
    link_text: View on GitHub
---

## Built on two great open-source projects

Resonix brings together:

* **[Spotifast](https://github.com/crmne/spotifast)** by **Carmine Paolino** (MIT) for the native `egui` + `fastframe` UI, skin engine, MilkDrop host, 10-band EQ, lyrics, themes, and desktop integration.
* **[Sunder](https://github.com/FrogSnot/Sunder)** (AGPL-3.0) for dual-source YouTube / YouTube Music search, `yt-dlp` extraction, local playback, and SQLite track storage.

## Skins of its own, and the classics

Resonix ships **Resonix Tall** and **Resonix Wide**, plus full support for classic Winamp `.wsz` skins and Windows Media Player `.wmz` skins with their playlists, equalizers, and MilkDrop panels. [See the mini player in detail](/winamp/).
