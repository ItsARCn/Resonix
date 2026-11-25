use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("audio: {0}")]
    Audio(String),
    #[error("database: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("extraction: {0}")]
    Extraction(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

/// Classify a playback failure string into a remedy selector.
///
/// Kinds:
/// - `"ytdlp_botwall"`: YouTube's interactive bot wall ("Sign in to confirm
///   you're not a bot").
/// - `"ytdlp_blocked"`: YouTube's enforcement rejected the fetch itself
///   (media-CDN 403).
/// - `"load_failed"`: anything else; track-level remedy.
pub fn classify_playback_error(msg: &str) -> &'static str {
    let lower = msg.to_lowercase();
    if lower.contains("sign in to confirm") && lower.contains("not a bot") {
        "ytdlp_botwall"
    } else if msg.contains("HTTP Error 403") || msg.contains("unable to download video data") {
        "ytdlp_blocked"
    } else {
        "load_failed"
    }
}

#[cfg(test)]
mod tests {
    use super::classify_playback_error;

    #[test]
    fn classifies_403_media_fetch_as_blocked() {
        assert_eq!(
            classify_playback_error(
                "yt-dlp failed (exit code: 1): ERROR: unable to download video data: HTTP Error 403: Forbidden"
            ),
            "ytdlp_blocked"
        );
        assert_eq!(
            classify_playback_error("HTTP Error 403: Forbidden"),
            "ytdlp_blocked"
        );
    }

    #[test]
    fn classifies_bot_wall_both_apostrophe_variants() {
        assert_eq!(
            classify_playback_error(
                "ERROR: Sign in to confirm you\u{2019}re not a bot. This helps protect our community."
            ),
            "ytdlp_botwall"
        );
        assert_eq!(
            classify_playback_error("ERROR: Sign in to confirm you're not a bot"),
            "ytdlp_botwall"
        );
        assert_eq!(
            classify_playback_error("sign in to confirm you're not a bot"),
            "ytdlp_botwall"
        );
    }

    #[test]
    fn unrelated_failures_are_load_failed() {
        assert_eq!(classify_playback_error("no such video"), "load_failed");
        assert_eq!(
            classify_playback_error("decoder init failed"),
            "load_failed"
        );
    }
}
