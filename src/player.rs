//! Local YouTube/Rust audio playback engine.
//!
//! Replaces the librespot playback engine with yt-dlp media resolution,
//! rodio/symphonia decoding, and Spotifast's existing [`RodioSink`] +
//! [`Tapped`] DSP chain (10-band EQ, Winamp/player-bar/MilkDrop audio tap,
//! limiter, and smooth volume ramping).

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use rand::seq::SliceRandom;
use rodio::{Decoder, Source};
use sha1::{Digest, Sha1};
use tokio::sync::mpsc;

use crate::api::models::ArtistRef;
use crate::eq::{NUM_CHANNELS, SAMPLE_RATE};
use crate::resample::Resampler;
use crate::sink::{AtomicVolume, AudioControl, AudioPacket, RodioSink, Sink as _};
use crate::vis::{AudioTap, Tapped};
use crate::youtube::models::encode_artist_id;
use crate::youtube::process::NoWindow;
use crate::youtube::{DownloadManager, Extractor, SearchCache, YtTrack, cookies, ytdlp};

#[derive(Clone, Debug)]
pub struct EngineConfig {
    pub device_name: String,
    pub bitrate_kbps: u16,
    pub normalisation: bool,
    pub autoplay: bool,
    pub gapless: bool,
    /// The saved output, an id or an older build's name (`crate::sink::Saved`).
    pub audio_device: Option<String>,
    pub initial_volume: u16,
    pub volume_dir: PathBuf,
    pub audio_cache_dir: Option<PathBuf>,
    pub audio_cache_limit: Option<u64>,
    /// Output buffer length in milliseconds.
    pub buffer_ms: u32,
    pub tap: Arc<AudioTap>,
    /// The equalizer's settings, shared with the window that sets them.
    pub eq: crate::eq::SharedEq,
    /// Proxy used by HTTP fetches.
    pub proxy: crate::settings::ProxyConfig,
    /// Optional local files directory.
    pub local_files: Option<PathBuf>,
}

impl EngineConfig {
    /// A stable local device id derived from the device name.
    pub fn device_id(&self) -> String {
        hex(&Sha1::digest(self.device_name.as_bytes()))
    }

    fn effective_cache_dir(&self) -> PathBuf {
        self.audio_cache_dir
            .clone()
            .unwrap_or_else(|| self.volume_dir.join("audio-cache"))
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Playback {
    #[default]
    Stopped,
    Loading,
    Playing,
    Paused,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RepeatMode {
    #[default]
    Off,
    Context,
    Track,
}

impl RepeatMode {
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Context,
            Self::Context => Self::Track,
            Self::Track => Self::Off,
        }
    }

    pub fn api_name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Context => "context",
            Self::Track => "track",
        }
    }

    pub fn from_api(name: &str) -> Self {
        match name {
            "context" => Self::Context,
            "track" => Self::Track,
            _ => Self::Off,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LocalTrack {
    pub uri: String,
    pub title: String,
    pub artists: Vec<ArtistRef>,
    pub album: String,
    pub art_url: Option<String>,
    pub art_small_url: Option<String>,
    pub duration_ms: u32,
    pub is_episode: bool,
}

impl LocalTrack {
    pub fn artist_names(&self) -> String {
        crate::api::models::join_names(self.artists.iter().map(|artist| artist.name.as_str()))
    }
}

/// Legacy constant retained for UI banner classification when playback is blocked.
pub const AUDIO_KEY_REFUSED: &str = "YouTube refused playback for this track";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LocalState {
    pub playback: Playback,
    pub track: Option<LocalTrack>,
    pub position_ms: u32,
    /// When `position_ms` was observed; `None` while not advancing.
    pub position_at: Option<Instant>,
    pub volume: u16,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub connected: bool,
    pub username: String,
    pub active_client: String,
    pub error: Option<String>,
    pub seek_sequence: u64,
    /// The engine is fetching a track right now.
    pub loading: bool,
    /// Incremented for every newly loaded track, including replays of the same URI.
    pub track_sequence: u64,
    /// Same track replayed; next Playing/Paused resets media-control progress.
    pub replay_pending: bool,
}

/// Snapshot of what local playback was doing before an interruption.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interrupted {
    pub uri: String,
    pub position_ms: u32,
    pub playing: bool,
}

impl LocalState {
    pub fn interrupted(&self) -> Option<Interrupted> {
        let track = self.track.as_ref()?;
        if self.playback == Playback::Stopped {
            return None;
        }
        Some(Interrupted {
            uri: track.uri.clone(),
            position_ms: self.position_now(),
            playing: matches!(self.playback, Playback::Playing | Playback::Loading),
        })
    }

    /// The position now, interpolated from the last report while playing.
    pub fn position_now(&self) -> u32 {
        match (self.playback, self.position_at) {
            (Playback::Playing, Some(at)) => {
                let elapsed = at.elapsed().as_millis() as u32;
                let limit = self
                    .track
                    .as_ref()
                    .map_or(u32::MAX, |track| track.duration_ms.max(self.position_ms));
                self.position_ms.saturating_add(elapsed).min(limit)
            }
            _ => self.position_ms,
        }
    }

    pub fn is_active(&self) -> bool {
        self.track.is_some() && self.playback != Playback::Stopped
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LoadSpec {
    pub context_uri: Option<String>,
    pub uris: Vec<String>,
    pub offset_uri: Option<String>,
    pub offset_index: Option<u32>,
    pub position_ms: u32,
    pub play: bool,
    pub shuffle: Option<bool>,
    pub repeat: Option<RepeatMode>,
    pub autoplay: bool,
}

#[derive(Clone, Debug)]
pub enum PlaybackResume {
    Track(LoadSpec),
}

#[derive(Clone, Debug, PartialEq)]
pub enum PlayerCommand {
    Toggle,
    Next,
    Previous,
    /// Remove manually queued tracks and keep context tracks.
    ClearQueue,
    /// Queue a track after the ones already queued.
    AddToQueue(String),
    /// Remove one upcoming row out of the queue without playing it.
    RemoveNextTrack {
        index: usize,
        uri: String,
    },
    Seek(u32),
    Volume(u16),
    VolumePreview(u16),
    Shuffle(bool),
    Repeat(RepeatMode),
    Load(LoadSpec),
    Transfer,
}

#[allow(clippy::large_enum_variant)]
pub enum EngineEvent {
    State(LocalState),
    SessionEnded,
    AudioDevice {
        name: String,
        id: String,
    },
    RemoteChanged,
    RemoteVolume {
        device_id: String,
        volume: u16,
    },
}

pub type Notify = Arc<dyn Fn(EngineEvent) + Send + Sync>;

/// Kept-alive handle stub (retained for backend interface compatibility).
#[derive(Clone)]
pub struct KeptPlayer {
    state: Arc<Mutex<LocalState>>,
    cmd_tx: mpsc::UnboundedSender<PlayerCommand>,
}

impl KeptPlayer {
    pub fn stop(&self) {
        // Handled when Engine shuts down.
    }

    pub fn toggle(&self) -> bool {
        let _ = self.cmd_tx.send(PlayerCommand::Toggle);
        self.has_song()
    }

    pub fn has_song(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .interrupted()
            .is_some()
    }
}

#[derive(Clone)]
pub(crate) struct Heard {
    state: Arc<Mutex<LocalState>>,
}

impl Heard {
    pub(crate) fn level(&self) -> u16 {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).volume
    }

    #[cfg(test)]
    pub(crate) fn at(volume: u16) -> Self {
        Self {
            state: Arc::new(Mutex::new(LocalState {
                volume,
                ..LocalState::default()
            })),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rootlist {
    pub entries: Vec<RootlistEntry>,
    pub editable: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RootlistEntry {
    Playlist(String),
    FolderStart { id: String, name: String },
    FolderEnd,
}

pub fn parse_rootlist(uris: &[String]) -> Vec<RootlistEntry> {
    let mut entries = Vec::new();
    let mut depth = 0usize;
    for uri in uris {
        if let Some(rest) = uri.strip_prefix("spotify:start-group:") {
            let (id, name) = match rest.split_once(':') {
                Some((id, name)) => (id.to_string(), decode_folder_name(name)),
                None => (rest.to_string(), String::new()),
            };
            entries.push(RootlistEntry::FolderStart { id, name });
            depth += 1;
        } else if uri.starts_with("spotify:end-group:") {
            if depth > 0 {
                entries.push(RootlistEntry::FolderEnd);
                depth -= 1;
            }
        } else if uri.starts_with("spotify:playlist:") {
            entries.push(RootlistEntry::Playlist(uri.clone()));
        }
    }
    entries.extend(std::iter::repeat_n(RootlistEntry::FolderEnd, depth));
    entries
}

fn decode_folder_name(encoded: &str) -> String {
    let bytes = encoded.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let digit = |byte: u8| (byte as char).to_digit(16);
                match (digit(bytes[i + 1]), digit(bytes[i + 2])) {
                    (Some(high), Some(low)) => {
                        out.push((high << 4 | low) as u8);
                        i += 3;
                    }
                    _ => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Internal queue & metadata cache for playback.
#[derive(Default)]
struct QueueState {
    context_uri: Option<String>,
    /// Original unshuffled order of URIs in the active context.
    original_uris: Vec<String>,
    /// Active play order of context URIs.
    active_uris: Vec<String>,
    /// Cursor inside `active_uris` for the current context track.
    cursor: usize,
    /// True if the currently playing track came from `manual_queue` rather than `active_uris`.
    playing_manual: bool,
    /// Tracks queued manually via "Add to queue" (played next FIFO before resuming context).
    manual_queue: VecDeque<String>,
    /// Current playing URI (if any).
    current_uri: Option<String>,
    /// History of played URIs for Previous.
    played_history: Vec<String>,
}

impl QueueState {
    fn upcoming_uris(&self) -> Vec<String> {
        let mut out: Vec<String> = self.manual_queue.iter().cloned().collect();
        if !self.active_uris.is_empty() {
            let start = if self.playing_manual {
                self.cursor
            } else {
                self.cursor.saturating_add(1)
            };
            if start < self.active_uris.len() {
                out.extend(self.active_uris[start..].iter().cloned());
            }
        }
        out
    }

    fn apply_shuffle(&mut self, shuffle: bool) {
        if self.original_uris.is_empty() {
            return;
        }
        let current = self.current_uri.clone();
        if shuffle {
            let mut rest: Vec<String> = self
                .original_uris
                .iter()
                .filter(|uri| current.as_ref() != Some(*uri))
                .cloned()
                .collect();
            let mut rng = rand::rng();
            rest.shuffle(&mut rng);
            let mut next = Vec::with_capacity(self.original_uris.len());
            if let Some(cur) = current
                && self.original_uris.contains(&cur)
            {
                next.push(cur);
            }
            next.extend(rest);
            self.active_uris = next;
            self.cursor = 0;
        } else {
            self.active_uris = self.original_uris.clone();
            self.cursor = current
                .as_ref()
                .and_then(|cur| self.active_uris.iter().position(|u| u == cur))
                .unwrap_or(0);
        }
    }
}

/// Feeder thread shared control state.
struct FeederControl {
    generation: AtomicU64,
    paused: AtomicBool,
    shutdown: AtomicBool,
    wake: Condvar,
    inbox: Mutex<VecDeque<FeederCmd>>,
}

enum FeederCmd {
    PlayFile {
        generation: u64,
        path: PathBuf,
        start_ms: u32,
        play: bool,
    },
    Pause,
    Resume,
    Stop,
    Shutdown,
}

#[derive(Debug)]
enum EngineMsg {
    Cmd(PlayerCommand),
    TrackReady {
        generation: u64,
        uri: String,
        track: LocalTrack,
        path: PathBuf,
        start_ms: u32,
        play: bool,
        same_uri_replay: bool,
    },
    TrackError {
        generation: u64,
        uri: String,
        message: String,
    },
    TrackEnded {
        generation: u64,
    },
    Shutdown,
}

fn volume_to_gain(volume: u16) -> f32 {
    let norm = f32::from(volume) / f32::from(u16::MAX);
    norm * norm
}

pub struct Engine {
    device_id: String,
    state: Arc<Mutex<LocalState>>,
    queue: Arc<Mutex<QueueState>>,
    tracks: Arc<Mutex<HashMap<String, LocalTrack>>>,
    audio: Arc<AudioControl>,
    shutting_down: Arc<AtomicBool>,
    cmd_tx: mpsc::UnboundedSender<PlayerCommand>,
    msg_tx: mpsc::UnboundedSender<EngineMsg>,
    feeder_ctl: Arc<FeederControl>,
}

impl Engine {
    pub fn start_youtube(
        config: &EngineConfig,
        db: Arc<SearchCache>,
        extractor: Arc<Extractor>,
        notify: Notify,
    ) -> Result<Self> {
        let device_id = config.device_id();
        let initial_volume = config.initial_volume;
        let state = Arc::new(Mutex::new(LocalState {
            connected: true,
            username: "resonix".to_string(),
            volume: initial_volume,
            repeat: RepeatMode::Off,
            ..LocalState::default()
        }));
        let queue = Arc::new(Mutex::new(QueueState::default()));
        let tracks = Arc::new(Mutex::new(HashMap::new()));
        let audio = AudioControl::new(config.buffer_ms);
        let volume_bits = Arc::new(AtomicU32::new(volume_to_gain(initial_volume).to_bits()));
        let shutting_down = Arc::new(AtomicBool::new(false));

        let feeder_ctl = Arc::new(FeederControl {
            generation: AtomicU64::new(0),
            paused: AtomicBool::new(false),
            shutdown: AtomicBool::new(false),
            wake: Condvar::new(),
            inbox: Mutex::new(VecDeque::new()),
        });

        let (msg_tx, msg_rx) = mpsc::unbounded_channel();
        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel();

        // Spawn audio feeder thread owning RodioSink + Tapped DSP chain.
        spawn_audio_feeder(
            config.clone(),
            Arc::clone(&audio),
            Arc::clone(&volume_bits),
            Arc::clone(&feeder_ctl),
            msg_tx.clone(),
            notify.clone(),
        );

        let forward_tx = msg_tx.clone();
        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                if forward_tx.send(EngineMsg::Cmd(cmd)).is_err() {
                    break;
                }
            }
        });

        let cache_dir = config.effective_cache_dir();
        let state_dir = config.volume_dir.clone();
        let cache_limit = config.audio_cache_limit;
        let engine_state = Arc::clone(&state);
        let engine_queue = Arc::clone(&queue);
        let engine_tracks = Arc::clone(&tracks);
        let engine_audio = Arc::clone(&audio);
        let engine_feeder = Arc::clone(&feeder_ctl);
        let engine_volume_bits = Arc::clone(&volume_bits);
        let engine_notify = notify.clone();
        let engine_msg_tx = msg_tx.clone();

        tokio::spawn(async move {
            run_coordinator(
                msg_rx,
                engine_msg_tx,
                db,
                extractor,
                cache_dir,
                state_dir,
                cache_limit,
                engine_state,
                engine_queue,
                engine_tracks,
                engine_audio,
                engine_feeder,
                engine_volume_bits,
                engine_notify,
            )
            .await;
        });

        // Emit initial ready state.
        notify(EngineEvent::State(
            state.lock().unwrap_or_else(|p| p.into_inner()).clone(),
        ));

        Ok(Self {
            device_id,
            state,
            queue,
            tracks,
            audio,
            shutting_down,
            cmd_tx,
            msg_tx,
            feeder_ctl,
        })
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    pub(crate) fn heard(&self) -> Heard {
        Heard {
            state: Arc::clone(&self.state),
        }
    }

    pub fn interrupted(&self) -> Option<Interrupted> {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .interrupted()
    }

    pub fn state_snapshot(&self) -> LocalState {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn queue_snapshot(&self) -> (Option<LocalTrack>, Vec<String>, Vec<LocalTrack>) {
        let state = self.state.lock().unwrap_or_else(|p| p.into_inner()).clone();
        let upcoming_uris = self
            .queue
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .upcoming_uris();
        let known = self.tracks.lock().unwrap_or_else(|p| p.into_inner());
        let upcoming_tracks = upcoming_uris
            .iter()
            .filter_map(|uri| known.get(uri).cloned())
            .collect();
        (state.track, upcoming_uris, upcoming_tracks)
    }

    pub fn keep_player(&self) -> KeptPlayer {
        KeptPlayer {
            state: Arc::clone(&self.state),
            cmd_tx: self.cmd_tx.clone(),
        }
    }

    pub fn command(&self, command: PlayerCommand) -> Result<()> {
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err(anyhow!("engine is shutting down"));
        }
        self.msg_tx
            .send(EngineMsg::Cmd(command))
            .map_err(|_| anyhow!("player coordinator stopped"))
    }

    pub fn shutdown(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        self.feeder_ctl.shutdown.store(true, Ordering::SeqCst);
        self.audio.interrupt();
        let _ = self.msg_tx.send(EngineMsg::Shutdown);
        push_feeder(&self.feeder_ctl, FeederCmd::Shutdown);
    }
}

fn push_feeder(ctl: &FeederControl, cmd: FeederCmd) {
    ctl.inbox.lock().unwrap_or_else(|p| p.into_inner()).push_back(cmd);
    ctl.wake.notify_all();
}

#[allow(clippy::too_many_arguments)]
async fn run_coordinator(
    mut msg_rx: mpsc::UnboundedReceiver<EngineMsg>,
    msg_tx: mpsc::UnboundedSender<EngineMsg>,
    db: Arc<SearchCache>,
    extractor: Arc<Extractor>,
    cache_dir: PathBuf,
    state_dir: PathBuf,
    cache_limit: Option<u64>,
    state: Arc<Mutex<LocalState>>,
    queue: Arc<Mutex<QueueState>>,
    tracks: Arc<Mutex<HashMap<String, LocalTrack>>>,
    audio: Arc<AudioControl>,
    feeder_ctl: Arc<FeederControl>,
    volume_bits: Arc<AtomicU32>,
    notify: Notify,
) {
    let mut load_gen: u64 = 0;

    let emit_state = |st: &Arc<Mutex<LocalState>>, notify: &Notify| {
        let snapshot = st.lock().unwrap_or_else(|p| p.into_inner()).clone();
        notify(EngineEvent::State(snapshot));
    };

    let start_uri = |uri: String,
                     start_ms: u32,
                     play: bool,
                     load_gen: &mut u64,
                     queue: &Arc<Mutex<QueueState>>,
                     state: &Arc<Mutex<LocalState>>,
                     tracks: &Arc<Mutex<HashMap<String, LocalTrack>>>,
                     audio: &Arc<AudioControl>,
                     feeder_ctl: &Arc<FeederControl>,
                     notify: &Notify| {
        *load_gen = load_gen.wrapping_add(1);
        let track_gen = *load_gen;
        feeder_ctl.generation.store(track_gen, Ordering::SeqCst);
        audio.interrupt();

        let prev_uri = {
            let mut q = queue.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(prev) = q.current_uri.take()
                && prev != uri
            {
                q.played_history.push(prev);
            }
            q.current_uri = Some(uri.clone());
            q.played_history.last().cloned()
        };
        let same_uri = prev_uri.as_deref() == Some(uri.as_str()) || {
            state
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .track
                .as_ref()
                .is_some_and(|t| t.uri == uri)
        };

        // Pre-populate optimistic LocalTrack if already known or cached in SQLite.
        let cached_track = tracks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&uri)
            .cloned()
            .or_else(|| {
                let vid = video_id_from_uri(&uri)?;
                db.get_track_by_id(vid)
                    .ok()
                    .flatten()
                    .map(|t| local_track_from_yt(&t))
            });

        {
            let mut st = state.lock().unwrap_or_else(|p| p.into_inner());
            st.error = None;
            st.loading = true;
            st.position_ms = start_ms;
            st.position_at = None;
            if let Some(t) = cached_track.clone() {
                st.track = Some(t);
            }
            if play {
                st.playback = Playback::Loading;
            } else {
                st.playback = Playback::Paused;
            }
        }
        emit_state(state, notify);

        let db = Arc::clone(&db);
        let extractor = Arc::clone(&extractor);
        let cache_dir = cache_dir.clone();
        let state_dir = state_dir.clone();
        let msg_tx = msg_tx.clone();
        let feeder_ctl = Arc::clone(&feeder_ctl);

        tokio::spawn(async move {
            match prepare_audio_track(
                &uri,
                &db,
                &extractor,
                &cache_dir,
                &state_dir,
                cache_limit,
                track_gen,
            )
            .await
            {
                Ok((track, path)) => {
                    if feeder_ctl.generation.load(Ordering::SeqCst) != track_gen {
                        return;
                    }
                    let _ = msg_tx.send(EngineMsg::TrackReady {
                        generation: track_gen,
                        uri,
                        track,
                        path,
                        start_ms,
                        play,
                        same_uri_replay: same_uri,
                    });
                }
                Err(err) => {
                    if feeder_ctl.generation.load(Ordering::SeqCst) != track_gen {
                        return;
                    }
                    let _ = msg_tx.send(EngineMsg::TrackError {
                        generation: track_gen,
                        uri,
                        message: err.to_string(),
                    });
                }
            }
        });
    };

    while let Some(msg) = msg_rx.recv().await {
        match msg {
            EngineMsg::Shutdown => break,

            EngineMsg::TrackReady {
                generation,
                uri,
                track,
                path,
                start_ms,
                play,
                same_uri_replay,
            } => {
                if generation != load_gen {
                    continue;
                }
                tracks
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .insert(uri.clone(), track.clone());

                // Record listen in history if playing.
                if let Some(vid) = video_id_from_uri(&uri) {
                    let db_clone = Arc::clone(&db);
                    let vid = vid.to_string();
                    tokio::task::spawn_blocking(move || {
                        let _ = db_clone.record_listen(&vid);
                    });
                }

                feeder_ctl.paused.store(!play, Ordering::SeqCst);
                audio.track_changed();
                push_feeder(
                    &feeder_ctl,
                    FeederCmd::PlayFile {
                        generation,
                        path,
                        start_ms,
                        play,
                    },
                );

                {
                    let mut st = state.lock().unwrap_or_else(|p| p.into_inner());
                    st.track = Some(track);
                    st.loading = false;
                    st.error = None;
                    st.track_sequence = st.track_sequence.wrapping_add(1);
                    st.replay_pending = same_uri_replay;
                    st.position_ms = start_ms;
                    if play {
                        st.playback = Playback::Playing;
                        st.position_at = Some(Instant::now());
                    } else {
                        st.playback = Playback::Paused;
                        st.position_at = None;
                    }
                }
                emit_state(&state, &notify);
            }

            EngineMsg::TrackError {
                generation,
                uri: _,
                message,
            } => {
                if generation != load_gen {
                    continue;
                }
                audio.stopped();
                push_feeder(&feeder_ctl, FeederCmd::Stop);
                {
                    let mut st = state.lock().unwrap_or_else(|p| p.into_inner());
                    st.loading = false;
                    st.playback = Playback::Stopped;
                    st.position_at = None;
                    st.error = Some(message);
                }
                emit_state(&state, &notify);
            }

            EngineMsg::TrackEnded { generation } => {
                if generation != load_gen {
                    continue;
                }
                let repeat = state.lock().unwrap_or_else(|p| p.into_inner()).repeat;
                if repeat == RepeatMode::Track {
                    let current = queue
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .current_uri
                        .clone();
                    if let Some(uri) = current {
                        start_uri(
                            uri,
                            0,
                            true,
                            &mut load_gen,
                            &queue,
                            &state,
                            &tracks,
                            &audio,
                            &feeder_ctl,
                            &notify,
                        );
                        continue;
                    }
                }
                if let Some(next_uri) = advance_queue(&queue, repeat, true) {
                    start_uri(
                        next_uri,
                        0,
                        true,
                        &mut load_gen,
                        &queue,
                        &state,
                        &tracks,
                        &audio,
                        &feeder_ctl,
                        &notify,
                    );
                } else {
                    push_feeder(&feeder_ctl, FeederCmd::Stop);
                    {
                        let mut st = state.lock().unwrap_or_else(|p| p.into_inner());
                        st.loading = false;
                        st.playback = Playback::Stopped;
                        st.position_ms = 0;
                        st.position_at = None;
                    }
                    emit_state(&state, &notify);
                }
            }

            EngineMsg::Cmd(cmd) => match cmd {
                PlayerCommand::Toggle => {
                    let action = {
                        let mut st = state.lock().unwrap_or_else(|p| p.into_inner());
                        match st.playback {
                            Playback::Playing => {
                                st.position_ms = st.position_now();
                                st.position_at = None;
                                st.playback = Playback::Paused;
                                st.replay_pending = false;
                                Some(false)
                            }
                            Playback::Paused => {
                                st.position_at = Some(Instant::now());
                                st.playback = Playback::Playing;
                                st.replay_pending = false;
                                Some(true)
                            }
                            Playback::Stopped | Playback::Loading => None,
                        }
                    };
                    if let Some(play) = action {
                        feeder_ctl.paused.store(!play, Ordering::SeqCst);
                        push_feeder(
                            &feeder_ctl,
                            if play {
                                FeederCmd::Resume
                            } else {
                                FeederCmd::Pause
                            },
                        );
                        emit_state(&state, &notify);
                    } else if let Some(uri) = queue
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .current_uri
                        .clone()
                    {
                        start_uri(
                            uri,
                            0,
                            true,
                            &mut load_gen,
                            &queue,
                            &state,
                            &tracks,
                            &audio,
                            &feeder_ctl,
                            &notify,
                        );
                    }
                }

                PlayerCommand::Next => {
                    let repeat = state.lock().unwrap_or_else(|p| p.into_inner()).repeat;
                    if let Some(next_uri) = advance_queue(&queue, repeat, false) {
                        start_uri(
                            next_uri,
                            0,
                            true,
                            &mut load_gen,
                            &queue,
                            &state,
                            &tracks,
                            &audio,
                            &feeder_ctl,
                            &notify,
                        );
                    } else {
                        load_gen = load_gen.wrapping_add(1);
                        feeder_ctl.generation.store(load_gen, Ordering::SeqCst);
                        audio.interrupt();
                        audio.stopped();
                        push_feeder(&feeder_ctl, FeederCmd::Stop);
                        let mut st = state.lock().unwrap_or_else(|p| p.into_inner());
                        st.loading = false;
                        st.playback = Playback::Stopped;
                        st.position_ms = 0;
                        st.position_at = None;
                        drop(st);
                        emit_state(&state, &notify);
                    }
                }

                PlayerCommand::Previous => {
                    let pos_now = state.lock().unwrap_or_else(|p| p.into_inner()).position_now();
                    let cur_uri = queue
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .current_uri
                        .clone();
                    if pos_now > 3_000
                        && let Some(uri) = cur_uri
                    {
                        start_uri(
                            uri,
                            0,
                            true,
                            &mut load_gen,
                            &queue,
                            &state,
                            &tracks,
                            &audio,
                            &feeder_ctl,
                            &notify,
                        );
                        continue;
                    }
                    let prev = {
                        let mut q = queue.lock().unwrap_or_else(|p| p.into_inner());
                        if let Some(prev_uri) = q.played_history.pop() {
                            if let Some(cur) = q.current_uri.take() {
                                if !q.playing_manual {
                                    // Step back context cursor if appropriate.
                                    if q.cursor > 0 {
                                        q.cursor -= 1;
                                    }
                                } else {
                                    q.manual_queue.push_front(cur);
                                }
                            }
                            Some(prev_uri)
                        } else {
                            q.current_uri.clone()
                        }
                    };
                    if let Some(uri) = prev {
                        start_uri(
                            uri,
                            0,
                            true,
                            &mut load_gen,
                            &queue,
                            &state,
                            &tracks,
                            &audio,
                            &feeder_ctl,
                            &notify,
                        );
                    }
                }

                PlayerCommand::ClearQueue => {
                    queue
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .manual_queue
                        .clear();
                    emit_state(&state, &notify);
                }

                PlayerCommand::AddToQueue(uri) => {
                    queue
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .manual_queue
                        .push_back(uri);
                    emit_state(&state, &notify);
                }

                PlayerCommand::RemoveNextTrack { index, uri } => {
                    let mut q = queue.lock().unwrap_or_else(|p| p.into_inner());
                    if index < q.manual_queue.len() {
                        if q.manual_queue.get(index) == Some(&uri) {
                            q.manual_queue.remove(index);
                        }
                    } else {
                        let base = if q.playing_manual {
                            q.cursor
                        } else {
                            q.cursor.saturating_add(1)
                        };
                        let ctx_idx = base + (index - q.manual_queue.len());
                        if q.active_uris.get(ctx_idx) == Some(&uri) {
                            q.active_uris.remove(ctx_idx);
                        }
                    }
                    drop(q);
                    emit_state(&state, &notify);
                }

                PlayerCommand::Seek(position_ms) => {
                    let (current_uri, was_playing) = {
                        let mut st = state.lock().unwrap_or_else(|p| p.into_inner());
                        let playing = st.playback == Playback::Playing;
                        st.seek_sequence = st.seek_sequence.wrapping_add(1);
                        st.position_ms = position_ms;
                        st.position_at = playing.then(Instant::now);
                        (st.track.as_ref().map(|t| t.uri.clone()), playing)
                    };
                    if let Some(uri) = current_uri {
                        audio.seeked();
                        start_uri(
                            uri,
                            position_ms,
                            was_playing,
                            &mut load_gen,
                            &queue,
                            &state,
                            &tracks,
                            &audio,
                            &feeder_ctl,
                            &notify,
                        );
                    } else {
                        emit_state(&state, &notify);
                    }
                }

                PlayerCommand::Volume(volume) => {
                    volume_bits.store(volume_to_gain(volume).to_bits(), Ordering::SeqCst);
                    {
                        let mut st = state.lock().unwrap_or_else(|p| p.into_inner());
                        st.volume = volume;
                    }
                    emit_state(&state, &notify);
                }

                PlayerCommand::VolumePreview(volume) => {
                    volume_bits.store(volume_to_gain(volume).to_bits(), Ordering::SeqCst);
                    let mut st = state.lock().unwrap_or_else(|p| p.into_inner());
                    st.volume = volume;
                }

                PlayerCommand::Shuffle(shuffle) => {
                    {
                        let mut st = state.lock().unwrap_or_else(|p| p.into_inner());
                        st.shuffle = shuffle;
                    }
                    {
                        let mut q = queue.lock().unwrap_or_else(|p| p.into_inner());
                        q.apply_shuffle(shuffle);
                    }
                    emit_state(&state, &notify);
                }

                PlayerCommand::Repeat(repeat) => {
                    {
                        let mut st = state.lock().unwrap_or_else(|p| p.into_inner());
                        st.repeat = repeat;
                    }
                    emit_state(&state, &notify);
                }

                PlayerCommand::Transfer => {
                    emit_state(&state, &notify);
                }

                PlayerCommand::Load(spec) => {
                    let (shuffle, repeat) = {
                        let mut st = state.lock().unwrap_or_else(|p| p.into_inner());
                        if let Some(s) = spec.shuffle {
                            st.shuffle = s;
                        }
                        if let Some(r) = spec.repeat {
                            st.repeat = r;
                        }
                        (st.shuffle, st.repeat)
                    };

                    // Resolve tracks for context_uri if `spec.uris` is empty.
                    let mut uris = spec.uris.clone();
                    if uris.is_empty()
                        && let Some(ctx_uri) = spec.context_uri.as_deref()
                    {
                        uris = resolve_context_uris(&db, ctx_uri);
                    }
                    if uris.is_empty() {
                        if let Some(offset) = spec.offset_uri.clone() {
                            uris.push(offset);
                        }
                    }

                    if uris.is_empty() {
                        let mut st = state.lock().unwrap_or_else(|p| p.into_inner());
                        st.error = Some("Nothing playable in selection".into());
                        st.loading = false;
                        drop(st);
                        emit_state(&state, &notify);
                        continue;
                    }

                    // Determine initial track URI before shuffling so offset picks the right starting track.
                    let start_index = if let Some(uri) = &spec.offset_uri {
                        uris.iter().position(|u| u == uri).unwrap_or(0)
                    } else if let Some(idx) = spec.offset_index {
                        (idx as usize).min(uris.len().saturating_sub(1))
                    } else {
                        0
                    };
                    let chosen_uri = uris[start_index].clone();

                    {
                        let mut q = queue.lock().unwrap_or_else(|p| p.into_inner());
                        q.context_uri = spec.context_uri.clone();
                        q.original_uris = uris;
                        q.current_uri = Some(chosen_uri.clone());
                        q.playing_manual = false;
                        q.manual_queue.clear();
                        q.played_history.clear();
                        q.apply_shuffle(shuffle);
                        if !shuffle {
                            q.cursor = start_index;
                        }
                    }
                    let _ = repeat;

                    start_uri(
                        chosen_uri,
                        spec.position_ms,
                        spec.play,
                        &mut load_gen,
                        &queue,
                        &state,
                        &tracks,
                        &audio,
                        &feeder_ctl,
                        &notify,
                    );
                }
            },
        }
    }
}

fn advance_queue(queue: &Arc<Mutex<QueueState>>, repeat: RepeatMode, natural_end: bool) -> Option<String> {
    let mut q = queue.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(next_manual) = q.manual_queue.pop_front() {
        q.playing_manual = true;
        return Some(next_manual);
    }
    if q.active_uris.is_empty() {
        return None;
    }
    let next_idx = if q.playing_manual {
        q.cursor
    } else {
        q.cursor.saturating_add(1)
    };
    q.playing_manual = false;
    if next_idx < q.active_uris.len() {
        q.cursor = next_idx;
        return Some(q.active_uris[next_idx].clone());
    }
    if repeat == RepeatMode::Context || (!natural_end && repeat == RepeatMode::Track) {
        q.cursor = 0;
        return q.active_uris.first().cloned();
    }
    None
}

fn resolve_context_uris(db: &SearchCache, context_uri: &str) -> Vec<String> {
    if context_uri.ends_with(":collection") || context_uri == "spotify:collection:tracks" {
        if let Ok((liked, _)) = db.liked_tracks(0, 500) {
            return liked
                .into_iter()
                .map(|t| format!("spotify:track:{}", t.id))
                .collect();
        }
    }
    if let Some(pl_id_str) = context_uri.strip_prefix("spotify:playlist:")
        && let Ok(pl_id) = pl_id_str.parse::<i64>()
        && let Ok(tracks) = db.get_playlist_tracks(pl_id)
    {
        return tracks
            .into_iter()
            .map(|t| format!("spotify:track:{}", t.id))
            .collect();
    }
    Vec::new()
}

pub fn video_id_from_uri(uri: &str) -> Option<&str> {
    uri.strip_prefix("spotify:track:")
        .or_else(|| uri.strip_prefix("youtube:video:"))
}

fn local_track_from_yt(t: &YtTrack) -> LocalTrack {
    let artist_id = encode_artist_id(&t.artist);
    LocalTrack {
        uri: format!("spotify:track:{}", t.id),
        title: t.title.clone(),
        artists: vec![ArtistRef {
            id: Some(artist_id.clone()),
            name: t.artist.clone(),
            uri: Some(format!("spotify:artist:{artist_id}")),
        }],
        album: if t.artist.is_empty() {
            "YouTube".to_string()
        } else {
            t.artist.clone()
        },
        art_url: (!t.thumbnail.is_empty()).then(|| t.thumbnail.clone()),
        art_small_url: (!t.thumbnail.is_empty()).then(|| t.thumbnail.clone()),
        duration_ms: (t.duration_secs.max(0.0) * 1000.0).round() as u32,
        is_episode: false,
    }
}

async fn prepare_audio_track(
    uri: &str,
    db: &Arc<SearchCache>,
    extractor: &Arc<Extractor>,
    cache_dir: &Path,
    state_dir: &Path,
    cache_limit: Option<u64>,
    track_gen: u64,
) -> Result<(LocalTrack, PathBuf)> {
    let video_id = video_id_from_uri(uri)
        .ok_or_else(|| anyhow!("Unsupported URI for YouTube playback: {uri}"))?
        .to_string();

    // Look up metadata in SQLite, fetching from YouTube if unknown.
    let db_clone = Arc::clone(db);
    let vid_clone = video_id.clone();
    let existing = tokio::task::spawn_blocking(move || db_clone.get_track_by_id(&vid_clone))
        .await
        .ok()
        .and_then(|r| r.ok())
        .flatten();

    let yt_track = match existing {
        Some(t) => t,
        None => {
            let meta = extractor.metadata(&video_id).await.map_err(|e| anyhow!("{e}"))?;
            let db_clone = Arc::clone(db);
            let meta_clone = meta.clone();
            tokio::task::spawn_blocking(move || {
                let _ = db_clone.upsert_tracks(&[meta_clone]);
            })
            .await
            .ok();
            meta
        }
    };
    let local_track = local_track_from_yt(&yt_track);

    // 1. Check offline downloaded MP3.
    let dl_path = DownloadManager::new(state_dir).path_for(&video_id);
    if dl_path.exists() {
        return Ok((local_track, dl_path));
    }

    // 2. Check cached audio in cache_dir.
    for ext in ["mp3", "m4a", "flac", "wav", "ogg"] {
        let candidate = cache_dir.join(format!("{video_id}.{ext}"));
        if candidate.exists() {
            return Ok((local_track, candidate));
        }
    }

    // 3. Download to temporary staging dir and move atomically into cache_dir.
    tokio::fs::create_dir_all(cache_dir).await?;
    let tmp_dir = cache_dir.join(format!(".tmp-{track_gen}"));
    tokio::fs::create_dir_all(&tmp_dir).await?;

    let managed = ytdlp::managed_bin_path(state_dir);
    let bin = ytdlp::resolve_bin(Some(&managed));
    let out_tmpl = tmp_dir
        .join(format!("{video_id}.%(ext)s"))
        .to_string_lossy()
        .into_owned();
    let url = format!("https://www.youtube.com/watch?v={video_id}");

    let mut cmd = tokio::process::Command::new(&bin);
    cmd.no_window();
    cmd.args([
        "--no-playlist",
        "--no-warnings",
        "--extract-audio",
        "--audio-format",
        "mp3",
        "--audio-quality",
        "0",
        "-o",
        &out_tmpl,
    ]);
    cmd.args(cookies::current_args());
    cmd.arg(&url);

    let output = tokio::time::timeout(Duration::from_secs(90), cmd.output())
        .await
        .context("yt-dlp timed out downloading audio")?
        .with_context(|| format!("failed to spawn {bin}"))?;

    if !output.status.success() {
        let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
        let stderr = String::from_utf8_lossy(&output.stderr);
        let msg = stderr
            .lines()
            .rfind(|l| !l.trim().is_empty())
            .unwrap_or("yt-dlp failed to download audio")
            .to_string();
        return Err(anyhow!(msg));
    }

    let final_path = cache_dir.join(format!("{video_id}.mp3"));
    let staged_mp3 = tmp_dir.join(format!("{video_id}.mp3"));
    if staged_mp3.exists() {
        let _ = tokio::fs::rename(&staged_mp3, &final_path).await;
    } else if let Ok(mut entries) = tokio::fs::read_dir(&tmp_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let p = entry.path();
            if p.is_file() {
                let target = cache_dir.join(
                    p.file_name()
                        .unwrap_or_else(|| std::ffi::OsStr::new("audio.mp3")),
                );
                let _ = tokio::fs::rename(&p, &target).await;
                let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
                return Ok((local_track, target));
            }
        }
    }
    let _ = tokio::fs::remove_dir_all(&tmp_dir).await;

    if !final_path.exists() {
        return Err(anyhow!("Audio file missing after yt-dlp finished"));
    }

    if let Some(limit) = cache_limit {
        let dir = cache_dir.to_path_buf();
        let keep = final_path.clone();
        tokio::task::spawn_blocking(move || prune_cache_dir(&dir, &keep, limit));
    }

    Ok((local_track, final_path))
}

fn prune_cache_dir(dir: &Path, keep: &Path, limit: u64) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files = Vec::new();
    let mut total: u64 = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if let Ok(meta) = entry.metadata() {
            let mtime = meta.modified().ok();
            total = total.saturating_add(meta.len());
            files.push((path, meta.len(), mtime));
        }
    }
    if total <= limit {
        return;
    }
    files.sort_by_key(|(_, _, mtime)| *mtime);
    for (path, len, _) in files {
        if total <= limit {
            break;
        }
        if path == keep {
            continue;
        }
        if std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(len);
        }
    }
}

fn spawn_audio_feeder(
    config: EngineConfig,
    audio: Arc<AudioControl>,
    volume_bits: Arc<AtomicU32>,
    ctl: Arc<FeederControl>,
    msg_tx: mpsc::UnboundedSender<EngineMsg>,
    notify: Notify,
) {
    let on_error = {
        let notify = notify.clone();
        Arc::new(move |message: String| {
            log::warn!("Audio output error: {message}");
            let _ = &notify;
        })
    };
    let on_device = {
        let notify = notify.clone();
        Arc::new(move |name: String, id: fastframe_audio::DeviceId| {
            notify(EngineEvent::AudioDevice {
                name,
                id: id.as_str().to_string(),
            });
        })
    };

    let normalisation = Arc::new(AtomicU64::new(1.0f64.to_bits()));

    let _ = std::thread::Builder::new()
        .name("resonix-audio-feeder".into())
        .spawn(move || {
            let sink = RodioSink::new(
                config.audio_device,
                on_error,
                Box::new(AtomicVolume(Arc::clone(&volume_bits))),
                config.buffer_ms,
                Arc::clone(&audio),
            )
            .on_device(on_device);

            let mut tapped = Tapped::new(
                Box::new(sink),
                Arc::clone(&audio),
                config.tap,
                Box::new(AtomicVolume(volume_bits)),
                false,
                config.eq,
                normalisation,
            );

            struct ActiveDecoder {
                generation: u64,
                decoder: Decoder<BufReader<File>>,
                channels: u16,
                resampler: Option<Resampler>,
                playing: bool,
            }

            let mut active: Option<ActiveDecoder> = None;

            loop {
                if ctl.shutdown.load(Ordering::SeqCst) {
                    let _ = tapped.stop();
                    break;
                }

                // Drain queued commands.
                let cmd = {
                    let mut inbox = ctl.inbox.lock().unwrap_or_else(|p| p.into_inner());
                    inbox.pop_front()
                };

                if let Some(cmd) = cmd {
                    match cmd {
                        FeederCmd::Shutdown => {
                            let _ = tapped.stop();
                            break;
                        }
                        FeederCmd::Stop => {
                            active = None;
                            let _ = tapped.stop();
                        }
                        FeederCmd::Pause => {
                            if let Some(act) = &mut active {
                                act.playing = false;
                            }
                            let _ = tapped.stop();
                        }
                        FeederCmd::Resume => {
                            if let Some(act) = &mut active {
                                act.playing = true;
                                let _ = tapped.start();
                            }
                        }
                        FeederCmd::PlayFile {
                            generation,
                            path,
                            start_ms,
                            play,
                        } => {
                            if ctl.generation.load(Ordering::SeqCst) != generation {
                                continue;
                            }
                            match File::open(&path).and_then(|f| {
                                Decoder::new(BufReader::new(f)).map_err(std::io::Error::other)
                            }) {
                                Ok(mut dec) => {
                                    if start_ms > 0 {
                                        let _ = dec.try_seek(Duration::from_millis(u64::from(
                                            start_ms,
                                        )));
                                    }
                                    let src_rate = dec.sample_rate();
                                    let channels = dec.channels();
                                    let resampler = Resampler::new(
                                        src_rate,
                                        SAMPLE_RATE,
                                        NUM_CHANNELS as usize,
                                    );
                                    if play {
                                        let _ = tapped.start();
                                    } else {
                                        let _ = tapped.stop();
                                    }
                                    active = Some(ActiveDecoder {
                                        generation,
                                        decoder: dec,
                                        channels,
                                        resampler,
                                        playing: play,
                                    });
                                }
                                Err(err) => {
                                    active = None;
                                    let _ = msg_tx.send(EngineMsg::TrackError {
                                        generation,
                                        uri: String::new(),
                                        message: format!("Failed to open audio file: {err}"),
                                    });
                                }
                            }
                        }
                    }
                    continue;
                }

                // If nothing active or paused, sleep until woken.
                let Some(act) = &mut active else {
                    let guard = ctl.inbox.lock().unwrap_or_else(|p| p.into_inner());
                    if guard.is_empty() && !ctl.shutdown.load(Ordering::SeqCst) {
                        let _unused = ctl.wake.wait_timeout(guard, Duration::from_millis(200));
                    }
                    continue;
                };

                if !act.playing || ctl.paused.load(Ordering::SeqCst) {
                    let guard = ctl.inbox.lock().unwrap_or_else(|p| p.into_inner());
                    if guard.is_empty() && !ctl.shutdown.load(Ordering::SeqCst) {
                        let _unused = ctl.wake.wait_timeout(guard, Duration::from_millis(50));
                    }
                    continue;
                }

                // Read 1024 stereo frames (~23 ms at 44.1 kHz) and write to Tapped -> RodioSink.
                const CHUNK_FRAMES: usize = 1024;
                let mut stereo_f32 = Vec::with_capacity(CHUNK_FRAMES * 2);
                let src_ch = usize::from(act.channels.max(1));
                let mut eof = false;

                for _ in 0..CHUNK_FRAMES {
                    match src_ch {
                        1 => {
                            if let Some(s) = act.decoder.next() {
                                stereo_f32.push(s);
                                stereo_f32.push(s);
                            } else {
                                eof = true;
                                break;
                            }
                        }
                        2 => {
                            if let (Some(l), Some(r)) = (act.decoder.next(), act.decoder.next()) {
                                stereo_f32.push(l);
                                stereo_f32.push(r);
                            } else {
                                eof = true;
                                break;
                            }
                        }
                        n => {
                            let Some(l) = act.decoder.next() else {
                                eof = true;
                                break;
                            };
                            let Some(r) = act.decoder.next() else {
                                eof = true;
                                break;
                            };
                            for _ in 2..n {
                                let _ = act.decoder.next();
                            }
                            stereo_f32.push(l);
                            stereo_f32.push(r);
                        }
                    }
                }

                if !stereo_f32.is_empty() {
                    let resampled = match &mut act.resampler {
                        Some(r) => r.process(&stereo_f32),
                        None => stereo_f32,
                    };
                    let f64_samples: Vec<f64> =
                        resampled.into_iter().map(f64::from).collect();
                    if let Err(err) = tapped.write(AudioPacket::Samples(f64_samples)) {
                        let track_gen = act.generation;
                        active = None;
                        let _ = tapped.stop();
                        let _ = msg_tx.send(EngineMsg::TrackError {
                            generation: track_gen,
                            uri: String::new(),
                            message: err.to_string(),
                        });
                        continue;
                    }
                }

                if eof {
                    let track_gen = act.generation;
                    active = None;
                    let _ = tapped.stop();
                    let _ = msg_tx.send(EngineMsg::TrackEnded { generation: track_gen });
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rootlist_parses_folders_and_playlists() {
        let uris = vec![
            "spotify:start-group:f1:My+Folder".to_string(),
            "spotify:playlist:p1".to_string(),
            "spotify:end-group:f1".to_string(),
            "spotify:playlist:p2".to_string(),
        ];
        assert_eq!(
            parse_rootlist(&uris),
            vec![
                RootlistEntry::FolderStart {
                    id: "f1".into(),
                    name: "My Folder".into(),
                },
                RootlistEntry::Playlist("spotify:playlist:p1".into()),
                RootlistEntry::FolderEnd,
                RootlistEntry::Playlist("spotify:playlist:p2".into()),
            ]
        );
    }

    #[test]
    fn repeat_mode_cycles() {
        assert_eq!(RepeatMode::Off.next(), RepeatMode::Context);
        assert_eq!(RepeatMode::Context.next(), RepeatMode::Track);
        assert_eq!(RepeatMode::Track.next(), RepeatMode::Off);
    }
}
