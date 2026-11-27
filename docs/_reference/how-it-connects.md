---
title: How Resonix Connects
description: Network endpoints, external tools, and local IPC channels Resonix uses.
---

Resonix contains no telemetry and runs no hosted backend. All network traffic goes directly from your computer to the services below:

| Purpose | Destination / Transport | Notes |
|---|---|---|
| **YouTube & YouTube Music search / audio** | `yt-dlp` + YouTube endpoints | Fetches track metadata, search results, streams, and downloads when you browse or play. |
| **Artwork thumbnails** | `i.ytimg.com` / YouTube thumbnail URLs | Cached locally in Resonix's art cache directory (`~/.cache/resonix/art` on Linux). |
| **Lyrics** | `https://lrclib.net/api` | Plain and synced LRC lyrics lookups, cached in `~/.cache/resonix/lyrics`. |
| **Discord Rich Presence** | Local IPC (`discord-ipc-*` / `vesktop-ipc-*` / Windows pipe) | Sends current track title, artist, album/channel, timestamps, artwork URL, and YouTube / GitHub buttons only when **Discord Rich Presence** is enabled in Settings. |
| **Update checks** | GitHub Releases (`ItsARCn/Resonix`) | Optional release checks via `fastframe-update`, verified against the embedded Ed25519 public key. |
| **Single-instance CLI control** | Local per-user socket / loopback channel | Forwards CLI commands (`resonix play-pause`, `resonix next`, etc.) to the running instance. |

All requests respect your configured HTTP / SOCKS5 / System proxy in **Settings → Proxy** (where supported by `reqwest` and `yt-dlp`).
