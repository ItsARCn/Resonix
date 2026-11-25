//! Personalized Home shelves, backed by YouTube explore in Resonix.

use crate::api::ApiError;
use crate::api::models::Playlist;
use crate::http::Http;

pub(crate) const UNSUPPORTED_DJ_URI: &str = "spotify:playlist:37i9dQZF1EYkqdzj48dyYq";

/// Retained for backend state compatibility.
pub struct MadeForYou {
    _http: Http,
}

#[derive(Clone, Debug)]
pub struct Shelves {
    pub made_for_you: Vec<Playlist>,
    pub top_mixes: Result<Vec<Playlist>, ApiError>,
}

impl MadeForYou {
    pub fn new(http: Http) -> Self {
        Self { _http: http }
    }

    pub async fn clear(&self) {}
}
