//! Discord Rich Presence client over local IPC (adapted from Sunder).
//!
//! Connects to Discord / Vesktop over Unix sockets (Linux & macOS) or named
//! pipes (Windows) without blocking the UI thread, advertising `Listening`
//! activity (`type: 2`) with live timestamps, artwork, and `Listen on
//! YouTube` / `Get Resonix` link buttons.

use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DEFAULT_APPLICATION_ID: &str = "1575919329109614783";
const RESONIX_REPO_URL: &str = "https://github.com/ItsARCn/Resonix";
const FALLBACK_ICON_URL: &str =
    "https://raw.githubusercontent.com/ItsARCn/Resonix/main/packaging/macos/icon-1024.png";
const MAX_FIELD_CHARS: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresenceTrack {
    pub uri: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub thumbnail: String,
    pub youtube_url: Option<String>,
    pub playing: bool,
    pub position_ms: u32,
    pub duration_ms: u32,
}

enum PresenceCommand {
    SetEnabled(bool),
    Update(Option<PresenceTrack>),
    Shutdown,
}

pub struct DiscordPresence {
    tx: mpsc::Sender<PresenceCommand>,
}

impl Default for DiscordPresence {
    fn default() -> Self {
        Self::new()
    }
}

impl DiscordPresence {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let app_id = std::env::var("RESONIX_DISCORD_APP_ID")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| DEFAULT_APPLICATION_ID.to_string());
        std::thread::Builder::new()
            .name("discord-rpc".into())
            .spawn(move || run_loop(rx, &app_id))
            .ok();
        Self { tx }
    }

    pub fn set_enabled(&self, enabled: bool) {
        let _ = self.tx.send(PresenceCommand::SetEnabled(enabled));
    }

    pub fn update(&self, track: Option<PresenceTrack>) {
        let _ = self.tx.send(PresenceCommand::Update(track));
    }

    pub fn clear(&self) {
        let _ = self.tx.send(PresenceCommand::Update(None));
    }

    pub fn shutdown(&self) {
        let _ = self.tx.send(PresenceCommand::Shutdown);
    }
}

#[cfg(unix)]
type IpcStream = std::os::unix::net::UnixStream;
#[cfg(windows)]
type IpcStream = std::fs::File;

#[cfg(any(unix, windows))]
fn run_loop(rx: mpsc::Receiver<PresenceCommand>, app_id: &str) {
    let mut stream: Option<IpcStream> = None;
    let mut enabled = false;
    let mut current: Option<PresenceTrack> = None;
    let mut nonce: u64 = 0;

    while let Ok(cmd) = rx.recv() {
        match cmd {
            PresenceCommand::Shutdown => {
                if let Some(ref mut s) = stream {
                    nonce += 1;
                    let _ = send_clear(s, nonce);
                }
                break;
            }
            PresenceCommand::SetEnabled(next) => {
                if enabled == next {
                    continue;
                }
                enabled = next;
                if !enabled {
                    if let Some(ref mut s) = stream {
                        nonce += 1;
                        let _ = send_clear(s, nonce);
                    }
                    stream = None;
                    continue;
                }
                if current.is_none() {
                    continue;
                }
            }
            PresenceCommand::Update(track) => {
                current = track;
                if !enabled {
                    continue;
                }
            }
        }

        if !enabled {
            continue;
        }

        if stream.is_none() {
            stream = connect_and_handshake(app_id);
        }
        let Some(ref mut s) = stream else {
            continue;
        };

        let ok = match &current {
            Some(track) => {
                let now_sec = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                nonce += 1;
                let with_buttons = build_activity_json(track, now_sec, nonce, true);
                match send_and_check(s, &with_buttons) {
                    Ok(true) => Ok(()),
                    Ok(false) if track.youtube_url.is_some() => {
                        // Retry without custom buttons in case the connected client rejects them.
                        nonce += 1;
                        let plain = build_activity_json(track, now_sec, nonce, false);
                        send_and_check(s, &plain).map(|_| ())
                    }
                    Ok(false) => Ok(()),
                    Err(e) => Err(e),
                }
            }
            None => {
                nonce += 1;
                send_clear(s, nonce)
            }
        };

        if let Err(err) = ok {
            log::debug!("discord rpc write failed: {err}");
            stream = None;
        }
    }
}

#[cfg(any(unix, windows))]
fn connect_and_handshake(app_id: &str) -> Option<IpcStream> {
    let mut s = connect()?;
    let handshake = format!(r#"{{"v":1,"client_id":"{}"}}"#, esc(app_id));
    if let Err(err) = write_frame(&mut s, 0, &handshake) {
        log::debug!("discord rpc handshake write failed: {err}");
        return None;
    }
    match read_frame_body(&mut s) {
        Ok(body) if !body.contains("\"ERROR\"") => {
            log::debug!("discord rpc connected");
            Some(s)
        }
        Ok(body) => {
            log::debug!("discord rpc handshake rejected: {body}");
            None
        }
        Err(err) => {
            log::debug!("discord rpc handshake read failed: {err}");
            None
        }
    }
}

#[cfg(any(unix, windows))]
fn send_clear(s: &mut IpcStream, nonce: u64) -> std::io::Result<()> {
    let pid = std::process::id();
    let payload = format!(
        r#"{{"cmd":"SET_ACTIVITY","args":{{"pid":{pid},"activity":null}},"nonce":"{nonce}"}}"#
    );
    send_and_check(s, &payload).map(|_| ())
}

#[cfg(any(unix, windows))]
fn send_and_check(s: &mut IpcStream, payload: &str) -> std::io::Result<bool> {
    write_frame(s, 1, payload)?;
    match read_frame_body(s) {
        Ok(body) if body.contains("\"ERROR\"") => {
            log::debug!("discord rpc activity rejected: {body}");
            Ok(false)
        }
        Ok(_) => Ok(true),
        Err(err) => Err(err),
    }
}

#[cfg(unix)]
fn connect() -> Option<IpcStream> {
    const PREFIXES: &[&str] = &["discord-ipc-", "vesktop-ipc-"];

    let base_dirs: Vec<String> = [
        std::env::var("XDG_RUNTIME_DIR").ok(),
        std::env::var("TMPDIR").ok(),
        Some("/tmp".into()),
    ]
    .into_iter()
    .flatten()
    .filter(|d| !d.trim().is_empty())
    .collect();

    let mut dirs = base_dirs.clone();
    if let Some(xdg) = base_dirs.first() {
        dirs.push(format!("{xdg}/app/com.discordapp.Discord"));
        dirs.push(format!("{xdg}/snap.discord"));
        dirs.push(format!("{xdg}/app/dev.vencord.vesktop"));
        dirs.push(format!("{xdg}/snap.vesktop"));
    }

    for dir in &dirs {
        for prefix in PREFIXES {
            for i in 0..10 {
                let path = format!("{dir}/{prefix}{i}");
                if let Ok(s) = IpcStream::connect(&path) {
                    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
                    let _ = s.set_write_timeout(Some(Duration::from_secs(5)));
                    return Some(s);
                }
            }
        }
    }
    None
}

#[cfg(windows)]
fn connect() -> Option<IpcStream> {
    for i in 0..10 {
        let path = format!(r"\\.\pipe\discord-ipc-{i}");
        if let Ok(f) = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
        {
            return Some(f);
        }
    }
    None
}

#[cfg(any(unix, windows))]
fn write_frame(s: &mut IpcStream, op: u32, payload: &str) -> std::io::Result<()> {
    use std::io::Write;
    let bytes = payload.as_bytes();
    let mut hdr = [0u8; 8];
    hdr[..4].copy_from_slice(&op.to_le_bytes());
    hdr[4..].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
    s.write_all(&hdr)?;
    s.write_all(bytes)?;
    s.flush()
}

#[cfg(any(unix, windows))]
fn read_frame_body(s: &mut IpcStream) -> std::io::Result<String> {
    use std::io::Read;
    let mut hdr = [0u8; 8];
    s.read_exact(&mut hdr)?;
    let len = u32::from_le_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]) as usize;
    if len > 1_048_576 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "oversized discord rpc frame",
        ));
    }
    let mut body = vec![0u8; len];
    s.read_exact(&mut body)?;
    Ok(String::from_utf8_lossy(&body).into_owned())
}

#[cfg(not(any(unix, windows)))]
fn run_loop(rx: mpsc::Receiver<PresenceCommand>, _app_id: &str) {
    while let Ok(cmd) = rx.recv() {
        if matches!(cmd, PresenceCommand::Shutdown) {
            break;
        }
    }
}

fn clamp_field(s: &str) -> String {
    let trimmed = s.trim();
    let mut out: String = trimmed.chars().take(MAX_FIELD_CHARS).collect();
    if out.chars().count() == 1 {
        out.push(' ');
    }
    out
}

fn build_activity_json(
    track: &PresenceTrack,
    now_sec: u64,
    nonce: u64,
    include_buttons: bool,
) -> String {
    let pid = std::process::id();
    let details_raw = if track.title.trim().is_empty() {
        "Unknown".to_string()
    } else {
        clamp_field(&track.title)
    };
    let state_raw = if !track.playing {
        "Paused".to_string()
    } else if track.artist.trim().is_empty() {
        "Listening on Resonix".to_string()
    } else {
        clamp_field(&format!("by {}", track.artist.trim()))
    };

    let timestamps = if track.playing {
        let pos_sec = u64::from(track.position_ms / 1000);
        let start = now_sec.saturating_sub(pos_sec);
        if track.duration_ms > 0 {
            let dur_sec = u64::from((track.duration_ms / 1000).max(1));
            let end = start + dur_sec;
            format!(r#","timestamps":{{"start":{start},"end":{end}}}"#)
        } else {
            format!(r#","timestamps":{{"start":{start}}}"#)
        }
    } else {
        String::new()
    };

    let image_url =
        if track.thumbnail.starts_with("https://") || track.thumbnail.starts_with("http://") {
            track.thumbnail.as_str()
        } else {
            FALLBACK_ICON_URL
        };
    let large_text = if track.album.trim().is_empty() {
        "Resonix".to_string()
    } else {
        clamp_field(&track.album)
    };
    let assets = format!(
        r#","assets":{{"large_image":"{}","large_text":"{}"}}"#,
        esc(image_url),
        esc(&large_text)
    );

    let buttons = if include_buttons
        && let Some(url) = track
            .youtube_url
            .as_deref()
            .filter(|u| u.starts_with("https://") || u.starts_with("http://"))
    {
        format!(
            r#","buttons":[{{"label":"Listen on YouTube","url":"{}"}},{{"label":"Get Resonix","url":"{}"}}]"#,
            esc(url),
            esc(RESONIX_REPO_URL),
        )
    } else {
        String::new()
    };

    format!(
        r#"{{"cmd":"SET_ACTIVITY","args":{{"pid":{pid},"activity":{{"type":2,"details":"{}","state":"{}"{timestamps}{assets}{buttons}}}}},"nonce":"{nonce}"}}"#,
        esc(&details_raw),
        esc(&state_raw),
    )
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_json_includes_youtube_button_and_timestamps() {
        let track = PresenceTrack {
            uri: "spotify:track:dQw4w9WgXcQ".into(),
            title: "Never Gonna Give You Up".into(),
            artist: "Rick Astley".into(),
            album: "Whenever You Need Somebody".into(),
            thumbnail: "https://i.ytimg.com/vi/dQw4w9WgXcQ/mqdefault.jpg".into(),
            youtube_url: Some("https://www.youtube.com/watch?v=dQw4w9WgXcQ".into()),
            playing: true,
            position_ms: 30_000,
            duration_ms: 213_000,
        };
        let json = build_activity_json(&track, 1_700_000_030, 1, true);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let activity = &parsed["args"]["activity"];
        assert_eq!(activity["type"], 2);
        assert_eq!(activity["details"], "Never Gonna Give You Up");
        assert_eq!(activity["state"], "by Rick Astley");
        assert_eq!(activity["timestamps"]["start"], 1_700_000_000);
        assert_eq!(activity["timestamps"]["end"], 1_700_000_213);
        assert_eq!(
            activity["assets"]["large_image"],
            "https://i.ytimg.com/vi/dQw4w9WgXcQ/mqdefault.jpg"
        );
        assert_eq!(
            activity["buttons"][0]["url"],
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ"
        );
        assert_eq!(activity["buttons"][0]["label"], "Listen on YouTube");
        assert_eq!(activity["buttons"][1]["url"], RESONIX_REPO_URL);
        assert_eq!(activity["buttons"][1]["label"], "Get Resonix");
    }

    #[test]
    fn activity_json_paused_omits_timestamps_and_uses_fallback_icon() {
        let track = PresenceTrack {
            uri: "spotify:track:abc".into(),
            title: "A".into(),
            artist: "One \"Quote\" Artist".into(),
            album: String::new(),
            thumbnail: "localart://track".into(),
            youtube_url: None,
            playing: false,
            position_ms: 10_000,
            duration_ms: 100_000,
        };
        let json = build_activity_json(&track, 1_700_000_000, 2, true);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let activity = &parsed["args"]["activity"];
        assert_eq!(activity["details"], "A ");
        assert_eq!(activity["state"], "Paused");
        assert!(activity["timestamps"].is_null());
        assert_eq!(activity["assets"]["large_image"], FALLBACK_ICON_URL);
        assert_eq!(activity["assets"]["large_text"], "Resonix");
        assert!(activity["buttons"].is_null());
    }
}
