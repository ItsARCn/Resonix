//! Latest releases shelf, backed by YouTube explore in Resonix.

use crate::api::models::Album;
use crate::http::Http;

pub struct NewReleases {
    _http: Http,
}

impl NewReleases {
    pub fn new(http: Http) -> Self {
        Self { _http: http }
    }

    pub async fn cached_fallback(&self, _account: &str) -> Option<Vec<Album>> {
        None
    }

    pub async fn remember_fallback(&self, _account: &str, _albums: Vec<Album>) {}

    pub async fn clear(&self) {}
}
