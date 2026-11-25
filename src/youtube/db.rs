use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

use rusqlite::{Connection, params};

use super::error::AppError;
use super::models::{YtPlaylist, YtTrack};

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct CachedLyrics {
    pub content: String,
    pub synced_lyrics: String,
    pub source: String,
}

pub struct SearchCache {
    conn: Mutex<Connection>,
}

impl SearchCache {
    pub fn new(data_dir: &Path) -> Result<Self, AppError> {
        std::fs::create_dir_all(data_dir)?;
        let db_path = data_dir.join("resonix.db");
        let conn = Connection::open(db_path)?;

        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;

             CREATE TABLE IF NOT EXISTS tracks (
                 id        TEXT PRIMARY KEY,
                 title     TEXT NOT NULL,
                 artist    TEXT NOT NULL,
                 thumbnail TEXT NOT NULL DEFAULT '',
                 duration  REAL NOT NULL DEFAULT 0
             );

             CREATE VIRTUAL TABLE IF NOT EXISTS tracks_fts USING fts5(
                 title, artist,
                 content='tracks',
                 content_rowid='rowid'
             );

             CREATE TRIGGER IF NOT EXISTS tracks_ai AFTER INSERT ON tracks BEGIN
                 INSERT INTO tracks_fts(rowid, title, artist)
                 VALUES (new.rowid, new.title, new.artist);
             END;
             CREATE TRIGGER IF NOT EXISTS tracks_ad AFTER DELETE ON tracks BEGIN
                 INSERT INTO tracks_fts(tracks_fts, rowid, title, artist)
                 VALUES ('delete', old.rowid, old.title, old.artist);
             END;
             CREATE TRIGGER IF NOT EXISTS tracks_au AFTER UPDATE ON tracks BEGIN
                 INSERT INTO tracks_fts(tracks_fts, rowid, title, artist)
                 VALUES ('delete', old.rowid, old.title, old.artist);
                 INSERT INTO tracks_fts(rowid, title, artist)
                 VALUES (new.rowid, new.title, new.artist);
             END;

             CREATE TABLE IF NOT EXISTS playlists (
                 id         INTEGER PRIMARY KEY AUTOINCREMENT,
                 name       TEXT NOT NULL,
                 thumbnail  TEXT NOT NULL DEFAULT '',
                 source_url TEXT NOT NULL DEFAULT '',
                 created    TEXT NOT NULL DEFAULT (datetime('now'))
             );

             CREATE TABLE IF NOT EXISTS playlist_tracks (
                 playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
                 track_id    TEXT NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
                 position    INTEGER NOT NULL DEFAULT 0,
                 added       TEXT NOT NULL DEFAULT (datetime('now')),
                 PRIMARY KEY (playlist_id, track_id)
             );

             CREATE TABLE IF NOT EXISTS liked_tracks (
                 track_id TEXT PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
                 added    TEXT NOT NULL DEFAULT (datetime('now'))
             );
             CREATE INDEX IF NOT EXISTS idx_liked_added ON liked_tracks(added DESC);

             CREATE TABLE IF NOT EXISTS listen_history (
                 id       INTEGER PRIMARY KEY AUTOINCREMENT,
                 track_id TEXT NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
                 played   TEXT NOT NULL DEFAULT (datetime('now'))
             );
             CREATE INDEX IF NOT EXISTS idx_history_track ON listen_history(track_id);
             CREATE INDEX IF NOT EXISTS idx_history_played ON listen_history(played DESC);

             CREATE TABLE IF NOT EXISTS lyrics_cache (
                 track_id      TEXT PRIMARY KEY,
                 content       TEXT NOT NULL DEFAULT '',
                 synced_lyrics TEXT NOT NULL DEFAULT '',
                 source        TEXT NOT NULL DEFAULT '',
                 fetched_at    TEXT NOT NULL DEFAULT (datetime('now'))
             );
             CREATE INDEX IF NOT EXISTS idx_lyrics_fetched ON lyrics_cache(fetched_at);

             CREATE TABLE IF NOT EXISTS lyric_offsets (
                 track_id  TEXT PRIMARY KEY,
                 offset_ms INTEGER NOT NULL DEFAULT 0
             );

             CREATE TABLE IF NOT EXISTS downloads (
                 track_id   TEXT PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
                 path       TEXT NOT NULL,
                 size       INTEGER NOT NULL DEFAULT 0,
                 downloaded TEXT NOT NULL DEFAULT (datetime('now'))
             );
             CREATE INDEX IF NOT EXISTS idx_downloads_time ON downloads(downloaded DESC);",
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn upsert_tracks(&self, tracks: &[YtTrack]) -> Result<(), AppError> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut stmt = conn.prepare_cached(
            "INSERT INTO tracks (id, title, artist, thumbnail, duration)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET
                 title = excluded.title,
                 artist = excluded.artist,
                 thumbnail = CASE WHEN excluded.thumbnail <> '' THEN excluded.thumbnail ELSE tracks.thumbnail END,
                 duration = CASE WHEN excluded.duration > 0 THEN excluded.duration ELSE tracks.duration END",
        )?;
        for t in tracks {
            stmt.execute(params![
                t.id,
                t.title,
                t.artist,
                t.thumbnail,
                t.duration_secs
            ])?;
        }
        Ok(())
    }

    pub fn get_track_by_id(&self, id: &str) -> Result<Option<YtTrack>, AppError> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut stmt = conn.prepare_cached(
            "SELECT id, title, artist, thumbnail, duration FROM tracks WHERE id = ?1",
        )?;
        let track = stmt
            .query_row(params![id], |row| {
                Ok(YtTrack {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    artist: row.get(2)?,
                    thumbnail: row.get(3)?,
                    duration_secs: row.get(4)?,
                    stream_url: None,
                })
            })
            .ok();
        Ok(track)
    }

    pub fn get_tracks_by_ids(&self, ids: &[String]) -> Result<Vec<YtTrack>, AppError> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let placeholders: String = std::iter::repeat_n("?", ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT id, title, artist, thumbnail, duration FROM tracks WHERE id IN ({placeholders})"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows: Vec<YtTrack> = stmt
            .query_map(rusqlite::params_from_iter(ids.iter()), |row| {
                Ok(YtTrack {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    artist: row.get(2)?,
                    thumbnail: row.get(3)?,
                    duration_secs: row.get(4)?,
                    stream_url: None,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        let map: HashMap<String, YtTrack> = rows.into_iter().map(|t| (t.id.clone(), t)).collect();
        Ok(ids.iter().filter_map(|id| map.get(id).cloned()).collect())
    }

    pub fn search_local(&self, query: &str) -> Result<Vec<YtTrack>, AppError> {
        let terms: Vec<String> = query
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(|w| format!("{w}*"))
            .collect();
        if terms.is_empty() {
            return Ok(vec![]);
        }
        let fts_query = terms.join(" ");

        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut stmt = conn.prepare_cached(
            "SELECT t.id, t.title, t.artist, t.thumbnail, t.duration
             FROM tracks_fts f
             JOIN tracks t ON t.rowid = f.rowid
             WHERE tracks_fts MATCH ?1
             ORDER BY rank
             LIMIT 20",
        )?;

        let tracks = stmt
            .query_map(params![fts_query], |row| {
                Ok(YtTrack {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    artist: row.get(2)?,
                    thumbnail: row.get(3)?,
                    duration_secs: row.get(4)?,
                    stream_url: None,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(tracks)
    }

    pub fn tracks_by_artist(&self, artist: &str, limit: usize) -> Result<Vec<YtTrack>, AppError> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut stmt = conn.prepare_cached(
            "SELECT id, title, artist, thumbnail, duration
             FROM tracks
             WHERE lower(artist) = lower(?1)
             LIMIT ?2",
        )?;
        let tracks = stmt
            .query_map(params![artist, limit as i64], |row| {
                Ok(YtTrack {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    artist: row.get(2)?,
                    thumbnail: row.get(3)?,
                    duration_secs: row.get(4)?,
                    stream_url: None,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(tracks)
    }

    // ---- Playlists ---------------------------------------------------------

    pub fn create_playlist(&self, name: &str, thumbnail: &str) -> Result<YtPlaylist, AppError> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        conn.execute(
            "INSERT INTO playlists (name, thumbnail) VALUES (?1, ?2)",
            params![name, thumbnail],
        )?;
        let id = conn.last_insert_rowid();
        Ok(YtPlaylist {
            id,
            name: name.to_string(),
            thumbnail: thumbnail.to_string(),
            track_count: 0,
        })
    }

    pub fn get_playlist(&self, playlist_id: i64) -> Result<Option<YtPlaylist>, AppError> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut stmt = conn.prepare_cached(
            "SELECT p.id, p.name, p.thumbnail, COUNT(pt.track_id)
             FROM playlists p
             LEFT JOIN playlist_tracks pt ON pt.playlist_id = p.id
             WHERE p.id = ?1
             GROUP BY p.id",
        )?;
        let pl = stmt
            .query_row(params![playlist_id], |row| {
                Ok(YtPlaylist {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    thumbnail: row.get(2)?,
                    track_count: row.get(3)?,
                })
            })
            .ok();
        Ok(pl)
    }

    pub fn list_playlists(&self) -> Result<Vec<YtPlaylist>, AppError> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut stmt = conn.prepare_cached(
            "SELECT p.id, p.name, p.thumbnail, COUNT(pt.track_id)
             FROM playlists p
             LEFT JOIN playlist_tracks pt ON pt.playlist_id = p.id
             GROUP BY p.id ORDER BY p.created DESC",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(YtPlaylist {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    thumbnail: row.get(2)?,
                    track_count: row.get(3)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    pub fn delete_playlist(&self, playlist_id: i64) -> Result<(), AppError> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        conn.execute("DELETE FROM playlists WHERE id = ?1", params![playlist_id])?;
        Ok(())
    }

    pub fn rename_playlist(&self, playlist_id: i64, name: &str) -> Result<(), AppError> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        conn.execute(
            "UPDATE playlists SET name = ?2 WHERE id = ?1",
            params![playlist_id, name],
        )?;
        Ok(())
    }

    pub fn add_to_playlist(&self, playlist_id: i64, track_id: &str) -> Result<(), AppError> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let pos: i64 = conn.query_row(
            "SELECT COALESCE(MAX(position), -1) + 1 FROM playlist_tracks WHERE playlist_id = ?1",
            params![playlist_id],
            |r| r.get(0),
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO playlist_tracks (playlist_id, track_id, position) VALUES (?1, ?2, ?3)",
            params![playlist_id, track_id, pos],
        )?;
        conn.execute(
            "UPDATE playlists
             SET thumbnail = (SELECT thumbnail FROM tracks WHERE id = ?2)
             WHERE id = ?1 AND thumbnail = ''",
            params![playlist_id, track_id],
        )?;
        Ok(())
    }

    pub fn remove_from_playlist(&self, playlist_id: i64, track_id: &str) -> Result<(), AppError> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        conn.execute(
            "DELETE FROM playlist_tracks WHERE playlist_id = ?1 AND track_id = ?2",
            params![playlist_id, track_id],
        )?;
        Ok(())
    }

    pub fn reorder_playlist_tracks(
        &self,
        playlist_id: i64,
        track_ids: &[String],
    ) -> Result<(), AppError> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let tx = conn.unchecked_transaction()?;
        for (i, tid) in track_ids.iter().enumerate() {
            tx.execute(
                "UPDATE playlist_tracks SET position = ?3 WHERE playlist_id = ?1 AND track_id = ?2",
                params![playlist_id, tid, i as i64],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn get_playlist_tracks(&self, playlist_id: i64) -> Result<Vec<YtTrack>, AppError> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut stmt = conn.prepare_cached(
            "SELECT t.id, t.title, t.artist, t.thumbnail, t.duration
             FROM playlist_tracks pt
             JOIN tracks t ON t.id = pt.track_id
             WHERE pt.playlist_id = ?1
             ORDER BY pt.position",
        )?;
        let tracks = stmt
            .query_map(params![playlist_id], |row| {
                Ok(YtTrack {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    artist: row.get(2)?,
                    thumbnail: row.get(3)?,
                    duration_secs: row.get(4)?,
                    stream_url: None,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(tracks)
    }

    pub fn set_playlist_source(&self, id: i64, url: &str) -> Result<(), AppError> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        conn.execute(
            "UPDATE playlists SET source_url = ?1 WHERE id = ?2",
            params![url, id],
        )?;
        Ok(())
    }

    // ---- Liked Songs -------------------------------------------------------

    pub fn set_liked(&self, track_id: &str, liked: bool) -> Result<(), AppError> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if liked {
            conn.execute(
                "INSERT OR IGNORE INTO liked_tracks (track_id) VALUES (?1)",
                params![track_id],
            )?;
        } else {
            conn.execute(
                "DELETE FROM liked_tracks WHERE track_id = ?1",
                params![track_id],
            )?;
        }
        Ok(())
    }

    pub fn is_liked(&self, track_id: &str) -> Result<bool, AppError> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM liked_tracks WHERE track_id = ?1",
                params![track_id],
                |_| Ok(true),
            )
            .unwrap_or(false);
        Ok(exists)
    }

    pub fn liked_tracks(&self, offset: u32, limit: u32) -> Result<(Vec<YtTrack>, u32), AppError> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let total: u32 = conn
            .query_row("SELECT COUNT(*) FROM liked_tracks", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap_or(0)
            .max(0) as u32;
        let mut stmt = conn.prepare_cached(
            "SELECT t.id, t.title, t.artist, t.thumbnail, t.duration
             FROM liked_tracks l
             JOIN tracks t ON t.id = l.track_id
             ORDER BY l.added DESC
             LIMIT ?1 OFFSET ?2",
        )?;
        let tracks = stmt
            .query_map(params![limit as i64, offset as i64], |row| {
                Ok(YtTrack {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    artist: row.get(2)?,
                    thumbnail: row.get(3)?,
                    duration_secs: row.get(4)?,
                    stream_url: None,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok((tracks, total))
    }

    // ---- Listen History & Affinities ---------------------------------------

    pub fn record_listen(&self, track_id: &str) -> Result<(), AppError> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        conn.execute(
            "INSERT INTO listen_history (track_id) VALUES (?1)",
            params![track_id],
        )?;
        Ok(())
    }

    pub fn recently_played(&self, limit: usize) -> Result<Vec<YtTrack>, AppError> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut stmt = conn.prepare_cached(
            "SELECT t.id, t.title, t.artist, t.thumbnail, t.duration
             FROM listen_history h
             JOIN tracks t ON t.id = h.track_id
             GROUP BY t.id
             ORDER BY MAX(h.played) DESC LIMIT ?1",
        )?;
        let tracks = stmt
            .query_map(params![limit as i64], |row| {
                Ok(YtTrack {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    artist: row.get(2)?,
                    thumbnail: row.get(3)?,
                    duration_secs: row.get(4)?,
                    stream_url: None,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(tracks)
    }

    pub fn top_tracks(&self, limit: usize) -> Result<Vec<YtTrack>, AppError> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut stmt = conn.prepare_cached(
            "SELECT t.id, t.title, t.artist, t.thumbnail, t.duration
             FROM listen_history h
             JOIN tracks t ON t.id = h.track_id
             GROUP BY t.id
             ORDER BY COUNT(*) DESC, MAX(h.played) DESC
             LIMIT ?1",
        )?;
        let tracks = stmt
            .query_map(params![limit as i64], |row| {
                Ok(YtTrack {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    artist: row.get(2)?,
                    thumbnail: row.get(3)?,
                    duration_secs: row.get(4)?,
                    stream_url: None,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(tracks)
    }

    pub fn artist_affinities(&self, limit: usize) -> Result<Vec<(String, i64, i64)>, AppError> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut stmt = conn.prepare_cached(
            "SELECT t.artist,
                    COUNT(*) as total_count,
                    SUM(CASE WHEN h.played >= datetime('now', '-30 days') THEN 1 ELSE 0 END) as recent_count
             FROM listen_history h
             JOIN tracks t ON t.id = h.track_id
             WHERE TRIM(t.artist) <> '' AND t.artist <> 'Unknown'
             GROUP BY t.artist
             ORDER BY total_count DESC, recent_count DESC
             LIMIT ?1",
        )?;
        let artists = stmt
            .query_map(params![limit as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(artists)
    }

    pub fn recent_track_ids(&self, days: i64) -> Result<HashSet<String>, AppError> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut stmt = conn.prepare_cached(
            "SELECT DISTINCT track_id FROM listen_history
             WHERE played >= datetime('now', ?1)",
        )?;
        let offset = format!("-{days} days");
        let ids = stmt
            .query_map(params![offset], |row| row.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(ids)
    }

    pub fn title_keywords(&self, limit: usize) -> Result<Vec<(String, i64)>, AppError> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut stmt = conn.prepare_cached(
            "SELECT t.title FROM listen_history h
             JOIN tracks t ON t.id = h.track_id
             ORDER BY h.played DESC LIMIT 200",
        )?;
        let titles: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();

        let mut freq: HashMap<String, i64> = HashMap::new();
        let stopwords: HashSet<&str> = [
            "the",
            "a",
            "an",
            "and",
            "or",
            "of",
            "in",
            "on",
            "at",
            "to",
            "for",
            "is",
            "it",
            "my",
            "me",
            "i",
            "you",
            "we",
            "he",
            "she",
            "this",
            "that",
            "with",
            "from",
            "by",
            "not",
            "no",
            "but",
            "so",
            "if",
            "up",
            "out",
            "all",
            "just",
            "like",
            "one",
            "do",
            "don",
            "be",
            "am",
            "are",
            "was",
            "has",
            "had",
            "have",
            "will",
            "can",
            "would",
            "could",
            "should",
            "ft",
            "feat",
            "vs",
            "official",
            "video",
            "audio",
            "music",
            "lyric",
            "lyrics",
            "visualizer",
            "visualiser",
            "hd",
            "hq",
            "full",
            "new",
            "version",
            "album",
            "single",
            "ep",
        ]
        .into_iter()
        .collect();

        for title in &titles {
            let cleaned = title
                .to_lowercase()
                .replace(|c: char| !c.is_alphanumeric() && c != ' ', " ");
            for word in cleaned.split_whitespace() {
                if word.len() < 3 || stopwords.contains(word) {
                    continue;
                }
                *freq.entry(word.to_string()).or_default() += 1;
            }
        }

        let mut pairs: Vec<_> = freq.into_iter().collect();
        pairs.sort_by_key(|p| std::cmp::Reverse(p.1));
        pairs.truncate(limit);
        Ok(pairs)
    }

    pub fn listen_count(&self) -> Result<i64, AppError> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM listen_history", [], |r| r.get(0))?;
        Ok(count)
    }

    // ---- Downloads & Lyrics Cache ------------------------------------------

    pub fn mark_downloaded(&self, track_id: &str, path: &str, size: u64) -> Result<(), AppError> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        conn.execute(
            "INSERT INTO downloads (track_id, path, size) VALUES (?1, ?2, ?3)
             ON CONFLICT(track_id) DO UPDATE SET
                 path = excluded.path,
                 size = excluded.size,
                 downloaded = datetime('now')",
            params![track_id, path, size as i64],
        )?;
        Ok(())
    }

    pub fn remove_download(&self, track_id: &str) -> Result<(), AppError> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        conn.execute(
            "DELETE FROM downloads WHERE track_id = ?1",
            params![track_id],
        )?;
        Ok(())
    }

    pub fn is_downloaded(&self, track_id: &str) -> Result<bool, AppError> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM downloads WHERE track_id = ?1",
                params![track_id],
                |_| Ok(true),
            )
            .unwrap_or(false);
        Ok(exists)
    }

    pub fn get_lyrics(&self, track_id: &str) -> Result<Option<CachedLyrics>, AppError> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut stmt = conn.prepare_cached(
            "SELECT content, synced_lyrics, source FROM lyrics_cache WHERE track_id = ?1",
        )?;
        let mut rows = stmt.query(params![track_id])?;
        if let Some(row) = rows.next()? {
            let content: String = row.get(0)?;
            let synced_lyrics: String = row.get(1)?;
            let source: String = row.get(2)?;
            if content.is_empty() && synced_lyrics.is_empty() {
                return Ok(None);
            }
            Ok(Some(CachedLyrics {
                content,
                synced_lyrics,
                source,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn upsert_lyrics(
        &self,
        track_id: &str,
        content: &str,
        synced_lyrics: &str,
        source: &str,
    ) -> Result<(), AppError> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        conn.execute(
            "INSERT INTO lyrics_cache (track_id, content, synced_lyrics, source, fetched_at)
             VALUES (?1, ?2, ?3, ?4, datetime('now'))
             ON CONFLICT(track_id) DO UPDATE SET
                 content = excluded.content,
                 synced_lyrics = excluded.synced_lyrics,
                 source = excluded.source,
                 fetched_at = datetime('now')",
            params![track_id, content, synced_lyrics, source],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_cache() -> SearchCache {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("resonix_test_{}_{id}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        SearchCache::new(&dir).unwrap()
    }

    fn sample_track(id: &str) -> YtTrack {
        YtTrack {
            id: id.to_string(),
            title: format!("Track {id}"),
            artist: "Test Artist".into(),
            thumbnail: String::new(),
            duration_secs: 210.0,
            stream_url: None,
        }
    }

    #[test]
    fn playlist_and_liked_crud() {
        let db = temp_cache();
        db.upsert_tracks(&[sample_track("t1"), sample_track("t2")])
            .unwrap();

        let pl = db.create_playlist("My List", "").unwrap();
        db.add_to_playlist(pl.id, "t1").unwrap();
        db.add_to_playlist(pl.id, "t2").unwrap();
        assert_eq!(db.get_playlist_tracks(pl.id).unwrap().len(), 2);

        db.set_liked("t1", true).unwrap();
        assert!(db.is_liked("t1").unwrap());
        let (liked, total) = db.liked_tracks(0, 50).unwrap();
        assert_eq!(total, 1);
        assert_eq!(liked[0].id, "t1");

        db.set_liked("t1", false).unwrap();
        assert!(!db.is_liked("t1").unwrap());
    }
}
