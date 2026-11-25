//! yt-dlp binary resolution and opt-in self-update.
//!
//! Resolution order:
//!   1. `RESONIX_YTDLP_PATH` or `SUNDER_YTDLP_PATH` env: explicit user override
//!   2. `<state_dir>/bin/yt-dlp`: app-managed copy
//!   3. `"yt-dlp"`: system PATH lookup

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::process::NoWindow;

pub const RELEASE_URL: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp";

pub fn managed_bin_path(data_dir: &Path) -> PathBuf {
    data_dir.join("bin").join("yt-dlp")
}

fn is_executable_file(p: &Path) -> bool {
    let Ok(md) = std::fs::metadata(p) else {
        return false;
    };
    if !md.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        md.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn env_override() -> Option<String> {
    for key in ["RESONIX_YTDLP_PATH", "SUNDER_YTDLP_PATH"] {
        if let Ok(p) = std::env::var(key)
            && !p.trim().is_empty()
        {
            return Some(p);
        }
    }
    None
}

/// Resolve which yt-dlp binary to spawn, in the documented order.
pub fn resolve_bin(managed: Option<&Path>) -> String {
    if let Some(p) = env_override() {
        return p;
    }
    if let Some(p) = managed
        && is_executable_file(p)
    {
        return p.to_string_lossy().into_owned();
    }
    "yt-dlp".into()
}

pub async fn version_of(bin: &str) -> Option<String> {
    let out = tokio::process::Command::new(bin)
        .no_window()
        .arg("--version")
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if v.is_empty() || v.contains('\n') {
        return None;
    }
    Some(v)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct YtdlpStatus {
    pub source: String,
    pub path: String,
    pub version: Option<String>,
    pub can_update: bool,
}

fn update_capable() -> bool {
    cfg!(target_os = "linux")
}

pub async fn status(data_dir: &Path) -> YtdlpStatus {
    if let Some(p) = env_override() {
        return YtdlpStatus {
            source: "override".into(),
            version: version_of(&p).await,
            path: p,
            can_update: false,
        };
    }
    let managed = managed_bin_path(data_dir);
    if is_executable_file(&managed) {
        let p = managed.to_string_lossy().into_owned();
        return YtdlpStatus {
            source: "managed".into(),
            version: version_of(&p).await,
            path: p,
            can_update: update_capable(),
        };
    }
    YtdlpStatus {
        source: "system".into(),
        version: version_of("yt-dlp").await,
        path: "yt-dlp".into(),
        can_update: update_capable(),
    }
}

static UPDATE_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

pub async fn update(data_dir: &Path) -> Result<String, String> {
    if !update_capable() {
        return Err(
            "one-click yt-dlp update is only available on Linux. Update yt-dlp with your package manager"
                .into(),
        );
    }
    if env_override().is_some() {
        return Err(
            "yt-dlp path override is set and overrides the managed copy. Update that yt-dlp manually"
                .into(),
        );
    }
    if UPDATE_IN_FLIGHT.swap(true, Ordering::SeqCst) {
        return Err("yt-dlp update already in progress".into());
    }
    let result = do_update(data_dir).await;
    UPDATE_IN_FLIGHT.store(false, Ordering::SeqCst);
    result
}

async fn do_update(data_dir: &Path) -> Result<String, String> {
    let bin_dir = data_dir.join("bin");
    std::fs::create_dir_all(&bin_dir).map_err(|e| format!("failed to create bin dir: {e}"))?;
    let final_path = bin_dir.join("yt-dlp");
    let tmp_path = bin_dir.join(format!("yt-dlp.download-{}", std::process::id()));
    let _ = std::fs::remove_file(&tmp_path);

    let fail = |msg: String| -> Result<String, String> {
        let _ = std::fs::remove_file(&tmp_path);
        Err(msg)
    };

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| format!("failed to build http client: {e}"))?;
    let resp = client
        .get(RELEASE_URL)
        .send()
        .await
        .map_err(|e| format!("download failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("download failed: HTTP {}", resp.status()));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("download failed: {e}"))?;
    if bytes.len() < 1_000_000 {
        return fail(format!(
            "download suspiciously small ({} bytes); refusing to install",
            bytes.len()
        ));
    }
    std::fs::write(&tmp_path, &bytes).map_err(|e| format!("failed to write file: {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("failed to set executable bit: {e}"))?;
    }

    let bin_str = tmp_path.to_string_lossy().into_owned();
    let Some(version) = version_of(&bin_str).await else {
        return fail(
            "downloaded binary failed verification (it does not run). Possibly out of disk space or a bad download. Try again."
                .into(),
        );
    };

    #[cfg(windows)]
    let _ = std::fs::remove_file(&final_path);
    std::fs::rename(&tmp_path, &final_path).map_err(|e| format!("failed to install: {e}"))?;

    Ok(version)
}
