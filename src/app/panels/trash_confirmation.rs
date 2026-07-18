use super::super::DiskMapApp;
use crate::app::palette;
use crate::format::format_bytes;
#[cfg(test)]
use crate::i18n::Locale;
use crate::i18n::TextKey;
use eframe::egui::{self, Color32, CornerRadius, Margin, RichText, Stroke};

pub fn show(ctx: &egui::Context, app: &mut DiskMapApp) {
    let Some(node_id) = app.trash_confirm_target_id else {
        return;
    };
    let Some(path) = app.trash_confirm_path.clone() else {
        app.clear_trash_confirmation();
        return;
    };
    if !app.tree.contains_id(node_id) {
        app.clear_trash_confirmation();
        return;
    }

    let node = app.tree.node(node_id);
    let name = node.name.clone();
    let size = node.size;
    let kind = app.locale.node_kind(node.kind, !node.children.is_empty());
    let item_count = app.cleanup_item_count(node_id);
    let path_text = path.display().to_string();
    let p = palette(ctx);
    let mut confirm = false;
    let mut cancel = false;

    let response = egui::Modal::new(egui::Id::new("trash_confirmation"))
        .backdrop_color(Color32::from_black_alpha(150))
        .frame(
            egui::Frame::new()
                .fill(p.panel_elevated)
                .stroke(Stroke::new(1.0, p.stroke_strong))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(Margin::same(18)),
        )
        .show(ctx, |ui| {
            ui.set_min_width(460.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(egui_phosphor::regular::WARNING)
                        .size(22.0)
                        .color(p.danger),
                );
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(app.text(TextKey::MoveToTrashTitle))
                            .strong()
                            .size(17.0),
                    );
                    ui.label(
                        RichText::new(app.text(TextKey::MoveToTrashDescription))
                            .small()
                            .color(p.text_muted),
                    );
                });
            });

            ui.add_space(14.0);
            ui.label(RichText::new(&name).strong().color(p.text));
            ui.add_space(6.0);
            ui.label(
                RichText::new(format!("{}: {}", app.text(TextKey::Path), path_text))
                    .monospace()
                    .small()
                    .color(p.text_muted),
            );
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(format!("{}: {}", app.text(TextKey::Type), kind))
                        .small()
                        .color(p.text_muted),
                );
                ui.separator();
                ui.label(
                    RichText::new(format!(
                        "{}: {}",
                        app.text(TextKey::Size),
                        format_bytes(size)
                    ))
                    .small()
                    .color(p.text_muted),
                );
                ui.separator();
                ui.label(
                    RichText::new(format!("{}: {}", app.text(TextKey::Items), item_count))
                        .small()
                        .color(p.text_muted),
                );
            });

            ui.add_space(12.0);
            ui.label(
                RichText::new(app.text(TextKey::Warning))
                    .small()
                    .color(p.danger),
            );
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let confirm_response = ui
                        .add_sized(
                            [150.0, 34.0],
                            egui::Button::new(format!(
                                "{}  {}",
                                egui_phosphor::regular::TRASH,
                                app.text(TextKey::Confirm)
                            ))
                            .fill(p.danger),
                        )
                        .on_hover_text(app.text(TextKey::Confirm));
                    if confirm_response.clicked() {
                        confirm = true;
                    }

                    let cancel_response = ui
                        .add_sized(
                            [100.0, 34.0],
                            egui::Button::new(format!(
                                "{}  {}",
                                egui_phosphor::regular::X,
                                app.text(TextKey::Cancel)
                            )),
                        )
                        .on_hover_text(app.text(TextKey::Cancel));
                    if cancel_response.clicked() {
                        cancel = true;
                    }
                });
            });
        });

    if cancel || response.should_close() {
        app.clear_trash_confirmation();
    } else if confirm {
        app.confirm_trash();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_locales_have_confirmation_title() {
        for locale in [
            Locale::English,
            Locale::SimplifiedChinese,
            Locale::TraditionalChinese,
        ] {
            assert!(!locale.text(TextKey::MoveToTrashTitle).is_empty());
        }
    }
}
