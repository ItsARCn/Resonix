//! Supplementary metadata and label normalization helpers.
//!
//! Spotify session reads have been replaced by Resonix's local SQLite and
//! YouTube extractor backend; this module preserves the shared data models
//! used across the UI and tests.

use std::collections::HashMap;

use crate::api::ApiError;

/// Why a catalogue read failed.
#[derive(Debug)]
pub enum Failure {
    Definitive(ApiError),
    Retry(anyhow::Error),
}

/// Supplementary album or artist metadata displayed on detail pages.
#[derive(Debug, Default)]
pub struct PageMetadata {
    pub play_counts: HashMap<String, u64>,
    pub upcoming: Option<crate::prerelease::UpcomingRelease>,
}

/// One playlist row picked for removal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemovedRow {
    pub uri: String,
    pub position: u32,
    pub copy: u32,
}

/// A station seeded by a track URI starts with that track once.
#[cfg(test)]
pub(crate) fn seed_first(station: &str, uris: Vec<String>) -> Vec<String> {
    let Some(seed) = crate::util::station_seed(station) else {
        return uris;
    };
    if crate::util::uri_kind(&seed) != Some("track") {
        return uris;
    }
    let mut ordered = Vec::with_capacity(uris.len() + 1);
    ordered.push(seed.clone());
    ordered.extend(uris.into_iter().filter(|uri| uri != &seed));
    ordered
}

/// Display name and avatar for a profile.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UserProfile {
    pub name: Option<String>,
    pub image_url: Option<String>,
}

/// Activity row shown in the Friends sidebar panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FriendActivity {
    pub user_uri: String,
    pub user_name: String,
    pub image_url: Option<String>,
    pub timestamp_ms: i64,
    pub playing: bool,
    pub track_uri: String,
    pub track_name: String,
    pub artist_uri: Option<String>,
    pub artist_name: String,
    pub album_uri: Option<String>,
    pub cover_url: Option<String>,
    pub context_uri: Option<String>,
    pub context_name: Option<String>,
}

#[derive(Default)]
pub struct FriendNames {
    pub profiles: HashMap<String, UserProfile>,
    pub playlists: HashMap<String, String>,
}
