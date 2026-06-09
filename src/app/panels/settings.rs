//! Settings window: scan root path and scan option controls.

use super::super::DiskMapApp;
use crate::app::palette;
use eframe::egui::{self, Align2, RichText};

pub fn show_settings_window(ctx: &egui::Context, app: &mut DiskMapApp) {
    if !app.settings_open {
        return;
    }

    let screen = ctx.content_rect();
    let width = (screen.width() * 0.48).clamp(420.0, 680.0);
    let mut open = app.settings_open;
    let mut close_requested = false;
    let p = palette(ctx);

    egui::Window::new("Settings")
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
                RichText::new("SCAN ROOT")
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
                RichText::new("SCAN CONDITIONS")
                    .size(10.0)
                    .strong()
                    .color(p.text_faint),
            );
            ui.add_sized(
                [ui.available_width(), 28.0],
                egui::TextEdit::singleline(&mut app.exclude_input)
                    .hint_text(".git,node_modules,target"),
            )
            .on_hover_text(
                "Excluded names or path fragments; comma, semicolon, or newline separated",
            );

            ui.columns(2, |cols| {
                cols[0]
                    .checkbox(&mut app.include_hidden, "Hidden")
                    .on_hover_text("Include hidden files and folders");
                cols[1]
                    .checkbox(&mut app.follow_symlinks, "Links")
                    .on_hover_text("Follow symlinked directories during scan");
            });
            ui.columns(2, |cols| {
                cols[0]
                    .checkbox(&mut app.stay_on_filesystem, "Same FS")
                    .on_hover_text("Stay on the scan root filesystem when supported");
                let before_watch = app.realtime_watch_enabled;
                cols[1]
                    .checkbox(&mut app.realtime_watch_enabled, "Watch")
                    .on_hover_text(
                        "Watch the scan root and rescan after debounced filesystem changes",
                    );
                if app.realtime_watch_enabled != before_watch {
                    app.update_watch_state();
                }
            });

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let scan_label = if app.scan.is_scanning() {
                    "Cancel Scan"
                } else {
                    "Start Scan"
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
                    .add_sized([72.0, 30.0], egui::Button::new("Close"))
                    .clicked()
                {
                    close_requested = true;
                }
            });
        });

    if close_requested {
        open = false;
    }
    app.settings_open = open;
}
