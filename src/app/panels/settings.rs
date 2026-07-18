//! Settings window: scan root path and scan option controls.

use super::super::DiskMapApp;
use crate::app::palette;
use crate::i18n::{Locale, TextKey};
use eframe::egui::{self, Align2, RichText};

pub fn show_settings_window(ctx: &egui::Context, app: &mut DiskMapApp) {
    if !app.settings_open {
        return;
    }

    let screen = ctx.content_rect();
    let width = (screen.width() * 0.48).clamp(420.0, 680.0);
    let mut open = app.settings_open;
    let mut close_requested = false;
    let mut selected_locale = app.locale;
    let mut follow_system = app.locale_follow_system;
    let p = palette(ctx);

    egui::Window::new(app.text(TextKey::Settings))
        .open(&mut open)
        .resizable(false)
        .collapsible(false)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .default_width(width)
        .frame(
            egui::Frame::new()
                .fill(p.panel_elevated)
                .stroke(egui::Stroke::new(1.0, p.stroke_strong))
                .corner_radius(egui::CornerRadius::same(10))
                .inner_margin(egui::Margin::same(12)),
        )
        .show(ctx, |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 8.0;

            ui.label(
                RichText::new(app.text(TextKey::ScanRootLabel).to_uppercase())
                    .size(10.0)
                    .strong()
                    .color(p.text_faint),
            );
            let path_response = ui.add_sized(
                [ui.available_width(), 28.0],
                egui::TextEdit::singleline(&mut app.path_input).hint_text("/path/to/scan"),
            );
            if path_response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                app.start_scan();
                close_requested = true;
            }

            ui.add_space(4.0);
            ui.label(
                RichText::new(app.text(TextKey::ScanConditions).to_uppercase())
                    .size(10.0)
                    .strong()
                    .color(p.text_faint),
            );
            ui.add_sized(
                [ui.available_width(), 28.0],
                egui::TextEdit::singleline(&mut app.exclude_input)
                    .hint_text(".git,node_modules,target"),
            )
            .on_hover_text(app.text(TextKey::ExcludeHint));

            let hidden_label = app.text(TextKey::IncludeHidden);
            ui.checkbox(&mut app.include_hidden, hidden_label)
                .on_hover_text(hidden_label);
            ui.columns(2, |cols| {
                let filesystem_label = app.text(TextKey::SameFilesystem);
                let filesystem_hint = app.text(TextKey::SameFilesystemHint);
                cols[0]
                    .checkbox(&mut app.stay_on_filesystem, filesystem_label)
                    .on_hover_text(filesystem_hint);
                let mut watch_enabled = app.realtime_watch_enabled();
                cols[1]
                    .checkbox(&mut watch_enabled, app.text(TextKey::Watch))
                    .on_hover_text(app.text(TextKey::WatchHint));
                if watch_enabled != app.realtime_watch_enabled() {
                    app.set_realtime_watch_enabled(watch_enabled);
                }
            });

            #[cfg(target_os = "macos")]
            {
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(6.0);
                ui.label(
                    RichText::new(app.text(TextKey::FileAccess).to_uppercase())
                        .size(10.0)
                        .strong()
                        .color(p.text_faint),
                );
                ui.add(
                    egui::Label::new(
                        RichText::new(app.text(TextKey::FullDiskAccessHint))
                            .small()
                            .color(p.text_muted),
                    )
                    .wrap(),
                );
                if ui.button(app.text(TextKey::OpenFullDiskAccess)).clicked() {
                    app.open_full_disk_access_settings();
                }
            }

            ui.add_space(6.0);
            ui.separator();
            ui.add_space(6.0);
            ui.label(
                RichText::new(app.text(TextKey::Appearance).to_uppercase())
                    .size(10.0)
                    .strong()
                    .color(p.text_faint),
            );
            ui.horizontal(|ui| {
                ui.label(app.text(TextKey::Language));
                ui.checkbox(&mut follow_system, app.text(TextKey::FollowSystem));
                ui.add_enabled_ui(!follow_system, |ui| {
                    egui::ComboBox::from_id_salt("language_preference")
                        .selected_text(selected_locale.display_name())
                        .show_ui(ui, |ui| {
                            for locale in [
                                Locale::English,
                                Locale::SimplifiedChinese,
                                Locale::TraditionalChinese,
                            ] {
                                ui.selectable_value(
                                    &mut selected_locale,
                                    locale,
                                    locale.display_name(),
                                );
                            }
                        });
                });
            });

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let scan_label = if app.scan.is_scanning() {
                    app.text(TextKey::CancelScan)
                } else {
                    app.text(TextKey::StartScan)
                };
                if ui
                    .add_sized([110.0, 30.0], egui::Button::new(scan_label))
                    .clicked()
                {
                    if app.scan.is_scanning() {
                        app.cancel_scan();
                    } else {
                        app.start_scan();
                        close_requested = true;
                    }
                }
                if ui
                    .add_sized([72.0, 30.0], egui::Button::new(app.text(TextKey::Close)))
                    .clicked()
                {
                    close_requested = true;
                }
            });
        });

    if close_requested {
        open = false;
    }
    if follow_system != app.locale_follow_system
        || (!follow_system && selected_locale != app.locale)
    {
        app.set_locale_preference(ctx, follow_system, selected_locale);
    }
    app.settings_open = open;
}
