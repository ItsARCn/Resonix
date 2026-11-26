//! Downloads page: grouped playlists/collections and individual offline tracks.

use std::sync::Arc;

use egui::{Align, CornerRadius, Frame, Margin, Rect, Sense, Vec2, pos2, vec2};

use crate::api::models::PlayableItem;
use crate::app::App;
use crate::i18n::{Locale, gettext, ngettext};
use crate::model::{
    Action, DownloadedCollection, DownloadedTrack, DownloadsLibrary, Loadable, Page, RowContext,
};
use crate::theme::{self, Icon};

use super::widgets::{self, TrackRow};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let locale = app.locale;
    ui.add_space(6.0);
    let loaded = app.downloads.get().cloned();
    ui.horizontal(|ui| {
        theme::text(
            ui,
            gettext(locale, "Downloads").as_ref(),
            theme::bold(28.0),
            palette.text,
        );
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            if theme::pill_button(ui, &palette, gettext(locale, "Refresh").as_ref(), false)
                .clicked()
            {
                app.actions.push(Action::RefreshDownloads);
            }
            if let Some(lib) = &loaded {
                let all_uris = all_playable_uris(lib);
                if !all_uris.is_empty()
                    && theme::pill_button(ui, &palette, gettext(locale, "Play all").as_ref(), true)
                        .clicked()
                {
                    app.actions.push(Action::PlayUris {
                        uris: all_uris,
                        index: 0,
                    });
                }
            }
        });
    });
    ui.add_space(12.0);

    match &app.downloads {
        Loadable::NotLoaded | Loadable::Loading => {
            widgets::loading_row(ui, &palette, locale);
        }
        Loadable::Failed(error) => {
            let error = error.clone();
            widgets::error_row(ui, app, &error, Some(Page::Downloads));
        }
        Loadable::Loaded(lib) => {
            let lib = lib.clone();
            if lib.collections.is_empty() && lib.tracks.is_empty() {
                widgets::empty_state(
                    ui,
                    &palette,
                    Icon::Download,
                    &gettext(locale, "No downloads yet"),
                    &gettext(
                        locale,
                        "Use Download on any playlist, album, or song to save it for offline playback.",
                    ),
                );
                return;
            }

            if !lib.collections.is_empty() {
                theme::text(
                    ui,
                    gettext(locale, "Playlists").as_ref(),
                    theme::bold(17.0),
                    palette.text,
                );
                ui.add_space(8.0);
                for col in &lib.collections {
                    collection_group(app, ui, col);
                    ui.add_space(6.0);
                }
            }

            if !lib.tracks.is_empty() {
                if !lib.collections.is_empty() {
                    ui.add_space(10.0);
                    theme::text(
                        ui,
                        gettext(locale, "Songs").as_ref(),
                        theme::bold(17.0),
                        palette.text,
                    );
                    ui.add_space(6.0);
                }
                let uris: Arc<[String]> = lib
                    .tracks
                    .iter()
                    .map(|d| d.track.uri.clone())
                    .collect::<Vec<_>>()
                    .into();
                let context = RowContext::Uris(uris);
                for (idx, dl) in lib.tracks.iter().enumerate() {
                    standalone_track_row(app, ui, idx, dl, &context);
                }
            }
        }
    }
}

fn collection_group(app: &mut App, ui: &mut egui::Ui, col: &DownloadedCollection) {
    let palette = app.palette;
    let locale = app.locale;
    let expand_id = ui.make_persistent_id(("download-collection-expanded", &col.uri));
    let mut expanded = ui.data(|d| d.get_temp::<bool>(expand_id)).unwrap_or(false);

    let play_uris: Vec<String> = col.tracks.iter().map(|d| d.track.uri.clone()).collect();
    let count_text = if col.tracks.len() < col.total_tracks {
        format!(
            "{} / {} {}",
            col.tracks.len(),
            col.total_tracks,
            ngettext(locale, "song", "songs", col.total_tracks as u32)
        )
    } else {
        ngettext(
            locale,
            "{count} song",
            "{count} songs",
            col.total_tracks as u32,
        )
        .replace("{count}", &col.total_tracks.to_string())
    };
    let size_text = format_bytes(col.total_bytes);
    let kind = collection_kind(locale, &col.uri);
    let subtitle = if col.total_bytes > 0 {
        format!("{kind} \u{2022} {count_text} \u{2022} {size_text}")
    } else {
        format!("{kind} \u{2022} {count_text}")
    };

    let row_width = ui.available_width();
    let row_height = 64.0;
    let (rect, response) = ui.allocate_exact_size(vec2(row_width, row_height), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            ui.is_enabled(),
            format!("{}, {}", col.label, subtitle),
        )
    });

    let hovered = ui.rect_contains_pointer(rect);
    let fill = if hovered {
        palette.surface_hover
    } else {
        palette.surface
    };
    ui.painter().rect_filled(rect, CornerRadius::same(8), fill);

    let cover_rect =
        Rect::from_min_size(pos2(rect.left() + 8.0, rect.top() + 8.0), Vec2::splat(48.0));
    if col.uri.ends_with(":collection") {
        super::sidebar::liked_cover(ui, cover_rect, 6.0);
    } else {
        let placeholder = if col.uri.starts_with("spotify:album:") {
            Icon::Disc
        } else if col.uri.starts_with("spotify:artist:") {
            Icon::User
        } else {
            Icon::Music
        };
        widgets::paint_cover(
            ui,
            &palette,
            col.thumbnail.as_deref(),
            cover_rect,
            6.0,
            placeholder,
            Some(app.backend.art()),
        );
    }

    // Reserve space on the right for Play, Expand/Collapse, and Delete buttons.
    let btn_size = 28.0;
    let btn_gap = 6.0;
    let right_pad = 10.0;
    let delete_rect = Rect::from_center_size(
        pos2(rect.right() - right_pad - btn_size / 2.0, rect.center().y),
        Vec2::splat(btn_size),
    );
    let expand_rect = Rect::from_center_size(
        pos2(
            delete_rect.left() - btn_gap - btn_size / 2.0,
            rect.center().y,
        ),
        Vec2::splat(btn_size),
    );
    let play_rect = Rect::from_center_size(
        pos2(
            expand_rect.left() - btn_gap - btn_size / 2.0,
            rect.center().y,
        ),
        Vec2::splat(btn_size),
    );

    let text_left = cover_rect.right() + 12.0;
    let text_right = (play_rect.left() - 10.0).max(text_left + 20.0);
    let text_width = text_right - text_left;
    let title_galley = widgets::ellipsized(
        ui,
        &col.label,
        theme::semibold(14.5),
        palette.text,
        text_width,
        1,
    );
    let sub_galley = widgets::ellipsized(
        ui,
        &subtitle,
        theme::regular(12.5),
        palette.secondary,
        text_width,
        1,
    );
    let total_text_h = title_galley.size().y + 2.0 + sub_galley.size().y;
    let text_top = rect.center().y - total_text_h / 2.0;
    ui.painter()
        .galley(pos2(text_left, text_top), title_galley, palette.text);
    ui.painter().galley(
        pos2(text_left, text_top + 19.0),
        sub_galley,
        palette.secondary,
    );

    let mut over_action_btn = false;

    let play_resp = ui.interact(
        play_rect,
        ui.id().with(("dl-col-play", &col.uri)),
        if play_uris.is_empty() {
            Sense::hover()
        } else {
            Sense::click()
        },
    );
    play_resp.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            !play_uris.is_empty(),
            gettext(locale, "Play").as_ref(),
        )
    });
    over_action_btn |= play_resp.hovered();
    let play_color = if play_uris.is_empty() {
        palette.dim
    } else if play_resp.hovered() {
        palette.accent
    } else {
        palette.text
    };
    theme::paint_icon(ui, Icon::PlayFilled, play_rect, 16.0, play_color);
    if play_resp.clicked() && !play_uris.is_empty() {
        app.actions.push(Action::PlayUris {
            uris: play_uris.clone(),
            index: 0,
        });
    }

    let expand_icon = if expanded {
        Icon::ChevronUp
    } else {
        Icon::ChevronDown
    };
    let expand_resp = ui.interact(
        expand_rect,
        ui.id().with(("dl-col-expand", &col.uri)),
        Sense::click(),
    );
    expand_resp.widget_info(|| {
        let label = if expanded {
            gettext(locale, "Collapse")
        } else {
            gettext(locale, "Expand")
        };
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label.as_ref())
    });
    over_action_btn |= expand_resp.hovered();
    theme::paint_icon(
        ui,
        expand_icon,
        expand_rect,
        16.0,
        if expand_resp.hovered() {
            palette.text
        } else {
            palette.secondary
        },
    );

    let delete_resp = ui.interact(
        delete_rect,
        ui.id().with(("dl-col-delete", &col.uri)),
        Sense::click(),
    );
    delete_resp.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            gettext(locale, "Delete download").as_ref(),
        )
    });
    over_action_btn |= delete_resp.hovered();
    theme::paint_icon(
        ui,
        Icon::Trash,
        delete_rect,
        16.0,
        if delete_resp.hovered() {
            palette.text
        } else {
            palette.secondary
        },
    );
    if delete_resp.clicked() {
        app.actions
            .push(Action::DeleteDownloadCollection(col.uri.clone()));
    }

    if expand_resp.clicked() || (response.clicked() && !over_action_btn) {
        expanded = !expanded;
    }
    ui.data_mut(|d| d.insert_temp(expand_id, expanded));

    if expanded && !col.tracks.is_empty() {
        let uris: Arc<[String]> = play_uris.into();
        let context = RowContext::Uris(uris);
        ui.add_space(4.0);
        Frame::new()
            .inner_margin(Margin {
                left: 16,
                right: 0,
                top: 0,
                bottom: 4,
            })
            .show(ui, |ui| {
                for (idx, dl) in col.tracks.iter().enumerate() {
                    let item = PlayableItem::Track(dl.track.clone());
                    widgets::track_row(
                        ui,
                        app,
                        TrackRow {
                            index: idx,
                            number: Some(idx + 1),
                            item: &item,
                            context: &context,
                            show_cover: true,
                            show_album: true,
                            added_at: None,
                            added_by: None,
                            show_added_by: false,
                            show_plays: false,
                            compact: false,
                            thin: false,
                            shift: 0.0,
                            picked: false,
                            picked_songs: &[],
                            picked_positions: &[],
                            position: None,
                        },
                    );
                }
            });
    }
}

fn standalone_track_row(
    app: &mut App,
    ui: &mut egui::Ui,
    idx: usize,
    dl: &DownloadedTrack,
    context: &RowContext,
) {
    let palette = app.palette;
    let item = PlayableItem::Track(dl.track.clone());
    ui.horizontal(|ui| {
        let delete_width = 32.0;
        let row_width = (ui.available_width() - delete_width - 8.0).max(200.0);
        ui.allocate_ui_with_layout(
            vec2(row_width, 54.0),
            egui::Layout::top_down(Align::LEFT),
            |ui| {
                widgets::track_row(
                    ui,
                    app,
                    TrackRow {
                        index: idx,
                        number: Some(idx + 1),
                        item: &item,
                        context,
                        show_cover: true,
                        show_album: true,
                        added_at: None,
                        added_by: None,
                        show_added_by: false,
                        show_plays: false,
                        compact: false,
                        thin: false,
                        shift: 0.0,
                        picked: false,
                        picked_songs: &[],
                        picked_positions: &[],
                        position: None,
                    },
                );
            },
        );
        if let Some(vid) = dl
            .track
            .id
            .as_deref()
            .or_else(|| dl.track.uri.strip_prefix("spotify:track:"))
            && theme::icon_button(
                ui,
                Icon::Trash,
                16.0,
                palette.secondary,
                palette.text,
                gettext(app.locale, "Delete download").as_ref(),
            )
            .clicked()
        {
            app.actions.push(Action::DeleteDownload(vid.to_string()));
        }
    });
}

fn all_playable_uris(lib: &DownloadsLibrary) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for col in &lib.collections {
        for dl in &col.tracks {
            if seen.insert(dl.track.uri.clone()) {
                out.push(dl.track.uri.clone());
            }
        }
    }
    for dl in &lib.tracks {
        if seen.insert(dl.track.uri.clone()) {
            out.push(dl.track.uri.clone());
        }
    }
    out
}

fn collection_kind(locale: Locale, uri: &str) -> String {
    if uri.ends_with(":collection") || uri.starts_with("spotify:playlist:") {
        gettext(locale, "Playlist").into_owned()
    } else if uri.starts_with("spotify:album:") {
        gettext(locale, "Album").into_owned()
    } else if uri.starts_with("spotify:artist:") {
        gettext(locale, "Artist").into_owned()
    } else {
        gettext(locale, "Collection").into_owned()
    }
}

fn format_bytes(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    const KB: f64 = 1024.0;
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / MB)
    } else if bytes >= 1024 {
        format!("{:.0} KB", bytes as f64 / KB)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(all(test, feature = "demo"))]
mod tests {
    use super::*;

    fn test_app() -> App {
        let root = std::env::temp_dir().join(format!(
            "resonix-downloads-test-{}-{}",
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

    fn track(id: &str, name: &str) -> DownloadedTrack {
        DownloadedTrack {
            track: crate::api::models::Track {
                id: Some(id.into()),
                name: name.into(),
                uri: format!("spotify:track:{id}"),
                ..Default::default()
            },
            path: format!("/tmp/{id}.mp3"),
            size_bytes: 2 * 1024 * 1024,
        }
    }

    #[test]
    fn downloads_page_groups_collections_separately_from_individual_songs() {
        use egui::accesskit::Role;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = test_app();
        app.attach(&ctx);
        app.downloads = Loadable::Loaded(DownloadsLibrary {
            collections: vec![DownloadedCollection {
                uri: "spotify:playlist:1".into(),
                label: "Evening Mix".into(),
                thumbnail: None,
                total_tracks: 2,
                tracks: vec![
                    track("p1", "Playlist Song One"),
                    track("p2", "Playlist Song Two"),
                ],
                total_bytes: 4 * 1024 * 1024,
            }],
            tracks: vec![track("s1", "Single Song")],
            all_track_ids: vec!["p1".into(), "p2".into(), "s1".into()],
        });

        let draw = |app: &mut App, events| {
            app.actions.clear();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(800.0, 600.0))),
                    events,
                    ..Default::default()
                },
                |ui| show(app, ui),
            );
            output.textures_delta.clear();
            (
                output.platform_output.accesskit_update.unwrap(),
                std::mem::take(&mut app.actions),
            )
        };

        let (tree, _) = draw(&mut app, vec![]);
        // Collapsed initially: playlist header is visible, nested songs are hidden, standalone song is visible.
        assert!(
            tree.nodes.iter().any(|(_, n)| {
                n.role() == Role::Button && n.label().is_some_and(|l| l.contains("Evening Mix"))
            }),
            "playlist collection should appear as a group"
        );
        assert!(
            !tree
                .nodes
                .iter()
                .any(|(_, n)| { n.label() == Some("Playlist Song One") }),
            "nested playlist songs should stay collapsed until expanded"
        );
        assert!(
            tree.nodes.iter().any(|(_, n)| {
                n.role() == Role::Button && n.label().is_some_and(|l| l.contains("Single Song"))
            }),
            "individual song should still be listed"
        );

        // Expand the playlist collection to reveal nested tracks.
        let expand_bounds = tree
            .nodes
            .iter()
            .find_map(|(_, n)| {
                (n.role() == Role::Button && n.label() == Some("Expand"))
                    .then(|| n.bounds())
                    .flatten()
            })
            .expect("Expand button");
        let pos = pos2(
            ((expand_bounds.x0 + expand_bounds.x1) * 0.5) as f32,
            ((expand_bounds.y0 + expand_bounds.y1) * 0.5) as f32,
        );
        let (tree, _) = draw(
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
        assert!(
            tree.nodes.iter().any(|(_, n)| {
                n.role() == Role::Button
                    && n.label().is_some_and(|l| l.contains("Playlist Song One"))
            }),
            "expanding the collection should reveal its tracks"
        );
    }
}
