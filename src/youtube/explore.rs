//! Home / Explore personalized discovery sections ported from Sunder.

use std::collections::{HashMap, HashSet};

use super::db::SearchCache;
use super::extractor::Extractor;
use super::models::{ExploreSection, YtTrack};

const RECENTLY_PLAYED_LIMIT: usize = 20;
const EXPLORE_TRACK_LIMIT: usize = 24;
const MAX_EXPLORE_QUERIES: usize = 6;
const MAX_TRACKS_PER_ARTIST_IN_SECTION: usize = 3;

#[derive(Clone)]
struct ExploreQuery {
    title: String,
    query: String,
    limit: usize,
    score: i64,
}

pub async fn build_explore(
    db: &SearchCache,
    extractor: &Extractor,
) -> Result<Vec<ExploreSection>, String> {
    let listen_count = db.listen_count().unwrap_or(0);
    let mut sections: Vec<ExploreSection> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();

    let mut queries = if listen_count < 5 {
        starter_queries()
    } else {
        let recent = db
            .recently_played(RECENTLY_PLAYED_LIMIT)
            .unwrap_or_default();
        let artist_signals = db.artist_affinities(12).unwrap_or_default();
        let keywords = db.title_keywords(20).unwrap_or_default();
        let recent_ids = db.recent_track_ids(14).unwrap_or_default();
        seen_ids.extend(recent_ids);

        if !recent.is_empty() {
            seen_ids.extend(recent.iter().map(|track| track.id.clone()));
            sections.push(ExploreSection {
                title: "Jump Back In".to_string(),
                tracks: recent.clone(),
            });
        }

        personalized_queries(&artist_signals, &keywords, &recent, listen_count)
    };

    dedupe_queries(&mut queries);
    queries.sort_by_key(|b| std::cmp::Reverse(b.score));
    queries.truncate(MAX_EXPLORE_QUERIES);

    let results: Vec<Vec<YtTrack>> = futures_util::future::join_all(queries.iter().map(|seed| async {
        let mut tracks = Vec::new();
        let mut result_seen = HashSet::new();

        if let Ok(youtube_tracks) = extractor
            .search_youtube(seed.query.as_str(), seed.limit)
            .await
        {
            append_unique(&mut tracks, &mut result_seen, youtube_tracks);
        }

        if tracks.len() < EXPLORE_TRACK_LIMIT {
            let fill_limit = seed.limit.saturating_sub(tracks.len()).max(10);
            if let Ok(music_tracks) = extractor.search(seed.query.as_str(), fill_limit).await {
                append_unique(&mut tracks, &mut result_seen, music_tracks);
            }
        }

        tracks
    }))
    .await;

    for (seed, tracks) in queries.into_iter().zip(results) {
        let filtered = select_section_tracks(tracks, &mut seen_ids);
        if !filtered.is_empty() {
            let _ = db.upsert_tracks(&filtered);
            sections.push(ExploreSection {
                title: seed.title,
                tracks: filtered,
            });
        }
    }

    Ok(sections)
}

fn starter_queries() -> Vec<ExploreQuery> {
    let starters = [
        ("Popular Right Now", "popular music hits official audio", 92),
        ("Fresh Finds", "new music discoveries official audio", 88),
        ("Chill Vibes", "chill relaxing songs official audio", 84),
        ("Upbeat Energy", "upbeat energetic songs official audio", 82),
        ("Indie Corner", "indie music discoveries official audio", 80),
        (
            "Electronic Beats",
            "electronic dance songs official audio",
            78,
        ),
        ("Acoustic Sessions", "acoustic singer songwriter songs", 76),
        ("R&B Soul", "rnb soul songs official audio", 74),
    ];
    let offset = chrono_bucket() % starters.len();
    let mut queries = Vec::new();
    for i in 0..5.min(starters.len()) {
        let (title, query, score) = starters[(offset + i) % starters.len()];
        queries.push(ExploreQuery {
            title: title.to_string(),
            query: query.to_string(),
            limit: 20,
            score: score - i as i64,
        });
    }
    queries
}

fn personalized_queries(
    artist_signals: &[(String, i64, i64)],
    keywords: &[(String, i64)],
    recent: &[YtTrack],
    listen_count: i64,
) -> Vec<ExploreQuery> {
    let mut queries = Vec::new();

    for (rank, (artist, total, recent_count)) in artist_signals.iter().take(4).enumerate() {
        let strategies = [
            format!("{artist} similar artists songs"),
            format!("{artist} radio similar songs"),
            format!("music like {artist} official audio"),
        ];
        let pick = simple_hash(artist) % strategies.len();
        queries.push(ExploreQuery {
            title: format!("{artist} Radio"),
            query: strategies[pick].clone(),
            limit: 20,
            score: 130 - rank as i64 * 8 + (*total).min(20) * 2 + (*recent_count).min(10) * 3,
        });
    }

    for (rank, track) in recent.iter().take(2).enumerate() {
        let title = clean_track_title(&track.title);
        if title.is_empty() || track.artist.trim().is_empty() {
            continue;
        }
        queries.push(ExploreQuery {
            title: format!("More Like {}", compact_title(&title, 30)),
            query: format!("songs like {} {}", track.artist, title),
            limit: 20,
            score: 112 - rank as i64 * 7,
        });
    }

    if artist_signals.len() >= 2 {
        let first = &artist_signals[0].0;
        let second = &artist_signals[artist_signals.len() / 2].0;
        queries.push(ExploreQuery {
            title: "Discovery Mix".to_string(),
            query: format!("{first} {second} similar songs"),
            limit: 20,
            score: 104,
        });
    }

    let mood_keywords = mood_keywords(keywords, artist_signals);
    for (rank, chunk) in mood_keywords.chunks(2).take(2).enumerate() {
        if chunk.len() < 2 {
            continue;
        }
        queries.push(ExploreQuery {
            title: format!(
                "More {}",
                chunk
                    .iter()
                    .map(|word| capitalize(word))
                    .collect::<Vec<_>>()
                    .join(" & ")
            ),
            query: format!("{} songs official audio", chunk.join(" ")),
            limit: 20,
            score: 96 - rank as i64 * 6,
        });
    }

    if let Some((artist, _, _)) = artist_signals.first() {
        queries.push(ExploreQuery {
            title: "Fresh For You".to_string(),
            query: if listen_count >= 20 {
                format!("new songs similar to {artist}")
            } else {
                format!("popular songs similar to {artist}")
            },
            limit: 20,
            score: 82,
        });
    }

    if queries.is_empty() {
        starter_queries()
    } else {
        queries
    }
}

fn mood_keywords(
    keywords: &[(String, i64)],
    artist_signals: &[(String, i64, i64)],
) -> Vec<String> {
    let artist_blob = artist_signals
        .iter()
        .map(|(artist, _, _)| artist.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ");

    keywords
        .iter()
        .filter(|(word, count)| *count >= 2 && !artist_blob.contains(word.as_str()))
        .take(6)
        .map(|(word, _)| word.clone())
        .collect()
}

fn dedupe_queries(queries: &mut Vec<ExploreQuery>) {
    let mut seen = HashSet::new();
    queries.retain(|seed| {
        let key = format!("{}|{}", seed.title.to_lowercase(), seed.query.to_lowercase());
        seen.insert(key)
    });
}

fn append_unique(tracks: &mut Vec<YtTrack>, seen: &mut HashSet<String>, incoming: Vec<YtTrack>) {
    for track in incoming {
        if seen.insert(track.id.clone()) {
            tracks.push(track);
        }
    }
}

fn select_section_tracks(tracks: Vec<YtTrack>, seen_ids: &mut HashSet<String>) -> Vec<YtTrack> {
    let mut candidates: Vec<(i64, usize, YtTrack)> = tracks
        .into_iter()
        .enumerate()
        .filter_map(|(index, track)| {
            if seen_ids.contains(&track.id) || !looks_like_song(&track) {
                return None;
            }
            Some((track_quality(&track, index), index, track))
        })
        .collect();

    candidates.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));

    let mut selected = Vec::new();
    let mut artist_counts: HashMap<String, usize> = HashMap::new();
    let mut title_keys: HashSet<String> = HashSet::new();
    for (_, _, track) in candidates {
        let artist_key = normalized_key(&track.artist);
        let title_key = normalized_key(&clean_track_title(&track.title));
        let artist_count = artist_counts.get(&artist_key).copied().unwrap_or(0);
        if !artist_key.is_empty() && artist_count >= MAX_TRACKS_PER_ARTIST_IN_SECTION {
            continue;
        }
        if !title_key.is_empty() && title_keys.contains(&title_key) {
            continue;
        }
        if !seen_ids.insert(track.id.clone()) {
            continue;
        }
        if !title_key.is_empty() {
            title_keys.insert(title_key);
        }
        if !artist_key.is_empty() {
            artist_counts.insert(artist_key, artist_count + 1);
        }
        selected.push(track);
        if selected.len() >= EXPLORE_TRACK_LIMIT {
            break;
        }
    }
    selected
}

fn normalized_key(value: &str) -> String {
    value.trim().to_lowercase()
}

fn track_quality(track: &YtTrack, index: usize) -> i64 {
    let mut score = 100 - index as i64;
    let duration = track.duration_secs;
    if (120.0..=420.0).contains(&duration) {
        score += 24;
    } else if (60.0..=600.0).contains(&duration) {
        score += 10;
    } else if duration > 900.0 {
        score -= 80;
    }
    if !track.thumbnail.is_empty() {
        score += 8;
    }
    let title = track.title.to_lowercase();
    if title.contains("official audio") || title.contains("official video") {
        score += 6;
    }
    if title.contains("full album")
        || title.contains("complete album")
        || title.contains("compilation")
        || title.contains("reaction")
        || title.contains("tutorial")
    {
        score -= 90;
    }
    score
}

fn looks_like_song(track: &YtTrack) -> bool {
    let duration = track.duration_secs;
    if duration > 0.0 && !(30.0..=1200.0).contains(&duration) {
        return false;
    }
    let title = track.title.to_lowercase();
    let long_form = [
        "full album",
        "complete album",
        "1 hour",
        "hour mix",
        "playlist",
        "compilation",
        "reaction",
        "tutorial",
        "documentary",
        "interview",
    ];
    !(duration > 480.0 && long_form.iter().any(|needle| title.contains(needle)))
}

fn clean_track_title(title: &str) -> String {
    let mut cleaned = title.to_string();
    for marker in ["(", "[", " - Official", " | Official"] {
        if let Some(idx) = cleaned.find(marker) {
            cleaned.truncate(idx);
        }
    }
    cleaned.trim().to_string()
}

fn compact_title(title: &str, limit: usize) -> String {
    if title.chars().count() <= limit {
        return title.to_string();
    }
    let mut compact = title
        .chars()
        .take(limit.saturating_sub(1))
        .collect::<String>();
    compact.push_str("...");
    compact
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().to_string() + c.as_str(),
        None => String::new(),
    }
}

fn simple_hash(s: &str) -> usize {
    s.bytes()
        .fold(0usize, |acc, b| acc.wrapping_mul(31).wrapping_add(b as usize))
}

fn chrono_bucket() -> usize {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    (secs / (60 * 60 * 6)) as usize
}
