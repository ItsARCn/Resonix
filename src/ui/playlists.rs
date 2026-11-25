//! Playlists page: grid overview of local and saved playlists.

use crate::api::models::pick_image;
use crate::app::App;
use crate::i18n::{gettext, ngettext};
use crate::model::{Action, Dialog, Loadable, Page};
use crate::theme::{self, Icon};

use super::widgets;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        theme::text(
            ui,
            gettext(app.locale, "Playlists").as_ref(),
            theme::bold(28.0),
            palette.text,
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if theme::pill_button(
                ui,
                &palette,
                gettext(app.locale, "New playlist").as_ref(),
                true,
            )
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
    ui.add_space(14.0);

    match &app.library.playlists {
        Loadable::NotLoaded | Loadable::Loading => {
            widgets::loading_row(ui, &palette, app.locale);
        }
        Loadable::Failed(error) => {
            let error = error.clone();
            widgets::error_row(ui, app, &error, Some(Page::Playlists));
        }
        Loadable::Loaded(playlists) => {
            let playlists = playlists.clone();
            if playlists.is_empty() {
                widgets::empty_state(
                    ui,
                    &palette,
                    Icon::Music,
                    &gettext(app.locale, "No playlists yet"),
                    &gettext(
                        app.locale,
                        "Create a playlist to organize your favorite tracks.",
                    ),
                );
                return;
            }
            let card_height = widgets::card_row_height(ui);
            let card_art = app.art_px(widgets::CARD_ART);
            widgets::virtual_wrapped_cards(ui, playlists.len(), card_height, |ui, index| {
                let playlist = &playlists[index];
                let total = playlist
                    .tracks
                    .as_ref()
                    .or(playlist.items_count.as_ref())
                    .map(|c| c.total);
                let subtitle = if let Some(count) = total {
                    ngettext(app.locale, "{count} song", "{count} songs", count)
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
                    .id(ui.make_persistent_id(("playlists-page-menu", &playlist.uri)))
                    .frame(widgets::menu_frame(&palette))
                    .show(|ui| {
                        widgets::context_menu_items(ui, app, &playlist.uri, &playlist.name, owned);
                    });
            });
        }
    }
}
