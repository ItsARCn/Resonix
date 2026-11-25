use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

use super::error::AppError;
use super::models::YtTrack;
use super::process::NoWindow;

const YTDLP_QUERY_TIMEOUT: Duration = Duration::from_secs(15);
const YTDLP_PLAYLIST_TIMEOUT: Duration = Duration::from_secs(30);

async fn run_ytdlp_with_timeout(
    mut cmd: Command,
    timeout: Duration,
) -> Result<std::process::Output, AppError> {
    tokio::time::timeout(timeout, cmd.output())
        .await
        .map_err(|_| AppError::Extraction("yt-dlp timed out".into()))?
        .map_err(|e| AppError::Extraction(format!("failed to run yt-dlp: {e}")))
}

pub struct Extractor {
    /// App-managed yt-dlp location. Resolution happens per spawn so a
    /// runtime update takes effect without restart.
    managed: PathBuf,
}

impl Extractor {
    pub fn new(data_dir: &std::path::Path) -> Self {
        Self {
            managed: super::ytdlp::managed_bin_path(data_dir),
        }
    }

    pub fn bin(&self) -> String {
        super::ytdlp::resolve_bin(Some(&self.managed))
    }

    /// Search YouTube Music specifically for tracks.
    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<YtTrack>, AppError> {
        let mut cmd = Command::new(self.bin());
        cmd.no_window()
            .args([
                &format!("https://music.youtube.com/search?q={}", encode_query(query)),
                "--dump-json",
                "--flat-playlist",
                "--playlist-end",
                &limit.to_string(),
                "--no-warnings",
                "--ignore-errors",
            ])
            .args(super::cookies::current_args())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let output = run_ytdlp_with_timeout(cmd, YTDLP_QUERY_TIMEOUT).await?;

        if !output.status.success() {
            return Err(AppError::Extraction("yt-dlp search failed".into()));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(parse_flat_tracks(&stdout))
    }

    /// Search generic YouTube (carries channel/uploader/duration in flat mode).
    pub async fn search_youtube(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<YtTrack>, AppError> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let mut cmd = Command::new(self.bin());
        cmd.no_window()
            .args([
                &format!("ytsearch{limit}:{query}"),
                "--dump-json",
                "--flat-playlist",
                "--no-warnings",
                "--ignore-errors",
            ])
            .args(super::cookies::current_args())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let output = run_ytdlp_with_timeout(cmd, YTDLP_QUERY_TIMEOUT).await?;

        if !output.status.success() {
            return Err(AppError::Extraction("yt-dlp search failed".into()));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(parse_flat_tracks(&stdout))
    }

    /// Fetch metadata for a single video/track.
    pub async fn metadata(&self, video_id: &str) -> Result<YtTrack, AppError> {
        let mut cmd = Command::new(self.bin());
        cmd.no_window()
            .args([
                &format!("https://www.youtube.com/watch?v={video_id}"),
                "-j",
                "--no-playlist",
                "--no-warnings",
            ])
            .args(super::cookies::current_args())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let output = run_ytdlp_with_timeout(cmd, YTDLP_QUERY_TIMEOUT).await?;

        let v: serde_json::Value = serde_json::from_slice(&output.stdout)
            .map_err(|e| AppError::Extraction(e.to_string()))?;

        Ok(YtTrack {
            id: v["id"].as_str().unwrap_or(video_id).to_string(),
            title: v["title"].as_str().unwrap_or("Unknown").to_string(),
            artist: v["channel"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .or_else(|| v["uploader"].as_str().filter(|s| !s.trim().is_empty()))
                .unwrap_or("Unknown")
                .to_string(),
            thumbnail: best_thumbnail(&v),
            duration_secs: v["duration"].as_f64().unwrap_or(0.0),
            stream_url: None,
        })
    }

    pub async fn get_subtitles(&self, video_id: &str, lang: &str) -> Result<String, AppError> {
        let tmp = std::env::temp_dir();
        let output = Command::new(self.bin())
            .no_window()
            .args([
                &format!("https://www.youtube.com/watch?v={video_id}"),
                "--skip-download",
                "--write-subs",
                "--sub-langs",
                lang,
                "--sub-format",
                "vtt",
                "-o",
                &tmp.join(video_id).to_string_lossy(),
                "--no-warnings",
            ])
            .args(super::cookies::current_args())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| AppError::Extraction(format!("failed to run yt-dlp: {e}")))?;

        if !output.status.success() {
            return Err(AppError::Extraction("failed to fetch subtitles".into()));
        }

        let vtt_path = tmp.join(format!("{video_id}.{lang}.vtt"));
        if !vtt_path.exists() {
            return Err(AppError::Extraction(format!("no {lang} subtitles found")));
        }

        let content = std::fs::read_to_string(&vtt_path)
            .map_err(|e| AppError::Extraction(format!("failed to read subtitles: {e}")))?;
        let _ = std::fs::remove_file(&vtt_path);

        use std::sync::LazyLock;
        static RE_TAGS: LazyLock<regex_lite::Regex> =
            LazyLock::new(|| regex_lite::Regex::new(r"<[^>]+>").expect("valid tag regex"));
        let lyrics = content
            .lines()
            .filter(|l| {
                let l = l.trim();
                !l.is_empty()
                    && !l.starts_with("WEBVTT")
                    && !l.starts_with("Kind:")
                    && !l.starts_with("Language:")
                    && !l.starts_with("NOTE")
                    && !l.contains(" --> ")
                    && l.parse::<u32>().is_err()
            })
            .map(|l| RE_TAGS.replace_all(l, "").to_string())
            .collect::<Vec<_>>()
            .join("\n");

        Ok(lyrics)
    }

    pub async fn extract_playlist(
        &self,
        url: &str,
    ) -> Result<(String, Option<String>, Vec<YtTrack>), AppError> {
        let mut cmd = Command::new(self.bin());
        cmd.no_window()
            .args([
                url,
                "--dump-json",
                "--flat-playlist",
                "--no-warnings",
                "--ignore-errors",
            ])
            .args(super::cookies::current_args())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let output = run_ytdlp_with_timeout(cmd, YTDLP_PLAYLIST_TIMEOUT).await?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut tracks = Vec::new();
        let mut playlist_title = "Imported Playlist".to_string();
        let mut playlist_thumbnail = None;

        for line in stdout.lines() {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                if playlist_title == "Imported Playlist" {
                    if let Some(t) = v["playlist_title"].as_str() {
                        playlist_title = t.to_string();
                    } else if let Some(t) = v["playlist"].as_str() {
                        playlist_title = t.to_string();
                    }
                }
                let thumb = best_thumbnail(&v);
                if playlist_thumbnail.is_none() && !thumb.is_empty() {
                    playlist_thumbnail = Some(thumb.clone());
                }
                if let Some(id) = v["id"].as_str() {
                    tracks.push(YtTrack {
                        id: id.to_string(),
                        title: v["title"].as_str().unwrap_or("Unknown").to_string(),
                        artist: v["channel"]
                            .as_str()
                            .or_else(|| v["uploader"].as_str())
                            .unwrap_or("Unknown")
                            .to_string(),
                        thumbnail: thumb,
                        duration_secs: v["duration"].as_f64().unwrap_or(0.0),
                        stream_url: None,
                    });
                }
            }
        }

        Ok((playlist_title, playlist_thumbnail, tracks))
    }
}

fn parse_flat_tracks(stdout: &str) -> Vec<YtTrack> {
    stdout
        .lines()
        .filter_map(|line| {
            let v: serde_json::Value = serde_json::from_str(line).ok()?;
            if !is_playable_video(&v) {
                return None;
            }
            let title = v["title"].as_str().unwrap_or("Unknown").to_string();
            let raw_artist = v["channel"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .or_else(|| v["uploader"].as_str().filter(|s| !s.trim().is_empty()))
                .or_else(|| {
                    v["playlist_uploader"]
                        .as_str()
                        .filter(|s| !s.trim().is_empty())
                });
            let artist = raw_artist
                .map(str::to_string)
                .unwrap_or_else(|| "Unknown".to_string());
            Some(YtTrack {
                id: v["id"].as_str()?.to_string(),
                title,
                artist,
                thumbnail: best_thumbnail(&v),
                duration_secs: v["duration"].as_f64().unwrap_or(0.0),
                stream_url: None,
            })
        })
        .collect()
}

fn is_playable_video(v: &serde_json::Value) -> bool {
    let url_ok = v["url"].as_str().is_some_and(|u| u.contains("/watch"));
    let title_ok = v["title"].as_str().is_some_and(|t| !t.trim().is_empty());
    url_ok && title_ok
}

fn best_thumbnail(v: &serde_json::Value) -> String {
    if let Some(thumbs) = v["thumbnails"].as_array() {
        let target = thumbs.iter().find(|t| {
            t["width"]
                .as_u64()
                .is_some_and(|w| (280..=400).contains(&w))
        });
        let chosen = target
            .or_else(|| thumbs.get(thumbs.len().min(2).min(thumbs.len().saturating_sub(1))))
            .or_else(|| thumbs.first());
        if let Some(t) = chosen
            && let Some(url) = t["url"].as_str()
        {
            return url.to_string();
        }
    }
    let base = v["thumbnail"].as_str().unwrap_or_default();
    if !base.is_empty() && base.contains("i.ytimg.com") && !base.contains("?sqp=") {
        return base
            .replace("maxresdefault", "mqdefault")
            .replace("hqdefault", "mqdefault");
    }
    base.to_string()
}

fn encode_query(q: &str) -> String {
    let mut out = String::with_capacity(q.len());
    for b in q.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
