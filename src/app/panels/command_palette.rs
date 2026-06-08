//! Command palette overlay. Press Cmd+K (mac) / Ctrl+K (other) to
//! open. Type to filter the built-in command list. Enter runs the
//! first match; Escape closes.

use super::super::DiskMapApp;
use crate::app::palette;
use crate::commands::{builtin_commands, filter_commands, Command};
use eframe::egui::{self, Align2, Key, RichText, Vec2};

const PALETTE_MAX_FILTERED: usize = 12;

pub fn show_command_palette(ctx: &egui::Context, app: &mut DiskMapApp) {
    if !app.palette_open {
        return;
    }
    // Pre-compute the registry once per frame.
    let commands = builtin_commands();
    let matches = filter_commands(&app.palette_query, &commands);

    let mut open = app.palette_open;
    let mut selected_index = app.palette_selected;

    // Centered modal-ish window. Anchor the bottom so it sits above
    // the toolbar/status bar.
    let screen = ctx.content_rect();
    let palette_height: f32 = 320.0;
    let palette_width: f32 = (screen.width() * 0.6).clamp(360.0, 720.0);
    let palette_rect = egui::Rect::from_center_size(
        egui::pos2(
            screen.center().x,
            screen.top() + palette_height * 0.5 + 60.0,
        ),
        Vec2::new(palette_width, palette_height),
    );

    let p = palette(ctx);
    egui::Window::new("__command_palette__")
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .fixed_rect(palette_rect)
        .anchor(Align2::CENTER_TOP, [0.0, 0.0])
        .frame(
            egui::Frame::new()
                .fill(p.panel_elevated)
                .stroke(egui::Stroke::new(1.0, p.stroke_strong))
                .corner_radius(egui::CornerRadius::same(10))
                .inner_margin(egui::Margin::same(8)),
        )
        .show(ctx, |ui| {
            ui.vertical(|ui| {
                // Text field at the top.
                let response = ui.add(
                    egui::TextEdit::singleline(&mut app.palette_query)
                        .hint_text("Type a command…")
                        .desired_width(palette_width - 32.0)
                        .font(egui::TextStyle::Heading),
                );
                response.request_focus();
                ui.add_space(6.0);
                ui.separator();

                // Result list (scrollable). If empty, show a hint.
                if matches.is_empty() {
                    ui.add_space(12.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("No matches").color(p.text_muted).italics());
                    });
                } else {
                    egui::ScrollArea::vertical()
                        .max_height(palette_height - 80.0)
                        .show(ui, |ui| {
                            for (i, cmd) in matches.iter().take(PALETTE_MAX_FILTERED).enumerate() {
                                let is_selected = i == selected_index;
                                let label = format!("{} — {}", cmd.label, cmd.hint);
                                let row_response = ui.add(egui::Button::selectable(
                                    is_selected,
                                    egui::RichText::new(label),
                                ));
                                if row_response.clicked() {
                                    selected_index = i;
                                    run_command(app, cmd);
                                    open = false;
                                    app.palette_query.clear();
                                    break;
                                }
                                if row_response.hovered() {
                                    selected_index = i;
                                }
                            }
                        });
                }
            });
        });

    app.palette_open = open;
    app.palette_selected = selected_index.min(matches.len().saturating_sub(1));

    // Keyboard handling: Enter runs the highlighted match, Escape
    // closes. (Cmd+K is handled in app.rs::handle_keyboard.)
    if ctx.input(|i| i.key_pressed(Key::Enter)) && !matches.is_empty() {
        if let Some(cmd) = matches.get(app.palette_selected.min(matches.len().saturating_sub(1))) {
            run_command(app, cmd);
            app.palette_open = false;
            app.palette_query.clear();
        }
    }
    if ctx.input(|i| i.key_pressed(Key::Escape)) {
        app.palette_open = false;
        app.palette_query.clear();
    }
    // Up/Down move the selection
    if ctx.input(|i| i.key_pressed(Key::ArrowDown)) && !matches.is_empty() {
        app.palette_selected = (app.palette_selected + 1).min(matches.len() - 1);
    }
    if ctx.input(|i| i.key_pressed(Key::ArrowUp)) {
        app.palette_selected = app.palette_selected.saturating_sub(1);
    }
}

fn run_command(app: &mut DiskMapApp, cmd: &Command) {
    (cmd.run)(app);
}
