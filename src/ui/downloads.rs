//! Downloads page: manage and play offline downloaded tracks.

use std::sync::Arc;

use crate::api::models::PlayableItem;
use crate::app::App;
use crate::i18n::gettext;
use crate::model::{Action, DownloadedTrack, Loadable, Page, RowContext};
use crate::theme::{self, Icon};

use super::widgets::{self, TrackRow};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    ui.add_space(6.0);
    let loaded = app.downloads.get().cloned();
    ui.horizontal(|ui| {
        theme::text(
            ui,
            gettext(app.locale, "Downloads").as_ref(),
            theme::bold(28.0),
            palette.text,
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if theme::pill_button(ui, &palette, gettext(app.locale, "Refresh").as_ref(), false)
                .clicked()
            {
                app.actions.push(Action::RefreshDownloads);
            }
            if let Some(items) = &loaded
                && !items.is_empty()
                && theme::pill_button(ui, &palette, gettext(app.locale, "Play all").as_ref(), true)
                    .clicked()
            {
                let uris = items.iter().map(|d| d.track.uri.clone()).collect();
                app.actions.push(Action::PlayUris { uris, index: 0 });
            }
        });
    });
    ui.add_space(12.0);

    match &app.downloads {
        Loadable::NotLoaded | Loadable::Loading => {
            widgets::loading_row(ui, &palette, app.locale);
        }
        Loadable::Failed(error) => {
            let error = error.clone();
            widgets::error_row(ui, app, &error, Some(Page::Downloads));
        }
        Loadable::Loaded(items) => {
            let items: Vec<DownloadedTrack> = items.clone();
            if items.is_empty() {
                widgets::empty_state(
                    ui,
                    &palette,
                    Icon::Download,
                    &gettext(app.locale, "No downloads yet"),
                    &gettext(
                        app.locale,
                        "Right-click any song and choose Download to save it for offline playback.",
                    ),
                );
                return;
            }
            let uris: Arc<[String]> = items
                .iter()
                .map(|d| d.track.uri.clone())
                .collect::<Vec<_>>()
                .into();
            let context = RowContext::Uris(uris);
            for (idx, dl) in items.iter().enumerate() {
                let item = PlayableItem::Track(dl.track.clone());
                ui.horizontal(|ui| {
                    let delete_width = 32.0;
                    let row_width = (ui.available_width() - delete_width - 8.0).max(200.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(row_width, 54.0),
                        egui::Layout::top_down(egui::Align::LEFT),
                        |ui| {
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
        }
    }
}
