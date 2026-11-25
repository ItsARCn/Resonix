//! Explore page: curated & personalized YouTube music discovery sections.

use crate::api::models::{PlayableItem, join_names, pick_image};
use crate::app::App;
use crate::i18n::gettext;
use crate::model::{Action, Loadable, Page};
use crate::theme::{self, Icon};

use super::widgets;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        theme::text(
            ui,
            gettext(app.locale, "Explore").as_ref(),
            theme::bold(28.0),
            palette.text,
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if theme::pill_button(ui, &palette, gettext(app.locale, "Refresh").as_ref(), false)
                .clicked()
            {
                app.actions.push(Action::RefreshExplore);
            }
        });
    });
    ui.add_space(12.0);

    match &app.explore_sections {
        Loadable::NotLoaded | Loadable::Loading => {
            widgets::loading_row(ui, &palette, app.locale);
            ui.add_space(8.0);
            widgets::skeleton_cards(ui, &palette, app.locale, 5, false);
        }
        Loadable::Failed(error) => {
            let error = error.clone();
            widgets::error_row(ui, app, &error, Some(Page::Explore));
        }
        Loadable::Loaded(sections) => {
            let sections = sections.clone();
            if sections.is_empty() {
                widgets::empty_state(
                    ui,
                    &palette,
                    Icon::Compass,
                    &gettext(app.locale, "Nothing to explore yet"),
                    &gettext(
                        app.locale,
                        "Try refreshing or searching for songs to seed your feed.",
                    ),
                );
                return;
            }
            for (sec_idx, (title, tracks)) in sections.iter().enumerate() {
                let uris: Vec<String> = tracks.iter().map(|t| t.uri.clone()).collect();
                let shelf_id = format!("explore-section-{sec_idx}");
                widgets::shelf(ui, &palette, &shelf_id, title, |ui| {
                    let card_art = app.art_px(widgets::CARD_ART);
                    for (idx, track) in tracks.iter().enumerate() {
                        let subtitle =
                            join_names(track.artists.iter().map(|artist| artist.name.as_str()));
                        let subtitle = if subtitle.is_empty() {
                            gettext(app.locale, "YouTube").into_owned()
                        } else {
                            subtitle
                        };
                        let playing_here = app
                            .now_playing()
                            .is_some_and(|now| now.playing && now.uri == track.uri);
                        let card = widgets::card(
                            ui,
                            app,
                            track.image(card_art).or_else(|| {
                                track
                                    .album
                                    .as_ref()
                                    .and_then(|album| pick_image(&album.images, card_art))
                            }),
                            &track.name,
                            &subtitle,
                            widgets::CardCover::square(playing_here),
                        );
                        if card.play || card.clicked {
                            if app.now_playing().is_some_and(|now| now.uri == track.uri)
                                && card.play
                            {
                                app.actions.push(Action::TogglePlay);
                            } else {
                                app.actions.push(Action::PlayUris {
                                    uris: uris.clone(),
                                    index: idx as u32,
                                });
                            }
                        }
                        egui::Popup::context_menu(&card.response)
                            .id(ui.make_persistent_id(("explore-track-menu", sec_idx, &track.uri)))
                            .frame(widgets::menu_frame(&palette))
                            .show(|ui| {
                                widgets::item_menu(
                                    ui,
                                    app,
                                    &PlayableItem::Track(track.clone()),
                                    None,
                                    Some(idx),
                                );
                            });
                    }
                });
            }
        }
    }
}
