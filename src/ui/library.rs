//! Full-page library grids: unified library overview, albums, artists, podcasts, episodes.

use crate::api::models::{join_names, pick_image};
use crate::app::App;
use crate::i18n::{gettext, ngettext};
use crate::model::{Action, Dialog, Loadable, Page};
use crate::settings::LibraryShelf as Filter;
use crate::theme::{self, Icon};

use super::widgets;

pub fn show(app: &mut App, ui: &mut egui::Ui, page: Page) {
    if page == Page::Library {
        show_library_page(app, ui);
        return;
    }
    let palette = app.palette;
    ui.add_space(8.0);
    let locale = app.locale;
    let (title, empty_title, empty_body) = match page {
        Page::Albums => (
            gettext(locale, "Albums"),
            gettext(locale, "No saved albums"),
            gettext(locale, "Saved albums appear here."),
        ),
        Page::Artists => (
            gettext(locale, "Artists"),
            gettext(locale, "No followed artists"),
            gettext(locale, "Followed artists appear here."),
        ),
        Page::Podcasts => (
            gettext(locale, "Podcasts"),
            gettext(locale, "No podcasts yet"),
            gettext(locale, "Followed podcasts appear here."),
        ),
        _ => (
            gettext(locale, "Episodes"),
            gettext(locale, "No saved episodes"),
            gettext(locale, "Saved episodes appear here."),
        ),
    };
    theme::text(ui, title.as_ref(), theme::bold(28.0), palette.text);
    ui.add_space(14.0);
    match page {
        Page::Albums => {
            show_albums_grid(app, ui, Page::Albums, &empty_title, &empty_body);
        }
        Page::Artists => {
            show_artists_grid(app, ui, Page::Artists, &empty_title, &empty_body);
        }
        Page::Podcasts => {
            let card_height = widgets::card_row_height(ui);
            let count = app.library.shows.items.len();
            widgets::virtual_wrapped_cards(ui, count, card_height, |ui, index| {
                let show = app.library.shows.items[index].show.clone();
                let id = show.id.clone();
                let card_art = app.art_px(widgets::CARD_ART);
                let card = widgets::card(
                    ui,
                    app,
                    pick_image(&show.images, card_art),
                    &show.name,
                    &show.publisher,
                    widgets::CardCover::default(),
                );
                if card.clicked {
                    app.actions.push(Action::Open(Page::Show(id)));
                }
                egui::Popup::context_menu(&card.response)
                    .id(ui.make_persistent_id(("library-show-menu", &show.uri)))
                    .frame(widgets::menu_frame(&palette))
                    .show(|ui| widgets::context_menu_items(ui, app, &show.uri, &show.name, None));
            });
            let list = &app.library.shows;
            let (loading, error, can_load, empty) = (
                list.loading,
                list.error.clone(),
                list.can_load_more(),
                list.items.is_empty() && list.loaded_once,
            );
            footer(
                app,
                ui,
                page,
                loading,
                error,
                can_load,
                empty,
                &empty_title,
                &empty_body,
                Icon::Mic,
            );
        }
        _ => {
            let episodes: Vec<_> = app
                .library
                .episodes
                .items
                .iter()
                .map(|saved| saved.episode.clone())
                .collect();
            widgets::virtual_rows(
                ui,
                episodes.len(),
                super::show::EPISODE_ROW_HEIGHT,
                |ui, index| {
                    super::show::episode_row(app, ui, &episodes[index], None);
                },
            );
            let list = &app.library.episodes;
            let (loading, error, can_load, empty) = (
                list.loading,
                list.error.clone(),
                list.can_load_more(),
                list.items.is_empty() && list.loaded_once,
            );
            footer(
                app,
                ui,
                page,
                loading,
                error,
                can_load,
                empty,
                &empty_title,
                &empty_body,
                Icon::Bookmark,
            );
        }
    }
}

fn show_library_page(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let locale = app.locale;
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        theme::text(
            ui,
            gettext(locale, "Library").as_ref(),
            theme::bold(28.0),
            palette.text,
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if theme::pill_button(ui, &palette, gettext(locale, "New playlist").as_ref(), true)
                .clicked()
            {
                app.actions.push(Action::ShowDialog(Dialog::CreatePlaylist {
                    name: String::new(),
                    public: true,
                    add_uris: Vec::new(),
                }));
            }
        });
    });
    ui.add_space(10.0);

    let has_albums = !app.library.albums.items.is_empty();
    let has_artists = !app.library.artists.items.is_empty();
    let filter_id = egui::Id::new("library-page-filter");
    let mut filter = ui
        .data(|data| data.get_temp::<Filter>(filter_id))
        .unwrap_or(Filter::Playlists);
    if (!has_albums && filter == Filter::Albums) || (!has_artists && filter == Filter::Artists) {
        filter = Filter::Playlists;
    }

    if has_albums || has_artists {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);
            let mut pills = vec![(Filter::Playlists, gettext(locale, "Playlists"))];
            if has_albums {
                pills.push((Filter::Albums, gettext(locale, "Albums")));
            }
            if has_artists {
                pills.push((Filter::Artists, gettext(locale, "Artists")));
            }
            for (value, label) in pills {
                if theme::soft_button(ui, &palette, None, &label, filter == value).clicked() {
                    filter = value;
                }
            }
        });
        ui.add_space(14.0);
    } else {
        ui.add_space(4.0);
    }
    ui.data_mut(|data| data.insert_temp(filter_id, filter));

    match filter {
        Filter::Albums => {
            if !app.library.albums.loading
                && app.library.albums.error.is_none()
                && !app.library.albums.loaded_once
            {
                app.actions.push(Action::LoadMore(Page::Albums));
            }
            show_albums_grid(
                app,
                ui,
                Page::Albums,
                &gettext(locale, "No saved albums"),
                &gettext(locale, "Saved albums appear here."),
            );
        }
        Filter::Artists => {
            if !app.library.artists.loading
                && app.library.artists.error.is_none()
                && !app.library.artists.loaded_once
            {
                app.actions.push(Action::LoadMore(Page::Artists));
            }
            show_artists_grid(
                app,
                ui,
                Page::Artists,
                &gettext(locale, "No followed artists"),
                &gettext(locale, "Followed artists appear here."),
            );
        }
        _ => match &app.library.playlists {
            Loadable::NotLoaded | Loadable::Loading => {
                widgets::loading_row(ui, &palette, locale);
            }
            Loadable::Failed(error) => {
                let error = error.clone();
                widgets::error_row(ui, app, &error, Some(Page::Library));
            }
            Loadable::Loaded(playlists) => {
                let playlists = playlists.clone();
                // Index 0 is Liked Songs, followed by the user's playlists.
                let total_cards = 1 + playlists.len();
                let card_height = widgets::card_row_height(ui);
                let card_art = app.art_px(widgets::CARD_ART);
                widgets::virtual_wrapped_cards(ui, total_cards, card_height, |ui, index| {
                    if index == 0 {
                        let liked_title = gettext(locale, "Liked Songs");
                        let liked_sub = app
                            .library
                            .liked
                            .total
                            .map(|n| locale.song_count(n))
                            .unwrap_or_else(|| gettext(locale, "Playlist").into_owned());
                        let liked_uri = app
                            .user
                            .as_ref()
                            .map(|u| format!("spotify:user:{}:collection", u.id));
                        let context_here = liked_uri.as_deref().is_some()
                            && app.playing_context_uri().as_deref() == liked_uri.as_deref();
                        let playing_here = context_here && app.believed_playing();
                        let card = widgets::card(
                            ui,
                            app,
                            None,
                            &liked_title,
                            &liked_sub,
                            widgets::CardCover::liked(playing_here),
                        );
                        if card.play
                            && let Some(uri) = liked_uri.clone()
                        {
                            if context_here {
                                app.actions.push(Action::TogglePlay);
                            } else {
                                app.actions.push(Action::PlayContext {
                                    uri,
                                    offset_uri: None,
                                    offset_index: None,
                                });
                            }
                        }
                        if card.clicked {
                            app.actions.push(Action::Open(Page::LikedSongs));
                        }
                        if let Some(uri) = liked_uri {
                            egui::Popup::context_menu(&card.response)
                                .id(ui.make_persistent_id("library-liked-menu"))
                                .frame(widgets::menu_frame(&palette))
                                .show(|ui| {
                                    widgets::context_menu_items(ui, app, &uri, &liked_title, None);
                                });
                        }
                        return;
                    }

                    let playlist = &playlists[index - 1];
                    let total = playlist
                        .tracks
                        .as_ref()
                        .or(playlist.items_count.as_ref())
                        .map(|c| c.total);
                    let subtitle = if let Some(count) = total {
                        ngettext(locale, "{count} song", "{count} songs", count)
                            .replace("{count}", &count.to_string())
                    } else {
                        playlist.owner_name().to_string()
                    };
                    let context_here =
                        app.playing_context_uri().as_deref() == Some(playlist.uri.as_str());
                    let playing_here = context_here && app.believed_playing();
                    let card = widgets::card(
                        ui,
                        app,
                        pick_image(&playlist.images, card_art),
                        &playlist.name,
                        &subtitle,
                        widgets::CardCover::square(playing_here),
                    );
                    if card.play {
                        if context_here {
                            app.actions.push(Action::TogglePlay);
                        } else {
                            app.actions.push(Action::PlayContext {
                                uri: playlist.uri.clone(),
                                offset_uri: None,
                                offset_index: None,
                            });
                        }
                    }
                    if card.clicked {
                        app.actions
                            .push(Action::Open(Page::Playlist(playlist.id.clone())));
                    }
                    let owned = app
                        .user_id()
                        .is_some_and(|uid| playlist.owned_by(uid))
                        .then_some(playlist);
                    egui::Popup::context_menu(&card.response)
                        .id(ui.make_persistent_id(("library-page-menu", &playlist.uri)))
                        .frame(widgets::menu_frame(&palette))
                        .show(|ui| {
                            widgets::context_menu_items(
                                ui,
                                app,
                                &playlist.uri,
                                &playlist.name,
                                owned,
                            );
                        });
                });
            }
        },
    }
}

fn show_albums_grid(
    app: &mut App,
    ui: &mut egui::Ui,
    page: Page,
    empty_title: &str,
    empty_body: &str,
) {
    let palette = app.palette;
    let card_height = widgets::card_row_height(ui);
    let count = app.library.albums.items.len();
    widgets::virtual_wrapped_cards(ui, count, card_height, |ui, index| {
        let album = app.library.albums.items[index].album.clone();
        let id = album.id.clone();
        let uri = album.uri.clone();
        let subtitle = join_names(album.artists.iter().map(|artist| artist.name.as_str()));
        let context_here = app.playing_context_uri().as_deref() == Some(uri.as_str());
        let playing_here = context_here && app.believed_playing();
        let card_art = app.art_px(widgets::CARD_ART);
        let card = widgets::card(
            ui,
            app,
            pick_image(&album.images, card_art),
            &album.name,
            &subtitle,
            widgets::CardCover::square(playing_here),
        );
        if card.play {
            if context_here {
                app.actions.push(Action::TogglePlay);
            } else {
                app.actions.push(Action::PlayContext {
                    uri,
                    offset_uri: None,
                    offset_index: None,
                });
            }
        }
        if card.clicked {
            app.actions.push(Action::Open(Page::Album(id)));
        }
        egui::Popup::context_menu(&card.response)
            .id(ui.make_persistent_id(("library-album-menu", &album.uri)))
            .frame(widgets::menu_frame(&palette))
            .show(|ui| widgets::context_menu_items(ui, app, &album.uri, &album.name, None));
    });
    let list = &app.library.albums;
    let (loading, error, can_load, empty) = (
        list.loading,
        list.error.clone(),
        list.can_load_more(),
        list.items.is_empty() && list.loaded_once,
    );
    footer(
        app,
        ui,
        page,
        loading,
        error,
        can_load,
        empty,
        empty_title,
        empty_body,
        Icon::Disc,
    );
}

fn show_artists_grid(
    app: &mut App,
    ui: &mut egui::Ui,
    page: Page,
    empty_title: &str,
    empty_body: &str,
) {
    let palette = app.palette;
    let card_height = widgets::card_row_height(ui);
    let count = app.library.artists.items.len();
    let artist_label = gettext(app.locale, "Artist");
    widgets::virtual_wrapped_cards(ui, count, card_height, |ui, index| {
        let artist = app.library.artists.items[index].clone();
        let id = artist.id.clone();
        let uri = artist.uri.clone();
        let context_here = app.playing_context_uri().as_deref() == Some(uri.as_str());
        let playing_here = context_here && app.believed_playing();
        let card_art = app.art_px(widgets::CARD_ART);
        let card = widgets::card(
            ui,
            app,
            pick_image(&artist.images, card_art),
            &artist.name,
            &artist_label,
            widgets::CardCover::portrait(playing_here),
        );
        if card.play {
            if context_here {
                app.actions.push(Action::TogglePlay);
            } else {
                app.actions.push(Action::PlayContext {
                    uri,
                    offset_uri: None,
                    offset_index: None,
                });
            }
        }
        if card.clicked {
            app.actions.push(Action::Open(Page::Artist(id)));
        }
        egui::Popup::context_menu(&card.response)
            .id(ui.make_persistent_id(("library-artist-menu", &artist.uri)))
            .frame(widgets::menu_frame(&palette))
            .show(|ui| widgets::context_menu_items(ui, app, &artist.uri, &artist.name, None));
    });
    let list = &app.library.artists;
    let (loading, error, can_load, empty) = (
        list.loading,
        list.error.clone(),
        list.can_load_more(),
        list.items.is_empty() && list.loaded_once,
    );
    footer(
        app,
        ui,
        page,
        loading,
        error,
        can_load,
        empty,
        empty_title,
        empty_body,
        Icon::Users,
    );
}

#[allow(clippy::too_many_arguments)]
fn footer(
    app: &mut App,
    ui: &mut egui::Ui,
    page: Page,
    loading: bool,
    error: Option<String>,
    can_load: bool,
    empty: bool,
    empty_title: &str,
    empty_body: &str,
    icon: Icon,
) {
    let palette = app.palette;
    if !app.network_reachable && empty {
        widgets::empty_state(
            ui,
            &palette,
            Icon::WifiOff,
            &gettext(app.locale, "You're offline"),
            &gettext(
                app.locale,
                "Resonix will reconnect automatically when your connection returns.",
            ),
        );
        return;
    }
    if loading {
        ui.add_space(8.0);
        widgets::loading_row(ui, &palette, app.locale);
    }
    if let Some(error) = error {
        widgets::error_row(ui, app, &error, Some(page.clone()));
    }
    if empty && !loading {
        widgets::empty_state(ui, &palette, icon, empty_title, empty_body);
    }
    widgets::load_more_when_near_end(ui, app, page, can_load && !loading);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_decode_maps_library_and_legacy_playlists() {
        assert_eq!(Page::Library.encode(), "library");
        assert_eq!(Page::decode("library"), Some(Page::Library));
        assert_eq!(Page::decode("playlists"), Some(Page::Library));
    }

    #[cfg(feature = "demo")]
    fn test_app() -> App {
        let root = std::env::temp_dir().join(format!(
            "resonix-library-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let mut app = App::new(
            &crate::backend::Waker::default(),
            crate::paths::AppDirs {
                config: root.join("config"),
                state: root.join("state"),
                cache: root.join("cache"),
            },
            crate::settings::Settings::default(),
            crate::app::AppOptions {
                media_controls: false,
                restore_sign_in: false,
                tray: false,
                autostart: false,
            },
        );
        app.backend.shutdown();
        app
    }

    #[cfg(feature = "demo")]
    #[test]
    fn sidebar_library_header_opens_library_page() {
        use egui::accesskit::Role;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = test_app();
        app.attach(&ctx);
        crate::demo::populate(&mut app);

        let frame = |app: &mut App, events| {
            app.actions.clear();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| app.frame_ui(ui),
            );
            output.textures_delta.clear();
            (
                output.platform_output.accesskit_update.unwrap(),
                std::mem::take(&mut app.actions),
            )
        };

        let (tree, _) = frame(&mut app, vec![]);
        let bounds = tree
            .nodes
            .iter()
            .find_map(|(_, node)| {
                (node.role() == Role::Button && node.label() == Some("Library"))
                    .then(|| node.bounds())
                    .flatten()
            })
            .expect("Library button in sidebar");
        let pos = egui::pos2(
            ((bounds.x0 + bounds.x1) * 0.5) as f32,
            ((bounds.y0 + bounds.y1) * 0.5) as f32,
        );
        frame(
            &mut app,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        assert_eq!(app.page(), &Page::Library);
    }

    #[cfg(feature = "demo")]
    #[test]
    fn library_page_shows_liked_songs_and_playlists() {
        use egui::accesskit::Role;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = test_app();
        app.attach(&ctx);
        app.library.liked.total = Some(4);
        app.library.playlists = Loadable::Loaded(vec![crate::api::models::Playlist {
            id: "1".into(),
            name: "Road Trip".into(),
            uri: "spotify:playlist:1".into(),
            tracks: Some(crate::api::models::TrackCount { total: 12 }),
            ..Default::default()
        }]);

        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(960.0, 640.0),
                )),
                ..Default::default()
            },
            |ui| show(&mut app, ui, Page::Library),
        );
        output.textures_delta.clear();
        let tree = output.platform_output.accesskit_update.unwrap();
        assert!(
            tree.nodes.iter().any(|(_, n)| n.role() == Role::Button
                && n.label().is_some_and(|l| l.contains("Liked Songs"))),
            "Library page should render Liked Songs card"
        );
        assert!(
            tree.nodes.iter().any(|(_, n)| n.role() == Role::Button
                && n.label().is_some_and(|l| l.contains("Road Trip"))),
            "Library page should render user playlists"
        );
    }
}
