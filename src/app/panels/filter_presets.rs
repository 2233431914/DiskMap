//! Filter presets section: list of saved search queries with
//! apply/delete buttons, plus a small name input for saving the
//! current filter. Sits in the sidebar after the rules section.

use super::super::DiskMapApp;
use super::super::Palette;
use eframe::egui::{self, RichText, Vec2};

const MAX_PRESET_PREVIEW: usize = 8;

pub fn show_filter_presets_section(ui: &mut egui::Ui, p: &Palette, app: &mut DiskMapApp) {
    ui.add_space(12.0);
    let count = app.filter_presets.len();
    ui.label(
        RichText::new(format!("FILTER PRESETS ({count})"))
            .size(10.0)
            .strong()
            .color(p.text_faint),
    );
    ui.add_space(4.0);

    // Save current filter as preset
    ui.horizontal(|ui| {
        ui.add_sized(
            [ui.available_width() - 64.0, 22.0],
            egui::TextEdit::singleline(&mut app.filter_preset_name)
                .hint_text("Name for new preset"),
        );
        if ui
            .add_sized([60.0, 22.0], egui::Button::new("Save"))
            .clicked()
        {
            let name = app.filter_preset_name.clone();
            app.add_filter_preset(&name);
        }
    });

    // List existing presets
    if app.filter_presets.is_empty() {
        ui.add_space(4.0);
        ui.label(
            RichText::new("No saved filters yet — type a name and Save")
                .small()
                .color(p.text_muted),
        );
        return;
    }

    ui.add_space(4.0);
    let presets = app.filter_presets.list();
    for preset in presets.iter().take(MAX_PRESET_PREVIEW) {
        let name = preset.name.clone();
        ui.horizontal(|ui| {
            let label = if preset.query.is_empty() {
                format!("(empty query){}", if preset.filter_enabled { " + filter" } else { "" })
            } else {
                format!(
                    "{}{}",
                    preset.query,
                    if preset.filter_enabled { " + filter" } else { "" }
                )
            };
            if ui
                .add(egui::Button::new(label).min_size(Vec2::new(0.0, 22.0)))
                .on_hover_text("Click to apply this filter preset")
                .clicked()
            {
                app.apply_filter_preset(&name);
            }
            if ui
                .add_sized([22.0, 22.0], egui::Button::new("✕"))
                .on_hover_text("Remove this preset")
                .clicked()
            {
                app.remove_filter_preset(&name);
            }
        });
    }
    if presets.len() > MAX_PRESET_PREVIEW {
        ui.label(
            RichText::new(format!("…and {} more", presets.len() - MAX_PRESET_PREVIEW))
                .small()
                .color(p.text_faint),
        );
    }
}
