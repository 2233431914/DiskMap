//! Sidebar details panel: search, selected-node metadata, and core file actions.

use super::super::search_nav::SearchDirection;
use super::super::DiskMapApp;
use crate::app::{accent_button, describe_node_kind, palette, section_divider};
use crate::format::format_bytes;
use crate::platform::{
    open_path, reveal_action_label, reveal_action_short_label, reveal_in_file_manager,
};
use crate::scanner::{size_basis_detail, size_basis_label};
use eframe::egui::{self, Color32, CornerRadius, Margin, RichText, Stroke, Vec2};

pub fn show(ui: &mut egui::Ui, app: &mut DiskMapApp) {
    let p = palette(ui.ctx());
    ui.add_space(4.0);
    ui.label(
        RichText::new("CONTROLS")
            .size(11.0)
            .strong()
            .color(p.text_muted),
    );
    ui.add_space(2.0);
    section_divider(ui, p);
    ui.add_space(8.0);
    show_controls_section(ui, app, p);
    ui.add_space(12.0);
    ui.label(
        RichText::new("DETAILS")
            .size(11.0)
            .strong()
            .color(p.text_muted),
    );
    ui.add_space(2.0);
    section_divider(ui, p);
    ui.add_space(8.0);

    let subject_id = app
        .navigation
        .selected_id()
        .or(app.navigation.focused_root());
    let Some(node_id) = subject_id else {
        app.show_state_message(ui, p, &app.no_root_state_message());
        app.show_progress_section(ui, p);
        app.show_scan_issue_section(ui, p);
        return;
    };

    let node_path = app.tree.node_real_path(node_id);
    let (node_name, node_size, node_kind, child_count, node_scanned, node_error, node_parent) = {
        let node = app.tree.node(node_id);
        (
            node.name.clone(),
            node.size,
            node.kind,
            node.children.len(),
            node.scanned,
            node.error.clone(),
            node.parent,
        )
    };
    let matched = app.search.state().is_match(node_id);
    let kind_label = describe_node_kind(node_kind, child_count > 0);

    egui::Frame::new()
        .fill(p.panel_elevated)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(12))
        .stroke(Stroke::new(1.0, p.stroke_subtle))
        .show(ui, |ui| {
            ui.label(RichText::new(&node_name).strong().size(14.0).color(p.text));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format_bytes(node_size))
                        .monospace()
                        .color(p.accent),
                );
            });
            ui.label(
                RichText::new(size_basis_label())
                    .small()
                    .color(p.text_faint),
            )
            .on_hover_text(size_basis_detail());
            ui.add_space(4.0);
            let meta = if child_count > 0 {
                format!("{kind_label} · {} items", child_count)
            } else {
                kind_label.to_string()
            };
            ui.label(RichText::new(meta).small().color(p.text_muted));
            if !node_scanned {
                ui.label(
                    RichText::new("Scanning in progress…")
                        .small()
                        .color(p.accent),
                );
            }
            if !app.search.query().is_empty() {
                let (txt, color) = if matched {
                    ("Matches search", p.accent)
                } else {
                    ("No search match", p.text_faint)
                };
                ui.label(RichText::new(txt).small().color(color));
            }
            if let Some(path) = &node_path {
                ui.add_space(4.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(path.display().to_string())
                            .monospace()
                            .small()
                            .color(p.text_faint),
                    )
                    .wrap(),
                );
            }
        });

    if let Some(err) = &node_error {
        ui.add_space(6.0);
        egui::Frame::new()
            .fill(Color32::from_rgba_unmultiplied(
                p.danger.r(),
                p.danger.g(),
                p.danger.b(),
                28,
            ))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(Margin::same(10))
            .show(ui, |ui| {
                ui.label(RichText::new(format!("Error: {err}")).color(p.danger));
            });
    }

    ui.add_space(12.0);
    ui.label(
        RichText::new("PRIMARY")
            .size(10.0)
            .strong()
            .color(p.text_faint),
    );
    ui.add_space(4.0);
    let path_available = node_path.is_some();
    ui.columns(2, |cols| {
        let w0 = cols[0].available_width();
        if accent_button(&mut cols[0], "Open", path_available, w0, p).clicked() {
            if let Some(path) = &node_path {
                app.apply_platform_result("Open", open_path(path));
            }
        }
        let w1 = cols[1].available_width();
        let reveal_response = cols[1].add_enabled(
            path_available,
            egui::Button::new(reveal_action_short_label()).min_size(Vec2::new(w1, 32.0)),
        );
        let reveal_response = reveal_response.on_hover_text(reveal_action_label());
        if reveal_response.clicked() {
            if let Some(path) = &node_path {
                app.apply_platform_result(reveal_action_label(), reveal_in_file_manager(path));
            }
        }
    });

    ui.add_space(10.0);
    ui.label(
        RichText::new("UTILITY")
            .size(10.0)
            .strong()
            .color(p.text_faint),
    );
    ui.add_space(4.0);
    let copy_width = ui.available_width();
    if ui
        .add_enabled(
            path_available,
            egui::Button::new("Copy Path").min_size(Vec2::new(copy_width, 28.0)),
        )
        .clicked()
    {
        if let Some(path) = &node_path {
            ui.ctx().copy_text(path.display().to_string());
        }
    }

    if let Some(parent) = node_parent {
        ui.add_space(10.0);
        ui.label(
            RichText::new("PARENT")
                .size(10.0)
                .strong()
                .color(p.text_faint),
        );
        ui.add_space(4.0);
        let parent_name = app.tree.node(parent).name.clone();
        if ui
            .add(
                egui::Button::new(RichText::new(format!("↑ {parent_name}")).color(p.text))
                    .fill(Color32::TRANSPARENT)
                    .stroke(Stroke::new(1.0, p.stroke_subtle)),
            )
            .clicked()
        {
            app.navigation.set_selected_id(Some(parent));
        }
    }

    app.show_progress_section(ui, p);
    app.show_scan_issue_section(ui, p);
}

fn show_controls_section(ui: &mut egui::Ui, app: &mut DiskMapApp, p: &crate::app::Palette) {
    ui.label(
        RichText::new("SEARCH")
            .size(10.0)
            .strong()
            .color(p.text_faint),
    );
    ui.add_space(4.0);
    let search_response = ui.add_sized(
        [ui.available_width(), 28.0],
        egui::TextEdit::singleline(app.search.input_mut()).hint_text("Search files & folders"),
    );
    if search_response.changed() {
        app.mark_search_dirty();
    }
    if search_response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
        if ui.input(|input| input.modifiers.shift) {
            app.navigate_search_match(SearchDirection::Previous);
        } else {
            app.navigate_search_match(SearchDirection::Next);
        }
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let can_nav = app.can_navigate_search_matches();
        if ui
            .add_enabled(
                can_nav,
                egui::Button::new("<").min_size(Vec2::new(30.0, 26.0)),
            )
            .on_hover_text("Previous search match")
            .clicked()
        {
            app.navigate_search_match(SearchDirection::Previous);
        }
        if ui
            .add_enabled(
                can_nav,
                egui::Button::new(">").min_size(Vec2::new(30.0, 26.0)),
            )
            .on_hover_text("Next search match")
            .clicked()
        {
            app.navigate_search_match(SearchDirection::Next);
        }
        if ui
            .add_enabled(
                !app.search.input().is_empty(),
                egui::Button::new("Clear").min_size(Vec2::new(52.0, 26.0)),
            )
            .on_hover_text("Clear search")
            .clicked()
        {
            app.clear_search();
        }
        if ui
            .checkbox(&mut app.search_filter_enabled, "Filter")
            .on_hover_text("Show only search matches and their ancestor folders")
            .changed()
        {
            app.mark_layout_dirty_now();
        }
    });
    let match_text = if app.search.query().is_empty() {
        "No search query".to_string()
    } else if app.search.is_dirty() {
        format!("{} matches · Updating...", app.search.state().match_count())
    } else if let Some(index) = app.search.active_match() {
        format!(
            "{} / {} matches",
            index + 1,
            app.search.state().match_count()
        )
    } else {
        format!("{} matches", app.search.state().match_count())
    };
    ui.label(
        RichText::new(match_text)
            .small()
            .color(if app.search.is_dirty() {
                p.accent
            } else {
                p.text_muted
            }),
    );

    ui.add_space(12.0);
    ui.label(
        RichText::new("VIEW")
            .size(10.0)
            .strong()
            .color(p.text_faint),
    );
    ui.add_space(4.0);
    if ui
        .checkbox(&mut app.color_by_extension, "Color by extension")
        .on_hover_text("Color files by extension")
        .changed()
    {
        app.mark_layout_dirty_now();
    }
}
