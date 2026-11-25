//! Session-scoped cookie support for yt-dlp spawns.
//!
//! Cookie source is NEVER persisted: held in memory only for the process
//! lifetime so every app restart resets it to [`CookieSource::None`].

use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Where yt-dlp should take cookies from for this session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CookieSource {
    None,
    Browser(String),
    File(PathBuf),
}

pub const ALLOWED_BROWSERS: [&str; 4] = ["firefox", "chrome", "chromium", "edge"];

static COOKIE_SOURCE: Mutex<Option<CookieSource>> = Mutex::new(None);

pub fn set(source: CookieSource) {
    let mut guard = COOKIE_SOURCE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = Some(source);
}

pub fn get() -> CookieSource {
    COOKIE_SOURCE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
        .unwrap_or(CookieSource::None)
}

pub fn current_args() -> Vec<String> {
    match get() {
        CookieSource::None => Vec::new(),
        CookieSource::Browser(b) => vec!["--cookies-from-browser".into(), b],
        CookieSource::File(p) => vec!["--cookies".into(), p.to_string_lossy().into_owned()],
    }
}

pub fn parse_cookie_source(raw: &str) -> Result<CookieSource, String> {
    let s = raw.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("none") {
        return Ok(CookieSource::None);
    }
    let lower = s.to_ascii_lowercase();
    if ALLOWED_BROWSERS.contains(&lower.as_str()) {
        return Ok(CookieSource::Browser(lower));
    }
    let is_txt = Path::new(s)
        .extension()
        .map(|e| e.eq_ignore_ascii_case("txt"))
        .unwrap_or(false);
    if is_txt {
        let p = PathBuf::from(s);
        if p.is_file() {
            return Ok(CookieSource::File(p));
        }
        return Err(format!("cookies file not found: {s}"));
    }
    Err(
        "invalid cookie source: use none, firefox, chrome, chromium, edge, or an existing .txt cookies file"
            .into(),
    )
}
