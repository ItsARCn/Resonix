//! Bridge between the UI thread and asynchronous work (YouTube / SQLite / Rodio).
//!
//! egui runs on the main thread and must never block. A dedicated tokio
//! runtime hosts Sunder's YouTube extractor, SQLite library cache, the
//! [`Engine`] audio player, and artwork fetches; the two sides talk through
//! channels.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::api::models::*;
use crate::api::{AccountId, ApiError, ApiSource, NetActivity, PlayRequest};
use crate::credentials::{
    Lease as CredentialLease, Slot as CredentialSlot, Store as CredentialStore,
};
use crate::http::Http;
use crate::images::{ArtLoader, art_palette};
use crate::model::{PlaylistCache, TopPeriod};
use crate::paths::AppDirs;
use crate::player::{
    Engine, EngineConfig, EngineEvent, LocalState, Playback, PlayerCommand, RepeatMode, Rootlist,
    RootlistEntry, video_id_from_uri,
};
use crate::session_reads;
use crate::settings::ProxyConfig;
use crate::youtube::models::{
    artist_from_name, decode_artist_id, encode_artist_id, thumbnail_images, to_playlist_item,
    to_spotify_playlist, to_spotify_track,
};
use crate::youtube::{DownloadManager, Extractor, SearchCache, YtTrack, explore};

pub type ApiResult<T> = Result<T, ApiError>;

pub const PLAYLIST_PAGE_SIZE: u32 = 50;

#[derive(Clone, Debug, PartialEq)]
pub enum AuthStatus {
    Starting,
    SignedOut,
    WaitingForBrowser { url: String },
    Connecting,
    Connected { username: String },
    Failed(String),
    Refused(SignInProblem),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignInProblem {
    MalformedClientId,
    UnknownClient,
    AccountNotAllowed,
    SharedRetired,
    Expired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteAction {
    Play,
    Pause,
    Next,
    Previous,
    Seek,
    Volume { request: u64 },
    Shuffle,
    Repeat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecentsFor {
    Home,
    Panel,
}

#[derive(Clone, Debug)]
pub enum ApiRequest {
    Me,
    Devices,
    PlaybackState {
        seq: u64,
    },
    Queue {
        seq: u64,
    },
    RecentlyPlayed {
        who: RecentsFor,
        generation: u64,
        before: Option<String>,
        limit: u32,
    },
    TopTracks {
        period: TopPeriod,
        offset: u32,
        full: bool,
        generation: u64,
    },
    TopArtists {
        generation: u64,
    },
    Recommendations {
        seed_tracks: Vec<String>,
        seed_artists: Vec<String>,
        generation: u64,
    },
    MadeForYou {
        generation: u64,
        force: bool,
    },
    Downloads,
    NewReleases {
        generation: u64,
    },
    NewReleaseTracks {
        albums: Vec<Album>,
        generation: u64,
    },
    HiddenSongs {
        generation: u64,
    },
    SetHiddenInPlaylist {
        playlist_uri: String,
        track_uri: String,
        hidden: bool,
    },
    MyPlaylists {
        offset: u32,
        generation: u64,
    },
    Playlist {
        id: String,
        generation: u64,
    },
    PlaylistItems {
        id: String,
        offset: u32,
        generation: u64,
    },
    PlaylistSample {
        id: String,
        offset: u32,
        generation: u64,
    },
    CreatePlaylist {
        name: String,
        public: bool,
        description: String,
    },
    UploadPlaylistCover {
        id: String,
        request: u64,
        previous_urls: Vec<String>,
        cover: crate::playlist_cover::Cover,
    },
    UpdatePlaylist {
        id: String,
        name: Option<String>,
        description: Option<String>,
        public: Option<bool>,
    },
    CheckPlaylistDuplicates {
        playlist_id: String,
        playlist_name: String,
        items: Vec<PlayableItem>,
        position: Option<u32>,
    },
    AddToPlaylist {
        playlist_id: String,
        playlist_name: String,
        uris: Vec<String>,
        position: Option<u32>,
    },
    RemoveFromPlaylist {
        playlist_id: String,
        uris: Vec<String>,
        snapshot_id: Option<String>,
    },
    ReorderPlaylist {
        playlist_id: String,
        range_start: u32,
        insert_before: u32,
        snapshot_id: Option<String>,
    },
    FollowPlaylist {
        id: String,
        follow: bool,
    },
    SavedTracks {
        offset: u32,
        generation: u64,
    },
    SavedAlbums {
        offset: u32,
    },
    FollowedArtists {
        after: Option<String>,
    },
    SavedShows {
        offset: u32,
    },
    SavedEpisodes {
        offset: u32,
    },
    SetSaved {
        uris: Vec<String>,
        saved: bool,
    },
    Contains {
        uris: Vec<String>,
    },
    Search {
        query: String,
        serial: u64,
    },
    SearchCatalogue {
        query: String,
        serial: u64,
    },
    SearchPlaylists {
        query: String,
        serial: u64,
    },
    PlaySearch {
        kind: String,
        query: String,
        serial: u64,
    },
    Artist {
        id: String,
    },
    ArtistTopTracks {
        id: String,
        generation: u64,
    },
    ArtistAlbums {
        id: String,
        groups: String,
        offset: u32,
    },
    RelatedArtists {
        id: String,
    },
    Album {
        id: String,
    },
    AlbumTracks {
        id: String,
        offset: u32,
        generation: u64,
    },
    AlbumQueueTracks {
        id: String,
        offset: u32,
        request: u64,
    },
    Show {
        id: String,
    },
    ShowEpisodes {
        id: String,
        offset: u32,
    },
    HomeEpisodes {
        shows: Vec<Show>,
        generation: u64,
    },
    Track {
        id: String,
    },
    Episode {
        id: String,
    },
    Remote {
        action: RemoteAction,
        device_id: Option<String>,
        play: Option<PlayRequest>,
        position_ms: u32,
        percent: u8,
        flag: bool,
        repeat: String,
    },
    Transfer {
        device_id: String,
        play: bool,
    },
    ShufflePlay {
        device_id: Option<String>,
        play: PlayRequest,
    },
    AddToQueue {
        uri: String,
        device_id: Option<String>,
        label: String,
    },
    AddManyToQueue {
        request: u64,
        uris: Vec<String>,
        device_id: Option<String>,
    },
}

#[derive(Debug)]
pub enum ApiResponse {
    Me(ApiResult<User>),
    Devices(ApiResult<Vec<Device>>),
    PlaybackState {
        seq: u64,
        result: ApiResult<Option<PlaybackState>>,
    },
    Queue {
        seq: u64,
        result: ApiResult<Queue>,
    },
    RecentlyPlayed {
        who: RecentsFor,
        generation: u64,
        limit: u32,
        result: ApiResult<CursorPage<PlayHistory>>,
    },
    TopTracks {
        period: TopPeriod,
        offset: u32,
        full: bool,
        generation: u64,
        result: ApiResult<Page<Track>>,
    },
    TopArtists {
        generation: u64,
        result: ApiResult<Vec<Artist>>,
    },
    Recommendations {
        generation: u64,
        result: ApiResult<Vec<Track>>,
    },
    MadeForYou {
        generation: u64,
        personalized: bool,
        result: ApiResult<Vec<Playlist>>,
        top_mixes: ApiResult<Vec<Playlist>>,
    },
    Downloads(ApiResult<crate::model::DownloadsLibrary>),
    NewReleases {
        generation: u64,
        result: ApiResult<Vec<Album>>,
    },
    NewReleaseTracks {
        generation: u64,
        result: ApiResult<Vec<Track>>,
    },
    HiddenSongs {
        generation: u64,
        result: ApiResult<Vec<(String, String)>>,
    },
    HiddenInPlaylistSet {
        playlist_uri: String,
        track_uri: String,
        hidden: bool,
        result: ApiResult<()>,
    },
    MyPlaylists {
        offset: u32,
        generation: u64,
        result: ApiResult<Page<Playlist>>,
    },
    Playlist {
        id: String,
        generation: u64,
        result: ApiResult<Playlist>,
    },
    PlaylistItems {
        id: String,
        offset: u32,
        generation: u64,
        result: ApiResult<Page<PlaylistItem>>,
    },
    PlaylistSample {
        id: String,
        generation: u64,
        result: ApiResult<Page<PlaylistItem>>,
    },
    PlaylistCreated(ApiResult<Playlist>),
    PlaylistCoverUploaded {
        id: String,
        request: u64,
        previous_urls: Vec<String>,
        cover: crate::playlist_cover::Cover,
        result: ApiResult<()>,
    },
    PlaylistUpdated {
        id: String,
        result: ApiResult<()>,
    },
    PlaylistDuplicatesChecked {
        playlist_id: String,
        playlist_name: String,
        items: Vec<PlayableItem>,
        position: Option<u32>,
        result: ApiResult<Vec<String>>,
    },
    PlaylistItemsChanged {
        id: String,
        message: String,
        result: ApiResult<Option<String>>,
    },
    PlaylistFollowChanged {
        id: String,
        followed: bool,
        result: ApiResult<()>,
    },
    SavedTracks {
        offset: u32,
        generation: u64,
        account_id: Option<String>,
        result: ApiResult<Page<SavedTrack>>,
    },
    SavedAlbums {
        offset: u32,
        result: ApiResult<Page<SavedAlbum>>,
    },
    FollowedArtists {
        after: Option<String>,
        result: ApiResult<CursorPage<Artist>>,
    },
    SavedShows {
        offset: u32,
        result: ApiResult<Page<SavedShow>>,
    },
    SavedEpisodes {
        offset: u32,
        result: ApiResult<Page<SavedEpisode>>,
    },
    SavedChanged {
        uris: Vec<String>,
        saved: bool,
        result: ApiResult<()>,
    },
    Contains {
        uris: Vec<String>,
        result: ApiResult<Vec<bool>>,
    },
    SearchStarted {
        query: String,
        serial: u64,
        split: bool,
    },
    Search {
        query: String,
        serial: u64,
        result: ApiResult<SearchResults>,
    },
    SearchPlaylists {
        query: String,
        serial: u64,
        result: ApiResult<Page<Playlist>>,
    },
    PlaySearch {
        query: String,
        serial: u64,
        result: ApiResult<Option<String>>,
    },
    Artist {
        id: String,
        result: ApiResult<Artist>,
    },
    ArtistTopTracks {
        id: String,
        generation: u64,
        result: ApiResult<Vec<Track>>,
    },
    ArtistAlbums {
        id: String,
        groups: String,
        offset: u32,
        result: ApiResult<Page<Album>>,
    },
    RelatedArtists {
        id: String,
        result: ApiResult<Vec<Artist>>,
    },
    Album {
        id: String,
        result: ApiResult<Album>,
    },
    AlbumTracks {
        id: String,
        offset: u32,
        generation: u64,
        result: ApiResult<Page<Track>>,
    },
    AlbumQueueTracks {
        offset: u32,
        request: u64,
        result: ApiResult<Page<Track>>,
    },
    Show {
        id: String,
        result: ApiResult<Show>,
    },
    ShowEpisodes {
        id: String,
        offset: u32,
        result: ApiResult<Page<Episode>>,
    },
    HomeEpisodes {
        generation: u64,
        result: ApiResult<Vec<(Show, Vec<Episode>)>>,
    },
    Track {
        id: String,
        result: ApiResult<Track>,
    },
    Episode {
        id: String,
        result: ApiResult<Episode>,
    },
    Remote {
        action: RemoteAction,
        result: ApiResult<()>,
    },
    Transferred {
        device_id: String,
        result: ApiResult<()>,
    },
    QueueAdded {
        label: String,
        result: ApiResult<()>,
    },
    QueueBatchAdded {
        request: u64,
        added: usize,
        result: ApiResult<()>,
    },
}

pub enum PlaylistCacheRows {
    Replace(Vec<PlaylistItem>),
    Append {
        previous_rows: usize,
        previous_offset: u32,
        items: Vec<PlaylistItem>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutostartRequest {
    Read,
    Set(Option<fastframe_autostart::Start>),
}

pub fn autostart() -> fastframe_autostart::Autostart {
    fastframe_autostart::Autostart::new("spotifast", "Spotifast")
}

pub enum Command {
    Soloist {
        enabled: bool,
        key: Option<String>,
    },
    SoloistDevice(Option<crate::api::models::Device>),
    SoloistUpdate {
        generation: u64,
        update: crate::soloist::Update,
    },
    OpenThemesFolder,
    Autostart(AutostartRequest),
    ListOutputDevices,
    ProxyRestored {
        lease: CredentialLease,
        result: Result<crate::credentials::Loaded, crate::credentials::Error>,
    },
    CredentialsRestored {
        slot: CredentialSlot,
        lease: CredentialLease,
        result: Result<crate::credentials::Loaded, crate::credentials::Error>,
    },
    CheckPlaylistCover {
        id: String,
        request: u64,
        cover: crate::playlist_cover::Cover,
        images: Vec<crate::api::models::Image>,
    },
    ChoosePlaylistCover {
        id: String,
        request: u64,
        selected:
            std::pin::Pin<Box<dyn std::future::Future<Output = Option<rfd::FileHandle>> + Send>>,
    },
    ChooseLocalFilesFolder {
        selected:
            std::pin::Pin<Box<dyn std::future::Future<Output = Option<rfd::FileHandle>> + Send>>,
    },
    ScanLocalFiles(std::path::PathBuf),
    SignIn {
        request: u64,
        config: ProxyConfig,
        client_id: Option<String>,
    },
    CancelSignIn,
    SignOut,
    AuthorizePlayback,
    RestartEngine(EngineConfig),
    ClearAudioCache,
    AudioCacheClearFinished(Result<crate::storage::AudioCleared, crate::storage::ClearAudioError>),
    ApplyProxy {
        request: u64,
        config: ProxyConfig,
    },
    Player(PlayerCommand),
    Api(ApiRequest),
    ApiFinished {
        generation: u64,
        home_account: Option<AccountId>,
        response: Box<ApiResponse>,
        expired: Vec<ApiSource>,
        shared_lease: CredentialLease,
        personal_lease: CredentialLease,
    },
    Accent {
        url: String,
    },
    Shutdown,
    NetworkStatus(bool),
    DiscoverReceivers,
    ActivateReceiver(Box<crate::zeroconf::Receiver>),
    CheckForUpdates {
        manual: bool,
        source: crate::updates::Source,
    },
    DownloadUpdate {
        release: crate::updates::Release,
        source: crate::updates::Source,
    },
    InstallUpdate {
        prepared: Box<crate::updates::Prepared>,
        arguments: Vec<String>,
    },
    Lyrics(Box<LyricsRequest>),
    Rootlist,
    Reconnect,
    VerifyResume,
    ConfigurePersonalWebApp(Option<String>),
    LoadPlaylistCache {
        id: String,
        generation: u64,
    },
    StorePlaylistCache {
        id: String,
        generation: u64,
        snapshot: String,
        rows: PlaylistCacheRows,
        total: u32,
        next_offset: Option<u32>,
    },
    UserNames(Vec<String>),
    PlayCounts {
        uri: String,
        generation: u64,
    },
    FriendActivity(u64),
    LoadLikedSongsCache {
        generation: u64,
    },
    StoreLikedSongsCache(crate::liked::Cache),
    LoadAccountCaches {
        generation: u64,
    },
    StoreLibraryCache(crate::account_cache::Cache<Vec<Playlist>>),
    StoreHomeCache(crate::account_cache::Cache<crate::model::HomeSnapshot>),
    AlbumTypes(Vec<String>),
    AudiobookShows(Vec<String>),
    PlaylistHeaders(Vec<String>),
    Radio {
        seed: String,
        generation: u64,
    },
    SetPrereleaseSaved {
        uri: String,
        saved: bool,
        generation: u64,
    },
    RemovePlaylistRows {
        playlist_id: String,
        rows: Vec<session_reads::RemovedRow>,
    },
    Prerelease {
        uri: String,
        generation: u64,
    },
    DownloadTrack(Track),
    DownloadUris {
        collection_uri: Option<String>,
        label: String,
        resolved_tracks: Vec<Track>,
        uris: Vec<String>,
    },
    DownloadContext {
        uri: String,
        label: String,
    },
    DeleteDownload(String),
    DeleteDownloadCollection(String),
}

pub struct LyricsRequest {
    pub uri: String,
    pub query: crate::lyrics::Query,
    pub local_file: Option<std::path::PathBuf>,
}

pub enum Event {
    Soloist(crate::soloist::Update),
    AudioCacheCleared(Result<crate::storage::AudioCleared, crate::storage::ClearAudioError>),
    LocalFilesFolderChosen(Option<std::path::PathBuf>),
    LocalFiles {
        folder: std::path::PathBuf,
        uris: Arc<HashMap<String, std::path::PathBuf>>,
    },
    ProxyRestored {
        config: ProxyConfig,
        password: Option<crate::credentials::ProxyPassword>,
    },
    ProxyPasswordStored,
    ProxyStorageFailed(crate::credentials::Error),
    ProxyApplied {
        request: u64,
        config: ProxyConfig,
        result: Result<bool, String>,
    },
    UpdateSupport(crate::updates::Support),
    UpdateProgress {
        received: u64,
        total: u64,
    },
    UpdateDownloaded(Result<Box<crate::updates::Prepared>, String>),
    UpdateInstalling(Result<(), String>),
    PlaylistCoverChecked {
        id: String,
        request: u64,
        images: Vec<crate::api::models::Image>,
        result: Result<bool, String>,
    },
    PlaylistCoverChosen {
        id: String,
        request: u64,
        result: Result<Option<crate::playlist_cover::Cover>, String>,
    },
    Auth(AuthStatus),
    Playback(LocalPlayback),
    Receivers(Vec<crate::zeroconf::Receiver>),
    OutputDevices(Result<Vec<fastframe_audio::DeviceInfo>, String>),
    AudioDeviceFound {
        name: String,
        id: String,
    },
    ReceiversProgress(crate::zeroconf::ReceiverUpdates),
    Autostart {
        request: AutostartRequest,
        result: Result<fastframe_autostart::State, String>,
    },
    ReceiverActivated {
        name: String,
        result: Result<(), String>,
    },
    Local(Box<LocalState>),
    RemoteChanged,
    RemoteVolume {
        device_id: String,
        volume: u16,
    },
    Api(Box<ApiResponse>),
    Accent {
        url: String,
        color: [u8; 3],
        palette: Vec<[u8; 3]>,
    },
    Error(String),
    UpdateChecked {
        manual: bool,
        result: Result<Option<crate::updates::Release>, String>,
    },
    Lyrics {
        uri: String,
        result: Result<Option<crate::lyrics::Lyrics>, String>,
    },
    Rootlist {
        result: Result<crate::player::Rootlist, String>,
    },
    PlaylistCache {
        account_id: String,
        id: String,
        generation: u64,
        cache: Option<PlaylistCache>,
    },
    PlaylistCacheStored {
        account_id: String,
        id: String,
        generation: u64,
        snapshot: String,
        success: bool,
    },
    FriendActivity {
        generation: u64,
        result: Result<Vec<session_reads::FriendActivity>, String>,
    },
    UserName {
        id: String,
        name: Option<String>,
        image: Option<String>,
    },
    PlayCounts {
        uri: String,
        generation: u64,
        result: Result<session_reads::PageMetadata, String>,
    },
    AudiobookShows(Vec<String>),
    PlaylistHeader(Box<crate::api::models::Playlist>),
    Radio {
        seed: String,
        generation: u64,
        result: Result<Vec<crate::api::models::Track>, String>,
    },
    Prerelease {
        uri: String,
        generation: u64,
        result: Result<Box<crate::prerelease::Prerelease>, String>,
    },
    PrereleaseSaved {
        uri: String,
        saved: bool,
        generation: u64,
        result: Result<(), String>,
    },
    DownloadStarted {
        track_ids: Vec<String>,
    },
    DownloadFinished {
        track_id: String,
        title: String,
        result: Result<(), String>,
    },
    AlbumType {
        uri: String,
        result: Result<bool, String>,
    },
    WebApp {
        client_id: Option<String>,
    },
    SharedGrant(bool),
    LikedSongsCache {
        account_id: String,
        generation: u64,
        cache: Option<crate::liked::Cache>,
    },
    NetworkStatus(bool),
    AccountCaches {
        account_id: String,
        generation: u64,
        playlists: Option<Vec<Playlist>>,
        home: Option<crate::model::HomeSnapshot>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum LocalPlayback {
    Unavailable,
    Authorizing,
    Connecting,
    WaitingForNetwork,
    Ready { device_id: String },
    Failed(String),
}

pub use fastframe_shell::Waker;

pub struct Backend {
    pub soloist_output_volume: crate::soloist::OutputVolume,
    commands: mpsc::UnboundedSender<Command>,
    events: std::sync::mpsc::Receiver<Event>,
    art: ArtLoader,
    activity: Arc<NetActivity>,
    thread: Option<std::thread::JoinHandle<()>>,
    offline: bool,
    #[cfg(test)]
    network_statuses: std::sync::Mutex<Vec<bool>>,
    #[cfg(test)]
    playlist_item_requests: std::sync::Mutex<Vec<(String, u32, u64)>>,
    #[cfg(test)]
    playlist_sample_requests: std::sync::Mutex<Vec<(String, u32, u64)>>,
    #[cfg(test)]
    playlist_add_requests: std::sync::Mutex<Vec<ApiRequest>>,
    #[cfg(test)]
    remote_play_requests: std::sync::Mutex<Vec<ApiRequest>>,
    #[cfg(test)]
    remote_shuffle_requests: std::sync::Mutex<Vec<ApiRequest>>,
    #[cfg(test)]
    transfer_requests: std::sync::Mutex<Vec<ApiRequest>>,
    #[cfg(test)]
    queue_requests: std::sync::Mutex<Vec<ApiRequest>>,
    #[cfg(test)]
    queued_tracks: std::sync::Mutex<Vec<String>>,
    #[cfg(test)]
    player_commands: std::sync::Mutex<Vec<PlayerCommand>>,
    #[cfg(test)]
    album_type_requests: std::sync::Mutex<Vec<Vec<String>>>,
    #[cfg(test)]
    home_episode_requests: std::sync::Mutex<Vec<(Vec<String>, u64)>>,
    #[cfg(test)]
    library_cache_stores: std::sync::Mutex<Vec<crate::account_cache::Cache<Vec<Playlist>>>>,
    #[cfg(test)]
    playlist_row_removals: std::sync::Mutex<Vec<(String, Vec<session_reads::RemovedRow>)>>,
    #[cfg(test)]
    home_cache_stores:
        std::sync::Mutex<Vec<crate::account_cache::Cache<crate::model::HomeSnapshot>>>,
    #[cfg(test)]
    top_tracks_requests: std::sync::Mutex<Vec<(TopPeriod, u32, bool, u64)>>,
}

impl Backend {
    pub fn spawn(
        dirs: AppDirs,
        engine_config: EngineConfig,
        _web_client_id: Option<String>,
        waker: Waker,
        _restore_sign_in: bool,
    ) -> Self {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("resonix-runtime")
            .enable_all()
            .build()
            .expect("unable to start the async runtime");

        let http = Http::from_proxy(&engine_config.proxy).unwrap_or_else(|error| {
            let _ = event_tx.send(Event::Error(format!(
                "Network configuration failed: {error}"
            )));
            Http::unavailable(error)
        });
        let art = ArtLoader::new(http.clone(), runtime.handle().clone(), dirs.art_cache_dir());
        let activity = Arc::new(NetActivity::default());

        let soloist_output_volume = crate::soloist::OutputVolume::default();
        let worker_activity = Arc::clone(&activity);
        let worker_commands = command_tx.clone();
        let worker_art = art.clone();
        let thread = std::thread::Builder::new()
            .name("resonix-backend".to_string())
            .spawn(move || {
                runtime.block_on(async move {
                    let mut worker = Worker::new(
                        dirs,
                        engine_config,
                        http,
                        worker_art,
                        worker_activity,
                        event_tx,
                        worker_commands,
                        waker,
                    );
                    worker.start_local_backend().await;
                    worker.run(command_rx).await;
                });
                runtime.shutdown_timeout(Duration::from_secs(2));
            })
            .expect("unable to start the backend thread");

        Self {
            soloist_output_volume,
            commands: command_tx,
            events: event_rx,
            art,
            activity,
            thread: Some(thread),
            offline: false,
            #[cfg(test)]
            network_statuses: std::sync::Mutex::new(Vec::new()),
            #[cfg(test)]
            playlist_item_requests: std::sync::Mutex::new(Vec::new()),
            #[cfg(test)]
            playlist_sample_requests: std::sync::Mutex::new(Vec::new()),
            #[cfg(test)]
            playlist_add_requests: std::sync::Mutex::new(Vec::new()),
            #[cfg(test)]
            remote_play_requests: std::sync::Mutex::new(Vec::new()),
            #[cfg(test)]
            remote_shuffle_requests: std::sync::Mutex::new(Vec::new()),
            #[cfg(test)]
            transfer_requests: std::sync::Mutex::new(Vec::new()),
            #[cfg(test)]
            queue_requests: std::sync::Mutex::new(Vec::new()),
            #[cfg(test)]
            queued_tracks: std::sync::Mutex::new(Vec::new()),
            #[cfg(test)]
            player_commands: std::sync::Mutex::new(Vec::new()),
            #[cfg(test)]
            album_type_requests: std::sync::Mutex::new(Vec::new()),
            #[cfg(test)]
            home_episode_requests: std::sync::Mutex::new(Vec::new()),
            #[cfg(test)]
            library_cache_stores: std::sync::Mutex::new(Vec::new()),
            #[cfg(test)]
            playlist_row_removals: std::sync::Mutex::new(Vec::new()),
            #[cfg(test)]
            home_cache_stores: std::sync::Mutex::new(Vec::new()),
            #[cfg(test)]
            top_tracks_requests: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub fn activity(&self) -> &NetActivity {
        &self.activity
    }

    #[cfg_attr(not(any(test, feature = "demo")), allow(dead_code))]
    pub fn set_offline(&mut self, offline: bool) {
        self.offline = offline;
    }

    pub fn send(&self, command: Command) {
        #[cfg(test)]
        if let Command::StoreLibraryCache(cache) = &command {
            self.library_cache_stores
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(cache.clone());
        }
        #[cfg(test)]
        if let Command::RemovePlaylistRows { playlist_id, rows } = &command {
            self.playlist_row_removals
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push((playlist_id.clone(), rows.clone()));
        }
        #[cfg(test)]
        if let Command::StoreHomeCache(cache) = &command {
            self.home_cache_stores
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(cache.clone());
        }
        if self.offline
            && !matches!(
                command,
                Command::Accent { .. }
                    | Command::ClearAudioCache
                    | Command::Shutdown
                    | Command::CheckForUpdates { .. }
                    | Command::DownloadUpdate { .. }
                    | Command::InstallUpdate { .. }
            )
        {
            return;
        }
        let _ = self.commands.send(command);
    }

    pub fn choose_playlist_cover(&self, id: String, request: u64) {
        if self.offline {
            return;
        }
        let selected = rfd::AsyncFileDialog::new()
            .set_title("Choose playlist cover")
            .add_filter("JPEG or PNG image", &["jpg", "jpeg", "png"])
            .pick_file();
        self.send(Command::ChoosePlaylistCover {
            id,
            request,
            selected: Box::pin(selected),
        });
    }

    pub fn choose_local_files_folder(&self) {
        if self.offline {
            return;
        }
        let selected = rfd::AsyncFileDialog::new()
            .set_title("Choose the local files folder")
            .pick_folder();
        self.send(Command::ChooseLocalFilesFolder {
            selected: Box::pin(selected),
        });
    }

    pub fn api(&self, request: ApiRequest) {
        #[cfg(test)]
        if matches!(
            request,
            ApiRequest::AlbumQueueTracks { .. }
                | ApiRequest::AddToQueue { .. }
                | ApiRequest::AddManyToQueue { .. }
        ) {
            self.queue_requests.lock().unwrap().push(request.clone());
        }
        #[cfg(test)]
        if matches!(
            request,
            ApiRequest::Remote {
                action: RemoteAction::Play,
                ..
            } | ApiRequest::ShufflePlay { .. }
        ) {
            self.remote_play_requests
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(request.clone());
        }
        #[cfg(test)]
        if matches!(
            request,
            ApiRequest::Remote {
                action: RemoteAction::Shuffle,
                ..
            }
        ) {
            self.remote_shuffle_requests
                .lock()
                .unwrap()
                .push(request.clone());
        }
        #[cfg(test)]
        if matches!(request, ApiRequest::Transfer { .. }) {
            self.transfer_requests.lock().unwrap().push(request.clone());
        }
        #[cfg(test)]
        if matches!(
            request,
            ApiRequest::AddToPlaylist { .. }
                | ApiRequest::CheckPlaylistDuplicates { .. }
                | ApiRequest::UpdatePlaylist { .. }
        ) {
            self.playlist_add_requests
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(request.clone());
        }
        #[cfg(test)]
        if let ApiRequest::PlaylistItems {
            id,
            offset,
            generation,
        } = &request
        {
            self.playlist_item_requests
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push((id.clone(), *offset, *generation));
        }
        #[cfg(test)]
        if let ApiRequest::PlaylistSample {
            id,
            offset,
            generation,
        } = &request
        {
            self.playlist_sample_requests
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push((id.clone(), *offset, *generation));
        }
        #[cfg(test)]
        if let ApiRequest::HomeEpisodes { shows, generation } = &request {
            self.home_episode_requests
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push((
                    shows.iter().map(|show| show.id.clone()).collect(),
                    *generation,
                ));
        }
        #[cfg(test)]
        if let ApiRequest::TopTracks {
            period,
            offset,
            full,
            generation,
        } = &request
        {
            self.top_tracks_requests
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push((*period, *offset, *full, *generation));
        }
        self.send(Command::Api(request));
    }

    #[cfg(test)]
    pub(crate) fn take_playlist_row_removals(
        &self,
    ) -> Vec<(String, Vec<session_reads::RemovedRow>)> {
        std::mem::take(
            &mut *self
                .playlist_row_removals
                .lock()
                .unwrap_or_else(|p| p.into_inner()),
        )
    }

    #[cfg(test)]
    pub(crate) fn take_library_cache_stores(
        &self,
    ) -> Vec<crate::account_cache::Cache<Vec<Playlist>>> {
        std::mem::take(
            &mut *self
                .library_cache_stores
                .lock()
                .unwrap_or_else(|p| p.into_inner()),
        )
    }

    #[cfg(test)]
    pub(crate) fn take_home_cache_stores(
        &self,
    ) -> Vec<crate::account_cache::Cache<crate::model::HomeSnapshot>> {
        std::mem::take(
            &mut *self
                .home_cache_stores
                .lock()
                .unwrap_or_else(|p| p.into_inner()),
        )
    }

    #[cfg(test)]
    pub(crate) fn take_home_episode_requests(&self) -> Vec<(Vec<String>, u64)> {
        std::mem::take(
            &mut *self
                .home_episode_requests
                .lock()
                .unwrap_or_else(|p| p.into_inner()),
        )
    }

    #[cfg(test)]
    pub(crate) fn take_top_tracks_requests(&self) -> Vec<(TopPeriod, u32, bool, u64)> {
        std::mem::take(
            &mut *self
                .top_tracks_requests
                .lock()
                .unwrap_or_else(|p| p.into_inner()),
        )
    }

    #[cfg(test)]
    pub fn take_playlist_item_requests(&self) -> Vec<(String, u32, u64)> {
        std::mem::take(
            &mut *self
                .playlist_item_requests
                .lock()
                .unwrap_or_else(|p| p.into_inner()),
        )
    }

    #[cfg(test)]
    pub fn take_playlist_sample_requests(&self) -> Vec<(String, u32, u64)> {
        std::mem::take(
            &mut *self
                .playlist_sample_requests
                .lock()
                .unwrap_or_else(|p| p.into_inner()),
        )
    }

    #[cfg(test)]
    pub fn take_playlist_add_requests(&self) -> Vec<ApiRequest> {
        std::mem::take(
            &mut *self
                .playlist_add_requests
                .lock()
                .unwrap_or_else(|p| p.into_inner()),
        )
    }

    #[cfg(test)]
    pub fn take_remote_play_requests(&self) -> Vec<ApiRequest> {
        std::mem::take(
            &mut *self
                .remote_play_requests
                .lock()
                .unwrap_or_else(|p| p.into_inner()),
        )
    }

    #[cfg(test)]
    pub fn take_transfer_requests(&self) -> Vec<ApiRequest> {
        std::mem::take(&mut *self.transfer_requests.lock().unwrap())
    }

    #[cfg(test)]
    pub fn take_remote_shuffle_requests(&self) -> Vec<ApiRequest> {
        std::mem::take(&mut *self.remote_shuffle_requests.lock().unwrap())
    }

    #[cfg(test)]
    pub(crate) fn take_queue_requests(&self) -> Vec<ApiRequest> {
        std::mem::take(&mut *self.queue_requests.lock().unwrap())
    }

    #[cfg(test)]
    pub(crate) fn take_queued_tracks(&self) -> Vec<String> {
        std::mem::take(&mut *self.queued_tracks.lock().unwrap())
    }

    #[cfg(test)]
    pub(crate) fn take_player_commands(&self) -> Vec<PlayerCommand> {
        std::mem::take(&mut *self.player_commands.lock().unwrap())
    }

    pub fn player(&self, command: PlayerCommand) {
        #[cfg(test)]
        self.player_commands.lock().unwrap().push(command.clone());
        #[cfg(test)]
        if let PlayerCommand::AddToQueue(uri) = &command {
            self.queued_tracks.lock().unwrap().push(uri.clone());
        }
        self.send(Command::Player(command));
    }

    pub fn network_status(&self, reachable: bool) {
        #[cfg(test)]
        self.network_statuses
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(reachable);
        self.send(Command::NetworkStatus(reachable));
    }

    #[cfg(test)]
    pub fn take_network_statuses(&self) -> Vec<bool> {
        std::mem::take(
            &mut *self
                .network_statuses
                .lock()
                .unwrap_or_else(|p| p.into_inner()),
        )
    }

    pub(crate) fn album_types(&self, uris: Vec<String>) {
        #[cfg(test)]
        self.album_type_requests
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(uris.clone());
        self.send(Command::AlbumTypes(uris));
    }

    #[cfg(test)]
    pub(crate) fn take_album_type_requests(&self) -> Vec<Vec<String>> {
        std::mem::take(
            &mut *self
                .album_type_requests
                .lock()
                .unwrap_or_else(|p| p.into_inner()),
        )
    }

    pub fn poll(&self) -> Vec<Event> {
        self.events.try_iter().collect()
    }

    pub fn art(&self) -> &ArtLoader {
        &self.art
    }

    pub fn shutdown(&mut self) {
        self.send(Command::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn local_user() -> User {
    User {
        id: "local".into(),
        display_name: Some("Resonix".into()),
        images: Vec::new(),
        product: Some("premium".into()),
        country: Some("US".into()),
        uri: Some("spotify:user:local".into()),
    }
}

fn build_explore_playlists(
    sections: &[crate::youtube::ExploreSection],
    cache: &mut HashMap<String, Vec<YtTrack>>,
    titles: &mut HashMap<String, String>,
    section_cache: &mut Vec<(String, Vec<YtTrack>)>,
) -> Vec<Playlist> {
    let mut playlists = Vec::new();
    section_cache.clear();
    for (idx, section) in sections.iter().enumerate() {
        let id = format!("explore_{idx}");
        let images = section
            .tracks
            .first()
            .map(|t| thumbnail_images(&t.thumbnail))
            .unwrap_or_default();
        let total = section.tracks.len() as u32;
        cache.insert(id.clone(), section.tracks.clone());
        titles.insert(id.clone(), section.title.clone());
        section_cache.push((section.title.clone(), section.tracks.clone()));
        playlists.push(Playlist {
            id: id.clone(),
            name: section.title.clone(),
            description: Some(format!("{} tracks", section.tracks.len())),
            images,
            owner: Owner {
                id: Some("local".into()),
                display_name: Some("Resonix Explore".into()),
                uri: None,
            },
            public: Some(false),
            collaborative: false,
            snapshot_id: Some("explore".into()),
            tracks: Some(TrackCount { total }),
            items_count: Some(TrackCount { total }),
            uri: format!("spotify:playlist:{id}"),
            ..Playlist::default()
        });
    }
    playlists
}

fn empty_page<T>(offset: u32, limit: u32) -> Page<T> {
    Page {
        items: Vec::new(),
        total: 0,
        limit,
        offset,
        next: None,
    }
}

/// Parses plain or WebVTT/SRT subtitle text into timed lyrics lines.
fn parse_subtitles(raw: &str) -> Option<crate::lyrics::Lyrics> {
    let mut lines = Vec::new();
    let mut current_time: Option<u32> = None;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed == "WEBVTT"
            || trimmed.starts_with("Kind:")
            || trimmed.starts_with("Language:")
        {
            current_time = None;
            continue;
        }
        if let Some((start, _end)) = trimmed.split_once("-->") {
            current_time = parse_vtt_ms(start.trim());
            continue;
        }
        if trimmed.chars().all(|c| c.is_ascii_digit()) && current_time.is_none() {
            continue;
        }
        let cleaned = strip_html_tags(trimmed);
        if !cleaned.is_empty() && cleaned != "\u{266a}" {
            lines.push(crate::lyrics::Line {
                at_ms: current_time,
                text: cleaned,
            });
        }
    }
    if lines.is_empty() {
        return None;
    }
    let synced = lines.iter().all(|l| l.at_ms.is_some());
    Some(crate::lyrics::Lyrics {
        lines,
        synced,
        instrumental: false,
    })
}

fn parse_vtt_ms(ts: &str) -> Option<u32> {
    let ts = ts.replace(',', ".");
    let (hms, ms_part) = ts.split_once('.')?;
    let ms: u32 = ms_part.get(..3).unwrap_or(ms_part).parse().ok()?;
    let parts: Vec<u32> = hms.split(':').filter_map(|p| p.parse().ok()).collect();
    let secs = match parts.as_slice() {
        [m, s] => m * 60 + s,
        [h, m, s] => h * 3600 + m * 60 + s,
        _ => return None,
    };
    Some(secs.saturating_mul(1000).saturating_add(ms))
}

fn strip_html_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.trim().to_string()
}

fn yt_track_from_spotify(track: &Track) -> Option<YtTrack> {
    let vid = track
        .id
        .clone()
        .or_else(|| video_id_from_uri(&track.uri).map(str::to_string))?;
    Some(YtTrack {
        id: vid,
        title: track.name.clone(),
        artist: crate::api::models::join_names(track.artists.iter().map(|a| a.name.as_str())),
        thumbnail: track
            .album
            .as_ref()
            .and_then(|a| a.images.first())
            .map(|img| img.url.clone())
            .unwrap_or_default(),
        duration_secs: f64::from(track.duration_ms) / 1000.0,
        stream_url: None,
    })
}

fn load_downloads_library(db: &SearchCache) -> Result<crate::model::DownloadsLibrary, String> {
    let db_cols = db.list_download_collections().map_err(|e| e.to_string())?;
    let standalone_rows = db.list_standalone_downloads().map_err(|e| e.to_string())?;
    let all_track_ids = db.list_all_downloaded_ids().map_err(|e| e.to_string())?;

    let collections = db_cols
        .into_iter()
        .map(|col| {
            let mut total_bytes = 0u64;
            let tracks: Vec<crate::model::DownloadedTrack> = col
                .tracks
                .into_iter()
                .map(|(t, path, size_bytes)| {
                    total_bytes = total_bytes.saturating_add(size_bytes);
                    crate::model::DownloadedTrack {
                        track: to_spotify_track(&t),
                        path,
                        size_bytes,
                    }
                })
                .collect();
            let thumbnail = (!col.thumbnail.is_empty())
                .then_some(col.thumbnail)
                .or_else(|| {
                    tracks.first().and_then(|d| {
                        d.track
                            .album
                            .as_ref()
                            .and_then(|a| a.images.first())
                            .map(|img| img.url.clone())
                    })
                });
            crate::model::DownloadedCollection {
                uri: col.uri,
                label: col.label,
                thumbnail,
                total_tracks: col.total_tracks,
                tracks,
                total_bytes,
            }
        })
        .collect();

    let tracks = standalone_rows
        .into_iter()
        .map(|(t, path, size_bytes)| crate::model::DownloadedTrack {
            track: to_spotify_track(&t),
            path,
            size_bytes,
        })
        .collect();

    Ok(crate::model::DownloadsLibrary {
        collections,
        tracks,
        all_track_ids,
    })
}

fn emit_downloads(db: &SearchCache, events: &std::sync::mpsc::Sender<Event>, waker: &Waker) {
    let res = load_downloads_library(db).map_err(|msg| ApiError::Network(msg));
    let _ = events.send(Event::Api(Box::new(ApiResponse::Downloads(res))));
    waker.wake();
}

fn spawn_yt_downloads(
    downloads: Arc<DownloadManager>,
    db: Arc<SearchCache>,
    events: std::sync::mpsc::Sender<Event>,
    waker: Waker,
    standalone: bool,
    yt_tracks: Vec<YtTrack>,
) {
    let mut seen = std::collections::HashSet::new();
    let mut queued = Vec::new();
    for track in yt_tracks {
        if track.id.is_empty() || !seen.insert(track.id.clone()) {
            continue;
        }
        if downloads.is_active(&track.id) {
            continue;
        }
        queued.push(track);
    }
    if queued.is_empty() {
        return;
    }
    let track_ids: Vec<String> = queued.iter().map(|t| t.id.clone()).collect();
    let _ = events.send(Event::DownloadStarted { track_ids });
    waker.wake();

    for yt_track in queued {
        let downloads = Arc::clone(&downloads);
        let db = Arc::clone(&db);
        let events = events.clone();
        let waker = waker.clone();
        let track_id = yt_track.id.clone();
        let title = yt_track.title.clone();
        tokio::spawn(async move {
            match downloads
                .download_with_mode(&db, &yt_track, standalone, |_| {})
                .await
            {
                Ok(true) => {
                    let _ = events.send(Event::DownloadFinished {
                        track_id,
                        title,
                        result: Ok(()),
                    });
                    waker.wake();
                }
                Ok(false) => {}
                Err(err) => {
                    let _ = events.send(Event::DownloadFinished {
                        track_id,
                        title,
                        result: Err(err),
                    });
                    waker.wake();
                }
            }
        });
    }
}

struct BusyGuard(Arc<NetActivity>);

impl BusyGuard {
    fn new(activity: Arc<NetActivity>) -> Self {
        activity.begin();
        Self(activity)
    }
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        self.0.end();
    }
}

struct Worker {
    dirs: AppDirs,
    engine_config: EngineConfig,
    http: Http,
    art: ArtLoader,
    activity: Arc<NetActivity>,
    events: std::sync::mpsc::Sender<Event>,
    commands: mpsc::UnboundedSender<Command>,
    waker: Waker,
    db: Arc<SearchCache>,
    extractor: Arc<Extractor>,
    downloads: Arc<DownloadManager>,
    engine: Option<Engine>,
    explore_cache: Arc<std::sync::Mutex<HashMap<String, Vec<YtTrack>>>>,
    explore_titles: Arc<std::sync::Mutex<HashMap<String, String>>>,
    explore_sections: Arc<std::sync::Mutex<Vec<(String, Vec<YtTrack>)>>>,
    _credentials: CredentialStore,
}

impl Worker {
    #[allow(clippy::too_many_arguments)]
    fn new(
        dirs: AppDirs,
        engine_config: EngineConfig,
        http: Http,
        art: ArtLoader,
        activity: Arc<NetActivity>,
        events: std::sync::mpsc::Sender<Event>,
        commands: mpsc::UnboundedSender<Command>,
        waker: Waker,
    ) -> Self {
        let _ = std::fs::create_dir_all(&dirs.state);
        let db = Arc::new(
            SearchCache::new(&dirs.state).expect("unable to initialize resonix SQLite database"),
        );
        let extractor = Arc::new(Extractor::new(&dirs.state));
        let downloads = Arc::new(DownloadManager::new(&dirs.state));
        let credentials = CredentialStore::new(dirs.clone());
        Self {
            dirs,
            engine_config,
            http,
            art,
            activity,
            events,
            commands,
            waker,
            db,
            extractor,
            downloads,
            engine: None,
            explore_cache: Arc::new(std::sync::Mutex::new(HashMap::new())),
            explore_titles: Arc::new(std::sync::Mutex::new(HashMap::new())),
            explore_sections: Arc::new(std::sync::Mutex::new(Vec::new())),
            _credentials: credentials,
        }
    }

    fn emit(&self, event: Event) {
        let _ = self.events.send(event);
        self.waker.wake();
    }

    fn engine_notify(&self) -> crate::player::Notify {
        let events = self.events.clone();
        let waker = self.waker.clone();
        Arc::new(move |engine_event| {
            let mapped = match engine_event {
                EngineEvent::State(state) => Event::Local(Box::new(state)),
                EngineEvent::SessionEnded => Event::Error("Playback session ended".into()),
                EngineEvent::AudioDevice { name, id } => Event::AudioDeviceFound { name, id },
                EngineEvent::RemoteChanged => Event::RemoteChanged,
                EngineEvent::RemoteVolume { device_id, volume } => {
                    Event::RemoteVolume { device_id, volume }
                }
            };
            let _ = events.send(mapped);
            waker.wake();
        })
    }

    async fn start_local_backend(&mut self) {
        self.emit(Event::Auth(AuthStatus::Connecting));
        match Engine::start_youtube(
            &self.engine_config,
            Arc::clone(&self.db),
            Arc::clone(&self.extractor),
            Arc::clone(&self.explore_cache),
            self.engine_notify(),
        ) {
            Ok(engine) => {
                let device_id = engine.device_id().to_string();
                self.engine = Some(engine);
                self.emit(Event::Auth(AuthStatus::Connected {
                    username: "resonix".into(),
                }));
                self.emit(Event::Playback(LocalPlayback::Ready { device_id }));
                self.emit(Event::Api(Box::new(ApiResponse::Me(Ok(local_user())))));
                self.emit_rootlist();
            }
            Err(err) => {
                let msg = format!("Failed to start audio engine: {err}");
                self.emit(Event::Playback(LocalPlayback::Failed(msg.clone())));
                self.emit(Event::Auth(AuthStatus::Failed(msg)));
            }
        }
    }

    fn emit_rootlist(&self) {
        let db = Arc::clone(&self.db);
        let events = self.events.clone();
        let waker = self.waker.clone();
        tokio::task::spawn_blocking(move || {
            let result = db
                .list_playlists()
                .map(|pls| {
                    let mut entries = Vec::with_capacity(pls.len());
                    let mut editable = BTreeSet::new();
                    for pl in pls {
                        let uri = format!("spotify:playlist:{}", pl.id);
                        editable.insert(uri.clone());
                        entries.push(RootlistEntry::Playlist(uri));
                    }
                    Rootlist { entries, editable }
                })
                .map_err(|e| e.to_string());
            let _ = events.send(Event::Rootlist { result });
            waker.wake();
        });
    }

    async fn run(&mut self, mut commands: mpsc::UnboundedReceiver<Command>) {
        while let Some(command) = commands.recv().await {
            match command {
                Command::Shutdown => {
                    if let Some(engine) = self.engine.take() {
                        engine.shutdown();
                    }
                    break;
                }
                Command::Player(cmd) => {
                    if let Some(engine) = &self.engine {
                        if let Err(err) = engine.command(cmd) {
                            self.emit(Event::Error(format!("Playback error: {err}")));
                        }
                    } else {
                        self.emit(Event::Error("Audio engine not initialized".into()));
                    }
                }
                Command::RestartEngine(config) => {
                    self.engine_config = config.clone();
                    if let Some(old) = self.engine.take() {
                        old.shutdown();
                    }
                    match Engine::start_youtube(
                        &config,
                        Arc::clone(&self.db),
                        Arc::clone(&self.extractor),
                        Arc::clone(&self.explore_cache),
                        self.engine_notify(),
                    ) {
                        Ok(engine) => {
                            let device_id = engine.device_id().to_string();
                            self.engine = Some(engine);
                            self.emit(Event::Playback(LocalPlayback::Ready { device_id }));
                        }
                        Err(err) => {
                            self.emit(Event::Playback(LocalPlayback::Failed(err.to_string())));
                        }
                    }
                }
                Command::SignIn { .. } | Command::AuthorizePlayback => {
                    if self.engine.is_none() {
                        self.start_local_backend().await;
                    } else if let Some(engine) = &self.engine {
                        self.emit(Event::Auth(AuthStatus::Connected {
                            username: "resonix".into(),
                        }));
                        self.emit(Event::Playback(LocalPlayback::Ready {
                            device_id: engine.device_id().to_string(),
                        }));
                        self.emit(Event::Api(Box::new(ApiResponse::Me(Ok(local_user())))));
                    }
                }
                Command::SignOut | Command::CancelSignIn => {
                    if let Some(engine) = self.engine.take() {
                        engine.shutdown();
                    }
                    self.emit(Event::Auth(AuthStatus::SignedOut));
                    self.emit(Event::Playback(LocalPlayback::Unavailable));
                }
                Command::Api(request) => self.handle_api(request),
                Command::Rootlist => self.emit_rootlist(),
                Command::RemovePlaylistRows { playlist_id, rows } => {
                    let db = Arc::clone(&self.db);
                    let events = self.events.clone();
                    let waker = self.waker.clone();
                    let uris: Vec<String> = rows.into_iter().map(|r| r.uri).collect();
                    tokio::task::spawn_blocking(move || {
                        if let Ok(pid) = playlist_id.parse::<i64>() {
                            for uri in &uris {
                                if let Some(vid) = video_id_from_uri(uri) {
                                    let _ = db.remove_from_playlist(pid, vid);
                                }
                            }
                        }
                        let _ =
                            events.send(Event::Api(Box::new(ApiResponse::PlaylistItemsChanged {
                                id: playlist_id,
                                message: "Removed from playlist".into(),
                                result: Ok(Some("local".into())),
                            })));
                        waker.wake();
                    });
                }
                Command::ListOutputDevices => {
                    let events = self.events.clone();
                    let waker = self.waker.clone();
                    tokio::task::spawn_blocking(move || {
                        let result = fastframe_audio::output_devices().map_err(|e| e.to_string());
                        let _ = events.send(Event::OutputDevices(result));
                        waker.wake();
                    });
                }
                Command::ClearAudioCache => {
                    let root = self.dirs.audio_cache_dir();
                    let events = self.events.clone();
                    let waker = self.waker.clone();
                    tokio::task::spawn_blocking(move || {
                        let res = crate::storage::clear_audio(&root, None);
                        let _ = events.send(Event::AudioCacheCleared(res));
                        waker.wake();
                    });
                }
                Command::AudioCacheClearFinished(res) => {
                    self.emit(Event::AudioCacheCleared(res));
                }
                Command::ApplyProxy { request, config } => {
                    let result = match Http::from_proxy(&config) {
                        Ok(http) => {
                            self.http = http;
                            Ok(true)
                        }
                        Err(e) => Err(e),
                    };
                    self.emit(Event::ProxyApplied {
                        request,
                        config,
                        result,
                    });
                }
                Command::Lyrics(req) => {
                    let client = self.http.client();
                    let lyrics_dir = self.dirs.lyrics_cache_dir();
                    let extractor = Arc::clone(&self.extractor);
                    let events = self.events.clone();
                    let waker = self.waker.clone();
                    let activity = Arc::clone(&self.activity);
                    tokio::spawn(async move {
                        let _busy = BusyGuard::new(activity);
                        let mut res = match client {
                            Ok(c) => crate::lyrics::fetch(&c, &lyrics_dir, &req.query)
                                .await
                                .map_err(|e| e.to_string()),
                            Err(e) => Err(e),
                        };
                        if matches!(res, Ok(None) | Err(_))
                            && let Some(vid) = video_id_from_uri(&req.uri)
                        {
                            if let Ok(raw_subs) = extractor.get_subtitles(vid, "en").await
                                && let Some(parsed) = parse_subtitles(&raw_subs)
                            {
                                res = Ok(Some(parsed));
                            }
                        }
                        let _ = events.send(Event::Lyrics {
                            uri: req.uri,
                            result: res,
                        });
                        waker.wake();
                    });
                }
                Command::Accent { url } => {
                    let art = self.art.clone();
                    let events = self.events.clone();
                    let waker = self.waker.clone();
                    tokio::spawn(async move {
                        if let Ok(bytes) = art.fetch(&url).await {
                            let palette = art_palette(&bytes);
                            if let Some(&color) = palette.first() {
                                let _ = events.send(Event::Accent {
                                    url,
                                    color,
                                    palette,
                                });
                                waker.wake();
                            }
                        }
                    });
                }
                Command::Autostart(request) => {
                    let events = self.events.clone();
                    let waker = self.waker.clone();
                    tokio::task::spawn_blocking(move || {
                        let result = match request {
                            AutostartRequest::Read => Ok(autostart().state()),
                            AutostartRequest::Set(mode) => autostart().set(mode),
                        };
                        let _ = events.send(Event::Autostart {
                            request,
                            result: result.map_err(|e| e.to_string()),
                        });
                        waker.wake();
                    });
                }
                Command::OpenThemesFolder => {
                    let dir = self.dirs.skins_dir();
                    let _ = std::fs::create_dir_all(&dir);
                    let _ = open::that(&dir);
                }
                Command::ScanLocalFiles(folder) => {
                    self.emit(Event::LocalFiles {
                        folder,
                        uris: Arc::new(HashMap::new()),
                    });
                }
                Command::ChooseLocalFilesFolder { selected } => {
                    let events = self.events.clone();
                    let waker = self.waker.clone();
                    tokio::spawn(async move {
                        let picked = selected.await.map(|h| h.path().to_path_buf());
                        let _ = events.send(Event::LocalFilesFolderChosen(picked));
                        waker.wake();
                    });
                }
                Command::ChoosePlaylistCover {
                    id,
                    request,
                    selected,
                } => {
                    let events = self.events.clone();
                    let waker = self.waker.clone();
                    tokio::spawn(async move {
                        let picked = selected.await;
                        let result = match picked {
                            Some(handle) => crate::playlist_cover::read(handle.path()).map(Some),
                            None => Ok(None),
                        };
                        let _ = events.send(Event::PlaylistCoverChosen {
                            id,
                            request,
                            result,
                        });
                        waker.wake();
                    });
                }
                Command::CheckPlaylistCover {
                    id,
                    request,
                    images,
                    ..
                } => {
                    self.emit(Event::PlaylistCoverChecked {
                        id,
                        request,
                        images,
                        result: Ok(true),
                    });
                }
                Command::CheckForUpdates { manual, .. } => {
                    self.emit(Event::UpdateChecked {
                        manual,
                        result: Ok(None),
                    });
                }
                Command::DownloadUpdate { .. } => {
                    self.emit(Event::UpdateDownloaded(Err(
                        "Self-updates are disabled for Resonix builds".into(),
                    )));
                }
                Command::InstallUpdate { .. } => {
                    self.emit(Event::UpdateInstalling(Err(
                        "Self-updates are disabled for Resonix builds".into(),
                    )));
                }
                Command::LoadPlaylistCache { id, generation } => {
                    self.emit(Event::PlaylistCache {
                        account_id: "local".into(),
                        id,
                        generation,
                        cache: None,
                    });
                }
                Command::StorePlaylistCache { .. }
                | Command::StoreLikedSongsCache(_)
                | Command::StoreLibraryCache(_)
                | Command::StoreHomeCache(_) => {}
                Command::LoadLikedSongsCache { generation } => {
                    self.emit(Event::LikedSongsCache {
                        account_id: "local".into(),
                        generation,
                        cache: None,
                    });
                }
                Command::LoadAccountCaches { generation } => {
                    self.emit(Event::AccountCaches {
                        account_id: "local".into(),
                        generation,
                        playlists: None,
                        home: None,
                    });
                }
                Command::FriendActivity(generation) => {
                    self.emit(Event::FriendActivity {
                        generation,
                        result: Ok(Vec::new()),
                    });
                }
                Command::PlayCounts { uri, generation } => {
                    self.emit(Event::PlayCounts {
                        uri,
                        generation,
                        result: Ok(session_reads::PageMetadata::default()),
                    });
                }
                Command::Radio { seed, generation } => {
                    let db = Arc::clone(&self.db);
                    let extractor = Arc::clone(&self.extractor);
                    let events = self.events.clone();
                    let waker = self.waker.clone();
                    tokio::spawn(async move {
                        let query = if let Some(vid) = video_id_from_uri(&seed) {
                            if let Ok(Some(t)) = db.get_track_by_id(vid) {
                                format!("{} {} radio", t.artist, t.title)
                            } else {
                                seed.clone()
                            }
                        } else {
                            seed.clone()
                        };
                        let res = extractor
                            .search(&query, 25)
                            .await
                            .map(|tracks| {
                                let _ = db.upsert_tracks(&tracks);
                                tracks.iter().map(to_spotify_track).collect()
                            })
                            .map_err(|e| e.to_string());
                        let _ = events.send(Event::Radio {
                            seed,
                            generation,
                            result: res,
                        });
                        waker.wake();
                    });
                }
                Command::Reconnect => {
                    if self.engine.is_none() {
                        self.start_local_backend().await;
                    }
                }
                Command::DownloadTrack(track) => {
                    if let Some(yt_track) = yt_track_from_spotify(&track) {
                        spawn_yt_downloads(
                            Arc::clone(&self.downloads),
                            Arc::clone(&self.db),
                            self.events.clone(),
                            self.waker.clone(),
                            true,
                            vec![yt_track],
                        );
                    }
                }
                Command::DownloadUris {
                    collection_uri,
                    label,
                    resolved_tracks,
                    uris,
                } => {
                    let downloads = Arc::clone(&self.downloads);
                    let db = Arc::clone(&self.db);
                    let extractor = Arc::clone(&self.extractor);
                    let events = self.events.clone();
                    let waker = self.waker.clone();
                    tokio::spawn(async move {
                        let mut by_id: HashMap<String, YtTrack> = resolved_tracks
                            .iter()
                            .filter_map(yt_track_from_spotify)
                            .map(|t| (t.id.clone(), t))
                            .collect();
                        let mut ordered_tracks = Vec::new();
                        let mut seen = std::collections::HashSet::new();
                        for uri in uris {
                            if let Some(vid) = video_id_from_uri(&uri)
                                && seen.insert(vid.to_string())
                            {
                                if let Some(t) = by_id.remove(vid) {
                                    ordered_tracks.push(t);
                                } else if let Ok(Some(t)) = db.get_track_by_id(vid) {
                                    ordered_tracks.push(t);
                                } else if let Ok(t) = extractor.metadata(vid).await {
                                    let _ = db.upsert_tracks(std::slice::from_ref(&t));
                                    ordered_tracks.push(t);
                                }
                            }
                        }
                        for (_, t) in by_id {
                            if seen.insert(t.id.clone()) {
                                ordered_tracks.push(t);
                            }
                        }
                        if ordered_tracks.is_empty() {
                            return;
                        }
                        let standalone = collection_uri.is_none();
                        if let Some(curi) = collection_uri {
                            let thumb = ordered_tracks
                                .iter()
                                .find_map(|t| {
                                    (!t.thumbnail.is_empty()).then_some(t.thumbnail.as_str())
                                })
                                .unwrap_or("")
                                .to_string();
                            let db2 = Arc::clone(&db);
                            let ordered2 = ordered_tracks.clone();
                            let _ = tokio::task::spawn_blocking(move || {
                                db2.record_download_collection(&curi, &label, &thumb, &ordered2)
                            })
                            .await;
                            emit_downloads(&db, &events, &waker);
                        }
                        spawn_yt_downloads(
                            downloads,
                            db,
                            events,
                            waker,
                            standalone,
                            ordered_tracks,
                        );
                    });
                }
                Command::DownloadContext { uri, label } => {
                    let downloads = Arc::clone(&self.downloads);
                    let db = Arc::clone(&self.db);
                    let extractor = Arc::clone(&self.extractor);
                    let explore_cache = Arc::clone(&self.explore_cache);
                    let events = self.events.clone();
                    let waker = self.waker.clone();
                    tokio::spawn(async move {
                        let resolved: Result<(Vec<YtTrack>, String), String> = if let Some(id) =
                            uri.strip_prefix("spotify:playlist:")
                        {
                            let cached = explore_cache
                                .lock()
                                .unwrap_or_else(|p| p.into_inner())
                                .get(id)
                                .cloned();
                            if let Some(tracks) = cached {
                                let thumb = tracks
                                    .first()
                                    .map(|t| t.thumbnail.clone())
                                    .unwrap_or_default();
                                Ok((tracks, thumb))
                            } else if let Ok(pid) = id.parse::<i64>() {
                                let db2 = Arc::clone(&db);
                                tokio::task::spawn_blocking(move || {
                                    let thumb = db2
                                        .get_playlist(pid)
                                        .ok()
                                        .flatten()
                                        .map(|p| p.thumbnail)
                                        .unwrap_or_default();
                                    let tracks = db2.get_playlist_tracks(pid)?;
                                    Ok((tracks, thumb))
                                })
                                .await
                                .map_err(|e| e.to_string())
                                .and_then(
                                    |r: Result<
                                        (Vec<YtTrack>, String),
                                        crate::youtube::error::AppError,
                                    >| {
                                        r.map_err(|e| e.to_string())
                                    },
                                )
                            } else {
                                Ok((Vec::new(), String::new()))
                            }
                        } else if let Some(id) = uri.strip_prefix("spotify:album:") {
                            let vid = id.strip_prefix("yt_").unwrap_or(id).to_string();
                            let db2 = Arc::clone(&db);
                            let vid2 = vid.clone();
                            let existing =
                                tokio::task::spawn_blocking(move || db2.get_track_by_id(&vid2))
                                    .await
                                    .ok()
                                    .and_then(|r| r.ok())
                                    .flatten();
                            match existing {
                                Some(t) => {
                                    let thumb = t.thumbnail.clone();
                                    Ok((vec![t], thumb))
                                }
                                None => match extractor.metadata(&vid).await {
                                    Ok(t) => {
                                        let _ = db.upsert_tracks(std::slice::from_ref(&t));
                                        let thumb = t.thumbnail.clone();
                                        Ok((vec![t], thumb))
                                    }
                                    Err(e) => Err(e.to_string()),
                                },
                            }
                        } else if let Some(id) = uri.strip_prefix("spotify:artist:") {
                            let name = decode_artist_id(id).unwrap_or_else(|| id.to_string());
                            extractor
                                .search_youtube(&name, 20)
                                .await
                                .map(|tracks| {
                                    let _ = db.upsert_tracks(&tracks);
                                    let list: Vec<YtTrack> = tracks.into_iter().take(20).collect();
                                    let thumb = list
                                        .first()
                                        .map(|t| t.thumbnail.clone())
                                        .unwrap_or_default();
                                    (list, thumb)
                                })
                                .map_err(|e| e.to_string())
                        } else if uri.ends_with(":collection") {
                            let db2 = Arc::clone(&db);
                            tokio::task::spawn_blocking(move || {
                                db2.liked_tracks(0, 10_000).map(|(tracks, _)| {
                                    let thumb = tracks
                                        .first()
                                        .map(|t| t.thumbnail.clone())
                                        .unwrap_or_default();
                                    (tracks, thumb)
                                })
                            })
                            .await
                            .map_err(|e| e.to_string())
                            .and_then(|r| r.map_err(|e| e.to_string()))
                        } else if let Some(vid) = video_id_from_uri(&uri) {
                            match db.get_track_by_id(vid).ok().flatten() {
                                Some(t) => {
                                    let thumb = t.thumbnail.clone();
                                    Ok((vec![t], thumb))
                                }
                                None => match extractor.metadata(vid).await {
                                    Ok(t) => {
                                        let _ = db.upsert_tracks(std::slice::from_ref(&t));
                                        let thumb = t.thumbnail.clone();
                                        Ok((vec![t], thumb))
                                    }
                                    Err(e) => Err(e.to_string()),
                                },
                            }
                        } else {
                            Ok((Vec::new(), String::new()))
                        };
                        match resolved {
                            Ok((yt_tracks, mut thumb)) => {
                                let is_single_track = uri.starts_with("spotify:track:");
                                if !is_single_track && !yt_tracks.is_empty() {
                                    if thumb.is_empty() {
                                        thumb = yt_tracks
                                            .iter()
                                            .find_map(|t| {
                                                (!t.thumbnail.is_empty())
                                                    .then_some(t.thumbnail.clone())
                                            })
                                            .unwrap_or_default();
                                    }
                                    let db2 = Arc::clone(&db);
                                    let uri2 = uri.clone();
                                    let label2 = label.clone();
                                    let tracks2 = yt_tracks.clone();
                                    let _ = tokio::task::spawn_blocking(move || {
                                        db2.record_download_collection(
                                            &uri2, &label2, &thumb, &tracks2,
                                        )
                                    })
                                    .await;
                                    emit_downloads(&db, &events, &waker);
                                }
                                spawn_yt_downloads(
                                    downloads,
                                    db,
                                    events,
                                    waker,
                                    is_single_track,
                                    yt_tracks,
                                );
                            }
                            Err(e) => {
                                let _ = events.send(Event::DownloadFinished {
                                    track_id: String::new(),
                                    title: label,
                                    result: Err(e),
                                });
                                waker.wake();
                            }
                        }
                    });
                }
                Command::DeleteDownload(vid) => {
                    let downloads = Arc::clone(&self.downloads);
                    let db = Arc::clone(&self.db);
                    let events = self.events.clone();
                    let waker = self.waker.clone();
                    tokio::task::spawn_blocking(move || {
                        let _ = downloads.delete(&db, &vid);
                        emit_downloads(&db, &events, &waker);
                    });
                }
                Command::DeleteDownloadCollection(uri) => {
                    let downloads = Arc::clone(&self.downloads);
                    let db = Arc::clone(&self.db);
                    let events = self.events.clone();
                    let waker = self.waker.clone();
                    tokio::task::spawn_blocking(move || {
                        let _ = downloads.delete_collection(&db, &uri);
                        emit_downloads(&db, &events, &waker);
                    });
                }
                Command::UserNames(_)
                | Command::AlbumTypes(_)
                | Command::AudiobookShows(_)
                | Command::PlaylistHeaders(_)
                | Command::SetPrereleaseSaved { .. }
                | Command::Prerelease { .. }
                | Command::ConfigurePersonalWebApp(_)
                | Command::DiscoverReceivers
                | Command::ActivateReceiver(_)
                | Command::VerifyResume
                | Command::NetworkStatus(_)
                | Command::Soloist { .. }
                | Command::SoloistDevice(_)
                | Command::SoloistUpdate { .. }
                | Command::ProxyRestored { .. }
                | Command::CredentialsRestored { .. }
                | Command::ApiFinished { .. } => {}
            }
        }
    }

    fn handle_api(&self, request: ApiRequest) {
        let db = Arc::clone(&self.db);
        let extractor = Arc::clone(&self.extractor);
        let events = self.events.clone();
        let waker = self.waker.clone();
        let activity = Arc::clone(&self.activity);
        let explore_cache = Arc::clone(&self.explore_cache);
        let explore_titles = Arc::clone(&self.explore_titles);
        let explore_sections = Arc::clone(&self.explore_sections);

        let local_device = Device {
            id: self.engine.as_ref().map(|e| e.device_id().to_string()),
            is_active: true,
            is_restricted: false,
            name: self.engine_config.device_name.clone(),
            kind: "Computer".into(),
            volume_percent: Some(
                ((u32::from(
                    self.engine
                        .as_ref()
                        .map_or(self.engine_config.initial_volume, |e| e.heard().level()),
                ) * 100)
                    / u32::from(u16::MAX)) as u8,
            ),
            supports_volume: Some(true),
        };

        let engine_state = self.engine.as_ref().map(|e| e.state_snapshot());
        let engine_queue = self.engine.as_ref().map(|e| e.queue_snapshot());
        let engine_cmd_target = self.commands.clone();

        tokio::spawn(async move {
            let _busy = BusyGuard::new(activity);

            if let ApiRequest::Search { ref query, serial }
            | ApiRequest::SearchCatalogue { ref query, serial } = request
            {
                let _ = events.send(Event::Api(Box::new(ApiResponse::SearchStarted {
                    query: query.clone(),
                    serial,
                    split: false,
                })));
                waker.wake();
            }

            let response: ApiResponse = match request {
                ApiRequest::Me => ApiResponse::Me(Ok(local_user())),

                ApiRequest::Devices => ApiResponse::Devices(Ok(vec![local_device])),

                ApiRequest::PlaybackState { seq } => {
                    let res = match engine_state {
                        Some(st) if st.playback != Playback::Stopped => {
                            let item = st.track.as_ref().map(|lt| {
                                let images = lt
                                    .art_url
                                    .as_deref()
                                    .map(thumbnail_images)
                                    .unwrap_or_default();
                                PlayableItem::Track(Track {
                                    id: video_id_from_uri(&lt.uri).map(str::to_string),
                                    name: lt.title.clone(),
                                    artists: lt.artists.clone(),
                                    album: Some(Album {
                                        name: lt.album.clone(),
                                        album_type: Some("single".into()),
                                        images,
                                        ..Album::default()
                                    }),
                                    duration_ms: lt.duration_ms,
                                    track_number: Some(1),
                                    disc_number: Some(1),
                                    explicit: false,
                                    uri: lt.uri.clone(),
                                    is_playable: Some(true),
                                    is_local: false,
                                    ..Track::default()
                                })
                            });
                            Ok(Some(PlaybackState {
                                device: Some(local_device),
                                repeat_state: st.repeat.api_name().to_string(),
                                shuffle_state: st.shuffle,
                                context: None,
                                timestamp: 0,
                                progress_ms: Some(st.position_now()),
                                is_playing: st.playback == Playback::Playing,
                                item,
                                currently_playing_type: Some("track".into()),
                            }))
                        }
                        _ => Ok(None),
                    };
                    ApiResponse::PlaybackState { seq, result: res }
                }

                ApiRequest::Queue { seq } => {
                    let res = match engine_queue {
                        Some((current, _uris, upcoming)) => {
                            let map_lt = |lt: crate::player::LocalTrack| {
                                let images = lt
                                    .art_url
                                    .as_deref()
                                    .map(thumbnail_images)
                                    .unwrap_or_default();
                                PlayableItem::Track(Track {
                                    id: video_id_from_uri(&lt.uri).map(str::to_string),
                                    name: lt.title,
                                    artists: lt.artists,
                                    album: Some(Album {
                                        name: lt.album,
                                        images,
                                        ..Album::default()
                                    }),
                                    duration_ms: lt.duration_ms,
                                    track_number: Some(1),
                                    disc_number: Some(1),
                                    explicit: false,
                                    uri: lt.uri,
                                    is_playable: Some(true),
                                    is_local: false,
                                    ..Track::default()
                                })
                            };
                            Ok(Queue {
                                currently_playing: current.map(map_lt),
                                queue: upcoming.into_iter().map(map_lt).collect(),
                            })
                        }
                        None => Ok(Queue {
                            currently_playing: None,
                            queue: Vec::new(),
                        }),
                    };
                    ApiResponse::Queue { seq, result: res }
                }

                ApiRequest::Search { query, serial }
                | ApiRequest::SearchCatalogue { query, serial } => {
                    let trimmed = query.trim().to_string();
                    if trimmed.is_empty() {
                        ApiResponse::Search {
                            query,
                            serial,
                            result: Ok(SearchResults::default()),
                        }
                    } else {
                        let db_local = Arc::clone(&db);
                        let q_local = trimmed.clone();
                        let local_tracks = tokio::task::spawn_blocking(move || {
                            db_local.search_local(&q_local).unwrap_or_default()
                        })
                        .await
                        .unwrap_or_default();
                        let res = match extractor.search_youtube(&trimmed, 25).await {
                            Ok(remote_tracks) => {
                                let db2 = Arc::clone(&db);
                                let to_upsert = remote_tracks.clone();
                                let _ = tokio::task::spawn_blocking(move || {
                                    let _ = db2.upsert_tracks(&to_upsert);
                                })
                                .await;
                                let mut seen = std::collections::HashSet::new();
                                let mut combined =
                                    Vec::with_capacity(remote_tracks.len() + local_tracks.len());
                                for t in remote_tracks.into_iter().chain(local_tracks.into_iter()) {
                                    if seen.insert(t.id.clone()) {
                                        combined.push(t);
                                    }
                                }
                                let sp_tracks: Vec<Track> =
                                    combined.iter().map(to_spotify_track).collect();
                                let mut seen_artists = BTreeMap::<String, Artist>::new();
                                for t in &combined {
                                    let key = encode_artist_id(&t.artist);
                                    seen_artists.entry(key).or_insert_with(|| {
                                        artist_from_name(
                                            &t.artist,
                                            (!t.thumbnail.is_empty())
                                                .then_some(t.thumbnail.as_str()),
                                        )
                                    });
                                }
                                let artists: Vec<Artist> = seen_artists.into_values().collect();
                                Ok(SearchResults {
                                    tracks: Some(Page {
                                        total: sp_tracks.len() as u32,
                                        limit: 50,
                                        offset: 0,
                                        next: None,
                                        items: sp_tracks,
                                    }),
                                    artists: Some(Page {
                                        total: artists.len() as u32,
                                        limit: 50,
                                        offset: 0,
                                        next: None,
                                        items: artists,
                                    }),
                                    albums: Some(empty_page(0, 50)),
                                    playlists: Some(empty_page(0, 50)),
                                    shows: Some(empty_page(0, 50)),
                                    episodes: Some(empty_page(0, 50)),
                                })
                            }
                            Err(_e) if !local_tracks.is_empty() => {
                                let sp_tracks: Vec<Track> =
                                    local_tracks.iter().map(to_spotify_track).collect();
                                Ok(SearchResults {
                                    tracks: Some(Page {
                                        total: sp_tracks.len() as u32,
                                        limit: 50,
                                        offset: 0,
                                        next: None,
                                        items: sp_tracks,
                                    }),
                                    artists: Some(empty_page(0, 50)),
                                    albums: Some(empty_page(0, 50)),
                                    playlists: Some(empty_page(0, 50)),
                                    shows: Some(empty_page(0, 50)),
                                    episodes: Some(empty_page(0, 50)),
                                })
                            }
                            Err(e) => Err(ApiError::Network(e.to_string())),
                        };
                        ApiResponse::Search {
                            query,
                            serial,
                            result: res,
                        }
                    }
                }

                ApiRequest::SearchPlaylists { query, serial } => {
                    let q_lc = query.to_ascii_lowercase();
                    let db2 = Arc::clone(&db);
                    let res = tokio::task::spawn_blocking(move || db2.list_playlists())
                        .await
                        .map_err(|e| ApiError::Network(e.to_string()))
                        .and_then(|r| r.map_err(|e| ApiError::Network(e.to_string())))
                        .map(|pls| {
                            let items: Vec<Playlist> = pls
                                .iter()
                                .filter(|p| p.name.to_ascii_lowercase().contains(&q_lc))
                                .map(to_spotify_playlist)
                                .collect();
                            Page {
                                total: items.len() as u32,
                                limit: 50,
                                offset: 0,
                                next: None,
                                items,
                            }
                        });
                    ApiResponse::SearchPlaylists {
                        query,
                        serial,
                        result: res,
                    }
                }

                ApiRequest::PlaySearch {
                    kind: _,
                    query,
                    serial,
                } => {
                    let res = extractor
                        .search_youtube(&query, 10)
                        .await
                        .map(|tracks| {
                            let _ = db.upsert_tracks(&tracks);
                            tracks.first().map(|t| format!("spotify:track:{}", t.id))
                        })
                        .map_err(|e| ApiError::Network(e.to_string()));
                    ApiResponse::PlaySearch {
                        query,
                        serial,
                        result: res,
                    }
                }

                ApiRequest::MyPlaylists { offset, generation } => {
                    let db2 = Arc::clone(&db);
                    let res = tokio::task::spawn_blocking(move || db2.list_playlists())
                        .await
                        .map_err(|e| ApiError::Network(e.to_string()))
                        .and_then(|r| r.map_err(|e| ApiError::Network(e.to_string())))
                        .map(|pls| {
                            let total = pls.len() as u32;
                            let items: Vec<Playlist> = pls
                                .iter()
                                .skip(offset as usize)
                                .take(PLAYLIST_PAGE_SIZE as usize)
                                .map(to_spotify_playlist)
                                .collect();
                            let next = (offset as usize + items.len() < total as usize)
                                .then(|| format!("offset={}", offset + PLAYLIST_PAGE_SIZE));
                            Page {
                                items,
                                total,
                                limit: PLAYLIST_PAGE_SIZE,
                                offset,
                                next,
                            }
                        });
                    ApiResponse::MyPlaylists {
                        offset,
                        generation,
                        result: res,
                    }
                }

                ApiRequest::Playlist { id, generation } => {
                    let cached = explore_cache
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .get(&id)
                        .cloned();
                    if let Some(tracks) = cached {
                        let name = explore_titles
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .get(&id)
                            .cloned()
                            .unwrap_or_else(|| "Resonix Mix".into());
                        let images = tracks
                            .first()
                            .map(|t| thumbnail_images(&t.thumbnail))
                            .unwrap_or_default();
                        let total = tracks.len() as u32;
                        let pl = Playlist {
                            id: id.clone(),
                            name,
                            description: None,
                            images,
                            owner: Owner {
                                id: Some("local".into()),
                                display_name: Some("Resonix".into()),
                                uri: None,
                            },
                            public: Some(false),
                            collaborative: false,
                            snapshot_id: Some("explore".into()),
                            tracks: Some(TrackCount { total }),
                            items_count: Some(TrackCount { total }),
                            uri: format!("spotify:playlist:{id}"),
                            ..Playlist::default()
                        };
                        ApiResponse::Playlist {
                            id,
                            generation,
                            result: Ok(pl),
                        }
                    } else if let Ok(pid) = id.parse::<i64>() {
                        let db2 = Arc::clone(&db);
                        let res = tokio::task::spawn_blocking(move || db2.get_playlist(pid))
                            .await
                            .map_err(|e| ApiError::Network(e.to_string()))
                            .and_then(|r| r.map_err(|e| ApiError::Network(e.to_string())))
                            .and_then(|opt| match opt {
                                Some(pl) => Ok(to_spotify_playlist(&pl)),
                                None => Err(ApiError::Status {
                                    status: 404,
                                    message: "Playlist not found".into(),
                                }),
                            });
                        ApiResponse::Playlist {
                            id,
                            generation,
                            result: res,
                        }
                    } else {
                        ApiResponse::Playlist {
                            id,
                            generation,
                            result: Err(ApiError::Status {
                                status: 404,
                                message: "Playlist not found".into(),
                            }),
                        }
                    }
                }

                ApiRequest::PlaylistItems {
                    id,
                    offset,
                    generation,
                } => {
                    let cached = explore_cache
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .get(&id)
                        .cloned();
                    if let Some(tracks) = cached {
                        let total = tracks.len() as u32;
                        let items: Vec<PlaylistItem> = tracks
                            .iter()
                            .skip(offset as usize)
                            .take(PLAYLIST_PAGE_SIZE as usize)
                            .map(to_playlist_item)
                            .collect();
                        let next = (offset as usize + items.len() < total as usize)
                            .then(|| format!("offset={}", offset + PLAYLIST_PAGE_SIZE));
                        ApiResponse::PlaylistItems {
                            id,
                            offset,
                            generation,
                            result: Ok(Page {
                                items,
                                total,
                                limit: PLAYLIST_PAGE_SIZE,
                                offset,
                                next,
                            }),
                        }
                    } else if let Ok(pid) = id.parse::<i64>() {
                        let db2 = Arc::clone(&db);
                        let res = tokio::task::spawn_blocking(move || db2.get_playlist_tracks(pid))
                            .await
                            .map_err(|e| ApiError::Network(e.to_string()))
                            .and_then(|r| r.map_err(|e| ApiError::Network(e.to_string())))
                            .map(|tracks| {
                                let total = tracks.len() as u32;
                                let items: Vec<PlaylistItem> = tracks
                                    .iter()
                                    .skip(offset as usize)
                                    .take(PLAYLIST_PAGE_SIZE as usize)
                                    .map(to_playlist_item)
                                    .collect();
                                let next = (offset as usize + items.len() < total as usize)
                                    .then(|| format!("offset={}", offset + PLAYLIST_PAGE_SIZE));
                                Page {
                                    items,
                                    total,
                                    limit: PLAYLIST_PAGE_SIZE,
                                    offset,
                                    next,
                                }
                            });
                        ApiResponse::PlaylistItems {
                            id,
                            offset,
                            generation,
                            result: res,
                        }
                    } else {
                        ApiResponse::PlaylistItems {
                            id,
                            offset,
                            generation,
                            result: Ok(empty_page(offset, PLAYLIST_PAGE_SIZE)),
                        }
                    }
                }

                ApiRequest::PlaylistSample { id, generation, .. } => ApiResponse::PlaylistSample {
                    id,
                    generation,
                    result: Ok(empty_page(0, PLAYLIST_PAGE_SIZE)),
                },

                ApiRequest::CreatePlaylist {
                    name,
                    public: _,
                    description: _,
                } => {
                    let db2 = Arc::clone(&db);
                    let res = tokio::task::spawn_blocking(move || db2.create_playlist(&name, ""))
                        .await
                        .map_err(|e| ApiError::Network(e.to_string()))
                        .and_then(|r| r.map_err(|e| ApiError::Network(e.to_string())))
                        .map(|pl| to_spotify_playlist(&pl));
                    if res.is_ok() {
                        let _ = engine_cmd_target.send(Command::Rootlist);
                    }
                    ApiResponse::PlaylistCreated(res)
                }

                ApiRequest::UpdatePlaylist {
                    id,
                    name,
                    description: _,
                    public: _,
                } => {
                    let db2 = Arc::clone(&db);
                    let id2 = id.clone();
                    let res = tokio::task::spawn_blocking(move || {
                        if let Ok(pid) = id2.parse::<i64>()
                            && let Some(n) = name
                        {
                            db2.rename_playlist(pid, &n)?;
                        }
                        Ok::<(), crate::youtube::error::AppError>(())
                    })
                    .await
                    .map_err(|e| ApiError::Network(e.to_string()))
                    .and_then(|r| r.map_err(|e| ApiError::Network(e.to_string())));
                    if res.is_ok() {
                        let _ = engine_cmd_target.send(Command::Rootlist);
                    }
                    ApiResponse::PlaylistUpdated { id, result: res }
                }

                ApiRequest::UploadPlaylistCover {
                    id,
                    request,
                    previous_urls,
                    cover,
                } => ApiResponse::PlaylistCoverUploaded {
                    id,
                    request,
                    previous_urls,
                    cover,
                    result: Ok(()),
                },

                ApiRequest::CheckPlaylistDuplicates {
                    playlist_id,
                    playlist_name,
                    items,
                    position,
                } => {
                    let db2 = Arc::clone(&db);
                    let pid_str = playlist_id.clone();
                    let existing: BTreeSet<String> = tokio::task::spawn_blocking(move || {
                        pid_str
                            .parse::<i64>()
                            .ok()
                            .and_then(|pid| db2.get_playlist_tracks(pid).ok())
                            .map(|tr| tr.into_iter().map(|t| t.id).collect())
                            .unwrap_or_default()
                    })
                    .await
                    .unwrap_or_default();
                    let dups: Vec<String> = items
                        .iter()
                        .filter_map(|item| {
                            let uri = item.uri();
                            let vid = video_id_from_uri(uri)?;
                            existing.contains(vid).then(|| uri.to_string())
                        })
                        .collect();
                    ApiResponse::PlaylistDuplicatesChecked {
                        playlist_id,
                        playlist_name,
                        items,
                        position,
                        result: Ok(dups),
                    }
                }

                ApiRequest::AddToPlaylist {
                    playlist_id,
                    playlist_name,
                    uris,
                    position: _,
                } => {
                    let db2 = Arc::clone(&db);
                    let pid_str = playlist_id.clone();
                    let res = tokio::task::spawn_blocking(move || {
                        if let Ok(pid) = pid_str.parse::<i64>() {
                            for uri in &uris {
                                if let Some(vid) = video_id_from_uri(uri) {
                                    db2.add_to_playlist(pid, vid)?;
                                }
                            }
                        }
                        Ok::<Option<String>, crate::youtube::error::AppError>(Some("local".into()))
                    })
                    .await
                    .map_err(|e| ApiError::Network(e.to_string()))
                    .and_then(|r| r.map_err(|e| ApiError::Network(e.to_string())));
                    ApiResponse::PlaylistItemsChanged {
                        id: playlist_id,
                        message: format!("Added to {playlist_name}"),
                        result: res,
                    }
                }

                ApiRequest::RemoveFromPlaylist {
                    playlist_id, uris, ..
                } => {
                    let db2 = Arc::clone(&db);
                    let pid_str = playlist_id.clone();
                    let res = tokio::task::spawn_blocking(move || {
                        if let Ok(pid) = pid_str.parse::<i64>() {
                            for uri in &uris {
                                if let Some(vid) = video_id_from_uri(uri) {
                                    db2.remove_from_playlist(pid, vid)?;
                                }
                            }
                        }
                        Ok::<Option<String>, crate::youtube::error::AppError>(Some("local".into()))
                    })
                    .await
                    .map_err(|e| ApiError::Network(e.to_string()))
                    .and_then(|r| r.map_err(|e| ApiError::Network(e.to_string())));
                    ApiResponse::PlaylistItemsChanged {
                        id: playlist_id,
                        message: "Removed from playlist".into(),
                        result: res,
                    }
                }

                ApiRequest::ReorderPlaylist {
                    playlist_id,
                    range_start,
                    insert_before,
                    ..
                } => {
                    let db2 = Arc::clone(&db);
                    let pid_str = playlist_id.clone();
                    let res = tokio::task::spawn_blocking(move || {
                        if let Ok(pid) = pid_str.parse::<i64>() {
                            let tracks = db2.get_playlist_tracks(pid)?;
                            let mut ids: Vec<String> = tracks.into_iter().map(|t| t.id).collect();
                            let from = (range_start as usize).min(ids.len().saturating_sub(1));
                            if !ids.is_empty() {
                                let item = ids.remove(from);
                                let to = if (insert_before as usize) > from {
                                    (insert_before as usize).saturating_sub(1)
                                } else {
                                    insert_before as usize
                                }
                                .min(ids.len());
                                ids.insert(to, item);
                                db2.reorder_playlist_tracks(pid, &ids)?;
                            }
                        }
                        Ok::<Option<String>, crate::youtube::error::AppError>(Some("local".into()))
                    })
                    .await
                    .map_err(|e| ApiError::Network(e.to_string()))
                    .and_then(|r| r.map_err(|e| ApiError::Network(e.to_string())));
                    ApiResponse::PlaylistItemsChanged {
                        id: playlist_id,
                        message: "Playlist updated".into(),
                        result: res,
                    }
                }

                ApiRequest::FollowPlaylist { id, follow } => {
                    if !follow && let Ok(pid) = id.parse::<i64>() {
                        let db2 = Arc::clone(&db);
                        let _ = tokio::task::spawn_blocking(move || db2.delete_playlist(pid)).await;
                        let _ = engine_cmd_target.send(Command::Rootlist);
                    }
                    ApiResponse::PlaylistFollowChanged {
                        id,
                        followed: follow,
                        result: Ok(()),
                    }
                }

                ApiRequest::SavedTracks { offset, generation } => {
                    let db2 = Arc::clone(&db);
                    let res = tokio::task::spawn_blocking(move || {
                        let (liked, total) = db2.liked_tracks(offset, PLAYLIST_PAGE_SIZE)?;
                        let items: Vec<SavedTrack> = liked
                            .iter()
                            .map(|t| SavedTrack {
                                added_at: Some("1970-01-01T00:00:00Z".into()),
                                track: to_spotify_track(t),
                            })
                            .collect();
                        let next = (offset as usize + items.len() < total as usize)
                            .then(|| format!("offset={}", offset + PLAYLIST_PAGE_SIZE));
                        Ok::<Page<SavedTrack>, crate::youtube::error::AppError>(Page {
                            items,
                            total,
                            limit: PLAYLIST_PAGE_SIZE,
                            offset,
                            next,
                        })
                    })
                    .await
                    .map_err(|e| ApiError::Network(e.to_string()))
                    .and_then(|r| r.map_err(|e| ApiError::Network(e.to_string())));
                    ApiResponse::SavedTracks {
                        offset,
                        generation,
                        account_id: Some("local".into()),
                        result: res,
                    }
                }

                ApiRequest::SetSaved { uris, saved } => {
                    let db2 = Arc::clone(&db);
                    let uris2 = uris.clone();
                    let res = tokio::task::spawn_blocking(move || {
                        for uri in &uris2 {
                            if let Some(vid) = video_id_from_uri(uri) {
                                db2.set_liked(vid, saved)?;
                            }
                        }
                        Ok::<(), crate::youtube::error::AppError>(())
                    })
                    .await
                    .map_err(|e| ApiError::Network(e.to_string()))
                    .and_then(|r| r.map_err(|e| ApiError::Network(e.to_string())));
                    ApiResponse::SavedChanged {
                        uris,
                        saved,
                        result: res,
                    }
                }

                ApiRequest::Contains { uris } => {
                    let db2 = Arc::clone(&db);
                    let uris2 = uris.clone();
                    let res = tokio::task::spawn_blocking(move || {
                        let mut out = Vec::with_capacity(uris2.len());
                        for uri in &uris2 {
                            let liked = video_id_from_uri(uri)
                                .and_then(|vid| db2.is_liked(vid).ok())
                                .unwrap_or(false);
                            out.push(liked);
                        }
                        out
                    })
                    .await
                    .map_err(|e| ApiError::Network(e.to_string()));
                    ApiResponse::Contains { uris, result: res }
                }

                ApiRequest::RecentlyPlayed {
                    who,
                    generation,
                    before: _,
                    limit,
                } => {
                    let db2 = Arc::clone(&db);
                    let res =
                        tokio::task::spawn_blocking(move || db2.recently_played(limit as usize))
                            .await
                            .map_err(|e| ApiError::Network(e.to_string()))
                            .and_then(|r| r.map_err(|e| ApiError::Network(e.to_string())))
                            .map(|tracks| CursorPage {
                                total: Some(tracks.len() as u32),
                                items: tracks
                                    .iter()
                                    .map(|t| PlayHistory {
                                        track: to_spotify_track(t),
                                        played_at: Some("1970-01-01T00:00:00Z".into()),
                                        context: None,
                                    })
                                    .collect(),
                                next: None,
                                cursors: None,
                            });
                    ApiResponse::RecentlyPlayed {
                        who,
                        generation,
                        limit,
                        result: res,
                    }
                }

                ApiRequest::TopTracks {
                    period,
                    offset,
                    full,
                    generation,
                } => {
                    let db2 = Arc::clone(&db);
                    let res = tokio::task::spawn_blocking(move || db2.top_tracks(50))
                        .await
                        .map_err(|e| ApiError::Network(e.to_string()))
                        .and_then(|r| r.map_err(|e| ApiError::Network(e.to_string())))
                        .map(|tracks| {
                            let items: Vec<Track> = tracks.iter().map(to_spotify_track).collect();
                            Page {
                                total: items.len() as u32,
                                limit: 50,
                                offset,
                                next: None,
                                items,
                            }
                        });
                    ApiResponse::TopTracks {
                        period,
                        offset,
                        full,
                        generation,
                        result: res,
                    }
                }

                ApiRequest::TopArtists { generation } => {
                    let db2 = Arc::clone(&db);
                    let res = tokio::task::spawn_blocking(move || db2.top_tracks(100))
                        .await
                        .map_err(|e| ApiError::Network(e.to_string()))
                        .and_then(|r| r.map_err(|e| ApiError::Network(e.to_string())))
                        .map(|tracks| {
                            let mut map = BTreeMap::<String, Artist>::new();
                            for t in &tracks {
                                let key = encode_artist_id(&t.artist);
                                map.entry(key).or_insert_with(|| {
                                    artist_from_name(
                                        &t.artist,
                                        (!t.thumbnail.is_empty()).then_some(t.thumbnail.as_str()),
                                    )
                                });
                            }
                            map.into_values().collect()
                        });
                    ApiResponse::TopArtists {
                        generation,
                        result: res,
                    }
                }

                ApiRequest::MadeForYou { generation, force } => {
                    let cached_sections = (!force)
                        .then(|| {
                            explore_sections
                                .lock()
                                .unwrap_or_else(|p| p.into_inner())
                                .clone()
                        })
                        .filter(|s| !s.is_empty());
                    let sections: Vec<crate::youtube::ExploreSection> = match cached_sections {
                        Some(s) => s
                            .into_iter()
                            .map(|(title, tracks)| crate::youtube::ExploreSection { title, tracks })
                            .collect(),
                        None => explore::build_explore(&db, &extractor)
                            .await
                            .unwrap_or_default(),
                    };
                    let playlists = {
                        let mut cache = explore_cache.lock().unwrap_or_else(|p| p.into_inner());
                        let mut titles = explore_titles.lock().unwrap_or_else(|p| p.into_inner());
                        let mut sec_cache =
                            explore_sections.lock().unwrap_or_else(|p| p.into_inner());
                        build_explore_playlists(&sections, &mut cache, &mut titles, &mut sec_cache)
                    };
                    ApiResponse::MadeForYou {
                        generation,
                        personalized: true,
                        result: Ok(playlists),
                        top_mixes: Ok(Vec::new()),
                    }
                }

                ApiRequest::Downloads => {
                    let db2 = Arc::clone(&db);
                    let res = tokio::task::spawn_blocking(move || load_downloads_library(&db2))
                        .await
                        .map_err(|e| ApiError::Network(e.to_string()))
                        .and_then(|r| r.map_err(ApiError::Network));
                    ApiResponse::Downloads(res)
                }

                ApiRequest::Recommendations {
                    seed_tracks,
                    seed_artists,
                    generation,
                } => {
                    let mut query = "popular indie music".to_string();
                    if let Some(vid) = seed_tracks.first()
                        && let Ok(Some(t)) = db.get_track_by_id(vid)
                    {
                        query = format!("{} mix", t.artist);
                    } else if let Some(aid) = seed_artists.first()
                        && let Some(name) = decode_artist_id(aid)
                    {
                        query = name;
                    }
                    let res = extractor
                        .search_youtube(&query, 20)
                        .await
                        .map(|tracks| {
                            let _ = db.upsert_tracks(&tracks);
                            tracks.iter().map(to_spotify_track).collect()
                        })
                        .map_err(|e| ApiError::Network(e.to_string()));
                    ApiResponse::Recommendations {
                        generation,
                        result: res,
                    }
                }

                ApiRequest::Track { id } => {
                    let existing = db.get_track_by_id(&id).ok().flatten();
                    let res = match existing {
                        Some(t) => Ok(to_spotify_track(&t)),
                        None => match extractor.metadata(&id).await {
                            Ok(t) => {
                                let _ = db.upsert_tracks(&[t.clone()]);
                                Ok(to_spotify_track(&t))
                            }
                            Err(e) => Err(ApiError::Network(e.to_string())),
                        },
                    };
                    ApiResponse::Track { id, result: res }
                }

                ApiRequest::Artist { id } => {
                    let name = decode_artist_id(&id).unwrap_or_else(|| id.clone());
                    let artist = artist_from_name(&name, None);
                    ApiResponse::Artist {
                        id,
                        result: Ok(artist),
                    }
                }

                ApiRequest::ArtistTopTracks { id, generation } => {
                    let name = decode_artist_id(&id).unwrap_or_else(|| id.clone());
                    let res = extractor
                        .search_youtube(&name, 20)
                        .await
                        .map(|tracks| {
                            let _ = db.upsert_tracks(&tracks);
                            tracks.iter().take(20).map(to_spotify_track).collect()
                        })
                        .map_err(|e| ApiError::Network(e.to_string()));
                    ApiResponse::ArtistTopTracks {
                        id,
                        generation,
                        result: res,
                    }
                }

                ApiRequest::ArtistAlbums { id, groups, offset } => ApiResponse::ArtistAlbums {
                    id,
                    groups,
                    offset,
                    result: Ok(empty_page(offset, 50)),
                },

                ApiRequest::RelatedArtists { id } => ApiResponse::RelatedArtists {
                    id,
                    result: Ok(Vec::new()),
                },

                ApiRequest::Album { id } => {
                    let vid = id.strip_prefix("yt_").unwrap_or(&id).to_string();
                    let db2 = Arc::clone(&db);
                    let res = tokio::task::spawn_blocking(move || db2.get_track_by_id(&vid))
                        .await
                        .ok()
                        .and_then(|r| r.ok())
                        .flatten()
                        .and_then(|t| to_spotify_track(&t).album);
                    ApiResponse::Album {
                        id,
                        result: res.ok_or_else(|| ApiError::Status {
                            status: 404,
                            message: "Track info not found".into(),
                        }),
                    }
                }

                ApiRequest::AlbumTracks {
                    id,
                    offset,
                    generation,
                } => {
                    let vid = id.strip_prefix("yt_").unwrap_or(&id).to_string();
                    let db2 = Arc::clone(&db);
                    let items = if offset == 0 {
                        tokio::task::spawn_blocking(move || db2.get_track_by_id(&vid))
                            .await
                            .ok()
                            .and_then(|r| r.ok())
                            .flatten()
                            .map(|t| vec![to_spotify_track(&t)])
                            .unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                    let total = items.len() as u32;
                    ApiResponse::AlbumTracks {
                        id,
                        offset,
                        generation,
                        result: Ok(Page {
                            total,
                            limit: 50,
                            offset,
                            next: None,
                            items,
                        }),
                    }
                }

                ApiRequest::AlbumQueueTracks {
                    id: _,
                    offset,
                    request,
                } => ApiResponse::AlbumQueueTracks {
                    offset,
                    request,
                    result: Ok(empty_page(offset, 50)),
                },

                ApiRequest::Show { id } => ApiResponse::Show {
                    id,
                    result: Err(ApiError::Status {
                        status: 404,
                        message: "Podcasts are unavailable in Resonix".into(),
                    }),
                },

                ApiRequest::ShowEpisodes { id, offset } => ApiResponse::ShowEpisodes {
                    id,
                    offset,
                    result: Ok(empty_page(offset, 50)),
                },

                ApiRequest::Episode { id } => ApiResponse::Episode {
                    id,
                    result: Err(ApiError::Status {
                        status: 404,
                        message: "Episodes are unavailable in Resonix".into(),
                    }),
                },

                ApiRequest::HomeEpisodes { generation, .. } => ApiResponse::HomeEpisodes {
                    generation,
                    result: Ok(Vec::new()),
                },

                ApiRequest::NewReleases { generation } => ApiResponse::NewReleases {
                    generation,
                    result: Ok(Vec::new()),
                },

                ApiRequest::NewReleaseTracks { generation, .. } => ApiResponse::NewReleaseTracks {
                    generation,
                    result: Ok(Vec::new()),
                },

                ApiRequest::HiddenSongs { generation } => ApiResponse::HiddenSongs {
                    generation,
                    result: Ok(Vec::new()),
                },

                ApiRequest::SetHiddenInPlaylist {
                    playlist_uri,
                    track_uri,
                    hidden,
                } => ApiResponse::HiddenInPlaylistSet {
                    playlist_uri,
                    track_uri,
                    hidden,
                    result: Ok(()),
                },

                ApiRequest::SavedAlbums { offset } => ApiResponse::SavedAlbums {
                    offset,
                    result: Ok(empty_page(offset, 50)),
                },

                ApiRequest::FollowedArtists { after } => ApiResponse::FollowedArtists {
                    after,
                    result: Ok(CursorPage {
                        total: Some(0),
                        items: Vec::new(),
                        next: None,
                        cursors: None,
                    }),
                },

                ApiRequest::SavedShows { offset } => ApiResponse::SavedShows {
                    offset,
                    result: Ok(empty_page(offset, 50)),
                },

                ApiRequest::SavedEpisodes { offset } => ApiResponse::SavedEpisodes {
                    offset,
                    result: Ok(empty_page(offset, 50)),
                },

                ApiRequest::Remote {
                    action,
                    device_id: _,
                    play,
                    position_ms,
                    percent,
                    flag,
                    repeat,
                } => {
                    let cmd = match action {
                        RemoteAction::Play => match play {
                            Some(req) => Some(PlayerCommand::Load(crate::player::LoadSpec {
                                context_uri: req.context_uri,
                                uris: req.uris,
                                offset_uri: req.offset_uri,
                                offset_index: req.offset_position,
                                position_ms: req.position_ms,
                                play: true,
                                ..Default::default()
                            })),
                            None => Some(PlayerCommand::Toggle),
                        },
                        RemoteAction::Pause => Some(PlayerCommand::Toggle),
                        RemoteAction::Next => Some(PlayerCommand::Next),
                        RemoteAction::Previous => Some(PlayerCommand::Previous),
                        RemoteAction::Seek => Some(PlayerCommand::Seek(position_ms)),
                        RemoteAction::Volume { .. } => {
                            let v =
                                (u32::from(percent.min(100)) * u32::from(u16::MAX) / 100) as u16;
                            Some(PlayerCommand::Volume(v))
                        }
                        RemoteAction::Shuffle => Some(PlayerCommand::Shuffle(flag)),
                        RemoteAction::Repeat => {
                            Some(PlayerCommand::Repeat(RepeatMode::from_api(&repeat)))
                        }
                    };
                    if let Some(c) = cmd {
                        let _ = engine_cmd_target.send(Command::Player(c));
                    }
                    ApiResponse::Remote {
                        action,
                        result: Ok(()),
                    }
                }

                ApiRequest::ShufflePlay { play, .. } => {
                    let _ = engine_cmd_target.send(Command::Player(PlayerCommand::Load(
                        crate::player::LoadSpec {
                            context_uri: play.context_uri,
                            uris: play.uris,
                            offset_uri: play.offset_uri,
                            offset_index: play.offset_position,
                            position_ms: play.position_ms,
                            play: true,
                            shuffle: Some(true),
                            ..Default::default()
                        },
                    )));
                    ApiResponse::Remote {
                        action: RemoteAction::Play,
                        result: Ok(()),
                    }
                }

                ApiRequest::Transfer { device_id, .. } => ApiResponse::Transferred {
                    device_id,
                    result: Ok(()),
                },

                ApiRequest::AddToQueue { uri, label, .. } => {
                    let _ = engine_cmd_target.send(Command::Player(PlayerCommand::AddToQueue(uri)));
                    ApiResponse::QueueAdded {
                        label,
                        result: Ok(()),
                    }
                }

                ApiRequest::AddManyToQueue { request, uris, .. } => {
                    let count = uris.len();
                    for uri in uris {
                        let _ =
                            engine_cmd_target.send(Command::Player(PlayerCommand::AddToQueue(uri)));
                    }
                    ApiResponse::QueueBatchAdded {
                        request,
                        added: count,
                        result: Ok(()),
                    }
                }
            };

            let _ = events.send(Event::Api(Box::new(response)));
            waker.wake();
        });
    }
}
