use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::Semaphore;

use super::db::SearchCache;
use super::models::YtTrack;
use super::process::NoWindow;

const MAX_CONCURRENT: usize = 3;

#[derive(Debug, Clone)]
pub struct DownloadProgress {
    pub track_id: String,
    pub status: String,
    pub percent: f64,
}

pub struct DownloadManager {
    dir: PathBuf,
    managed_ytdlp: PathBuf,
    sem: Arc<Semaphore>,
    active: Arc<Mutex<HashSet<String>>>,
}

impl DownloadManager {
    pub fn new(state_dir: &Path) -> Self {
        let dir = state_dir.join("downloads");
        let _ = std::fs::create_dir_all(&dir);
        Self {
            dir,
            managed_ytdlp: super::ytdlp::managed_bin_path(state_dir),
            sem: Arc::new(Semaphore::new(MAX_CONCURRENT)),
            active: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub fn path_for(&self, track_id: &str) -> PathBuf {
        self.dir.join(format!("{track_id}.mp3"))
    }

    pub fn is_active(&self, track_id: &str) -> bool {
        self.active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(track_id)
    }

    fn try_begin(&self, track_id: &str) -> bool {
        self.active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(track_id.to_string())
    }

    fn finish(&self, track_id: &str) {
        self.active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(track_id);
    }

    pub async fn download(
        &self,
        db: &SearchCache,
        track: &YtTrack,
        on_progress: impl FnMut(DownloadProgress),
    ) -> Result<bool, String> {
        self.download_with_mode(db, track, true, on_progress).await
    }

    pub async fn download_with_mode(
        &self,
        db: &SearchCache,
        track: &YtTrack,
        standalone: bool,
        mut on_progress: impl FnMut(DownloadProgress),
    ) -> Result<bool, String> {
        let track_id = track.id.clone();
        let path = self.path_for(&track_id);

        if path.exists() {
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            let _ = db.upsert_tracks(std::slice::from_ref(track));
            let _ =
                db.mark_downloaded_with_mode(&track_id, &path.to_string_lossy(), size, standalone);
            on_progress(DownloadProgress {
                track_id,
                status: "done".into(),
                percent: 100.0,
            });
            return Ok(true);
        }

        if !self.try_begin(&track_id) {
            return Ok(false);
        }

        let _ = db.upsert_tracks(std::slice::from_ref(track));
        on_progress(DownloadProgress {
            track_id: track_id.clone(),
            status: "queued".into(),
            percent: 0.0,
        });

        let permit = self.sem.clone().acquire_owned().await;
        let bin = super::ytdlp::resolve_bin(Some(&self.managed_ytdlp));
        let result = run_ytdlp(&bin, &self.dir, &track_id, &mut on_progress).await;
        drop(permit);

        self.finish(&track_id);

        match result {
            Ok(final_path) => {
                let size = std::fs::metadata(&final_path).map(|m| m.len()).unwrap_or(0);
                db.mark_downloaded_with_mode(
                    &track_id,
                    &final_path.to_string_lossy(),
                    size,
                    standalone,
                )
                .map_err(|e| e.to_string())?;
                on_progress(DownloadProgress {
                    track_id,
                    status: "done".into(),
                    percent: 100.0,
                });
                Ok(true)
            }
            Err(e) => {
                cleanup_partials(&self.dir, &track_id);
                on_progress(DownloadProgress {
                    track_id,
                    status: "error".into(),
                    percent: 0.0,
                });
                Err(e)
            }
        }
    }

    pub fn delete(&self, db: &SearchCache, track_id: &str) -> Result<(), String> {
        if let Some(path) = db
            .remove_standalone_download(track_id)
            .map_err(|e| e.to_string())?
        {
            let _ = std::fs::remove_file(path);
        }
        Ok(())
    }

    pub fn delete_collection(&self, db: &SearchCache, collection_uri: &str) -> Result<(), String> {
        let orphan_paths = db
            .delete_download_collection(collection_uri)
            .map_err(|e| e.to_string())?;
        for path in orphan_paths {
            let _ = std::fs::remove_file(path);
        }
        Ok(())
    }
}

async fn run_ytdlp(
    bin: &str,
    dir: &Path,
    track_id: &str,
    on_progress: &mut impl FnMut(DownloadProgress),
) -> Result<PathBuf, String> {
    let url = format!("https://www.youtube.com/watch?v={track_id}");
    let out_template = dir.join(format!("{track_id}.%(ext)s"));
    let expected_path = dir.join(format!("{track_id}.mp3"));

    let mut child = Command::new(bin)
        .no_window()
        .args([
            url.as_str(),
            "--extract-audio",
            "--audio-format",
            "mp3",
            "--audio-quality",
            "2",
            "-o",
            out_template.to_str().unwrap_or_default(),
            "--no-playlist",
            "--newline",
            "--concurrent-fragments",
            "4",
        ])
        .args(super::cookies::current_args())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn yt-dlp: {e}"))?;

    if let Some(stdout) = child.stdout.take() {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(pct) = parse_download_pct(&line) {
                on_progress(DownloadProgress {
                    track_id: track_id.to_string(),
                    status: "downloading".into(),
                    percent: pct,
                });
            } else if line.contains("[ExtractAudio]") {
                on_progress(DownloadProgress {
                    track_id: track_id.to_string(),
                    status: "converting".into(),
                    percent: 100.0,
                });
            }
        }
    }

    let status = child
        .wait()
        .await
        .map_err(|e| format!("yt-dlp wait: {e}"))?;
    if status.success() && expected_path.exists() {
        return Ok(expected_path);
    }

    let stderr = match child.stderr.take() {
        Some(mut s) => {
            use tokio::io::AsyncReadExt;
            let mut buf = String::new();
            let _ = s.read_to_string(&mut buf).await;
            buf
        }
        None => String::new(),
    };
    let trimmed = stderr.trim();
    Err(format!(
        "download failed ({status}): {}",
        trimmed.lines().last().unwrap_or("unknown error")
    ))
}

fn cleanup_partials(dir: &Path, track_id: &str) {
    for ext in [
        "mp3",
        "webm",
        "m4a",
        "opus",
        "part",
        "webm.part",
        "m4a.part",
    ] {
        let _ = std::fs::remove_file(dir.join(format!("{track_id}.{ext}")));
    }
}

fn parse_download_pct(line: &str) -> Option<f64> {
    let content = line.trim().strip_prefix("[download]")?;
    let pct_end = content.find('%')?;
    content[..pct_end].trim().parse::<f64>().ok()
}
