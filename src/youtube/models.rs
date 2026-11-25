use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::api::models::{
    Album, Artist, ArtistRef, Image, Owner, PlayableItem, Playlist, PlaylistItem, Track,
    TrackCount,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YtTrack {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub thumbnail: String,
    pub duration_secs: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub tracks: Vec<YtTrack>,
    pub source: SearchSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchSource {
    Local,
    Remote,
    Mixed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YtPlaylist {
    pub id: i64,
    pub name: String,
    pub thumbnail: String,
    pub track_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExploreSection {
    pub title: String,
    pub tracks: Vec<YtTrack>,
}

/// Lossless, colon-free artist ID derived from an artist/channel display name.
pub fn encode_artist_id(name: &str) -> String {
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(name.as_bytes());
    format!("ytart_{encoded}")
}

/// Decode an artist ID created by [`encode_artist_id`].
pub fn decode_artist_id(id: &str) -> Option<String> {
    let encoded = id.strip_prefix("ytart_")?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .ok()?;
    String::from_utf8(bytes).ok()
}

pub fn thumbnail_images(url: &str) -> Vec<Image> {
    if url.trim().is_empty() {
        return Vec::new();
    }
    vec![Image {
        url: url.to_string(),
        width: Some(320),
        height: Some(180),
    }]
}

impl YtTrack {
    pub fn to_spotify_track(&self) -> Track {
        let artist_id = encode_artist_id(&self.artist);
        let images = thumbnail_images(&self.thumbnail);
        let artist_ref = ArtistRef {
            id: Some(artist_id.clone()),
            name: self.artist.clone(),
            uri: Some(format!("spotify:artist:{artist_id}")),
        };
        let album_id = format!("yt_{}", self.id);
        Track {
            id: Some(self.id.clone()),
            name: self.title.clone(),
            uri: format!("spotify:track:{}", self.id),
            duration_ms: (self.duration_secs * 1000.0).round().max(0.0) as u32,
            explicit: false,
            artists: vec![artist_ref.clone()],
            album: Some(Album {
                id: album_id.clone(),
                name: self.artist.clone(),
                uri: format!("spotify:album:{album_id}"),
                album_type: Some("single".to_string()),
                total_tracks: Some(1),
                images,
                artists: vec![artist_ref],
                ..Album::default()
            }),
            track_number: Some(1),
            disc_number: Some(1),
            is_local: false,
            is_playable: Some(true),
            popularity: None,
            ..Track::default()
        }
    }

    pub fn to_playable_item(&self) -> PlayableItem {
        PlayableItem::Track(self.to_spotify_track())
    }

    pub fn to_playlist_item(&self) -> PlaylistItem {
        PlaylistItem {
            added_at: None,
            added_by: None,
            is_local: false,
            item: Some(self.to_playable_item()),
            track: None,
        }
    }
}

impl YtPlaylist {
    pub fn to_spotify_playlist(&self) -> Playlist {
        let id = self.id.to_string();
        let count = u32::try_from(self.track_count.max(0)).unwrap_or(0);
        Playlist {
            id: id.clone(),
            name: self.name.clone(),
            uri: format!("spotify:playlist:{id}"),
            description: None,
            images: thumbnail_images(&self.thumbnail),
            owner: Owner {
                id: Some("local".into()),
                display_name: Some("Resonix".into()),
                uri: None,
            },
            public: Some(false),
            collaborative: false,
            snapshot_id: Some(format!("yt-pl-{}-{}", self.id, self.track_count)),
            tracks: Some(TrackCount { total: count }),
            items_count: Some(TrackCount { total: count }),
            followers: None,
            ..Playlist::default()
        }
    }
}

pub fn artist_from_name(name: &str, thumbnail: Option<&str>) -> Artist {
    let id = encode_artist_id(name);
    Artist {
        id: id.clone(),
        name: name.to_string(),
        uri: format!("spotify:artist:{id}"),
        images: thumbnail.map(thumbnail_images).unwrap_or_default(),
        ..Artist::default()
    }
}
