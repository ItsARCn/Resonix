//! YouTube/Rust backend ported from Sunder for Resonix.

pub mod cookies;
pub mod db;
pub mod downloads;
pub mod error;
pub mod explore;
pub mod extractor;
pub mod models;
pub mod process;
pub mod ytdlp;

pub use db::{CachedLyrics, SearchCache};
pub use downloads::DownloadManager;
pub use error::{AppError, classify_playback_error};
pub use extractor::Extractor;
pub use models::{ExploreSection, SearchResult, SearchSource, YtPlaylist, YtTrack};
