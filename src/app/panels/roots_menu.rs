//! Roots menu: list of recent scan roots + pinned favorites, with
//! "pin current" / "unpin current" toggle. Lives in the toolbar.

use super::super::DiskMapApp;
use crate::app::{palette, truncate_middle};
use crate::i18n::TextKey;
use eframe::egui::{self, RichText};
use std::path::PathBuf;

pub fn show_roots_menu(ui: &mut egui::Ui, app: &mut DiskMapApp) {
    let pin_candidate = app.current_root_candidate();
    let is_pinned = pin_candidate
        .as_deref()
        .is_some_and(|path| app.is_root_pinned(path));

    ui.menu_button(app.text(TextKey::Roots), |ui| {
        ui.set_min_width(280.0);
        let can_pin = pin_candidate.is_some();
        let pin_label = if is_pinned {
            app.text(TextKey::UnpinCurrent)
        } else {
            app.text(TextKey::PinCurrent)
        };
        if ui
            .add_enabled(can_pin, egui::Button::new(pin_label))
            .clicked()
        {
            if let Some(path) = pin_candidate.as_deref() {
                app.toggle_pinned_root(path);
            }
            ui.close();
        }

        ui.separator();
        show_root_menu_group(ui, app, app.text(TextKey::Pinned), app.pinned_roots.clone());
        show_root_menu_group(ui, app, app.text(TextKey::Recent), app.recent_roots.clone());
    })
    .response
    .on_hover_text(app.text(TextKey::RecentAndPinnedRoots));
}

fn show_root_menu_group(ui: &mut egui::Ui, app: &mut DiskMapApp, label: &str, roots: Vec<String>) {
    ui.label(
        RichText::new(label)
            .size(10.0)
            .strong()
            .color(palette(ui.ctx()).text_faint),
    );
    if roots.is_empty() {
        ui.label(
            RichText::new(app.text(TextKey::None))
                .small()
                .color(palette(ui.ctx()).text_faint),
        );
        return;
    }

    for path in roots {
        if ui
            .button(truncate_middle(&path, 54))
            .on_hover_text(&path)
            .clicked()
        {
            app.start_scan_path(PathBuf::from(path));
            ui.close();
        }
    }
}
