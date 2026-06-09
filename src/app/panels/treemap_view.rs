//! Treemap rendering: fixed-area layout caching, selection, context menu, and
//! hover tooltip. Extracted from `app.rs` so the per-frame render path reads
//! top-to-bottom in one file.
//!
//! Helper paint routines (`paint_visual`, `extension_color_for_visual`,
//! `show_hover_tooltip`) stay as methods on `DiskMapApp` in `app.rs` because
//! they are also reused by other code paths; this module calls them through
//! the `app` argument.

use super::super::DiskMapApp;
use crate::app::{
    find_hovered_visual, palette, truncate_middle, CONTEXT_MENU_MAX_TITLE_CHARS,
    CONTEXT_MENU_MIN_WIDTH, LAYOUT_REFRESH_INTERVAL,
};
use crate::format::format_bytes;
use crate::platform::{open_path, reveal_action_label, reveal_in_file_manager};
use crate::treemap::{layout_treemap, TreemapLayoutParams, VisualKind};
use eframe::egui::{self, RichText, Sense, Vec2};
use std::time::Instant;

pub fn show(ui: &mut egui::Ui, app: &mut DiskMapApp) {
    let p = palette(ui.ctx());
    let available = ui.available_rect_before_wrap().intersect(ui.clip_rect());
    if available.width() <= 0.0 || available.height() <= 0.0 {
        app.cached_visuals.clear();
        app.last_canvas_rect = None;
        app.layout_dirty = true;
        return;
    }
    let response = ui.allocate_rect(available, Sense::click());
    let painter = ui.painter_at(available);
    painter.rect_filled(available, 0.0, p.surface);

    let Some(root_id) = app.navigation.focused_root() else {
        app.paint_state_message(
            &painter,
            available,
            ui.style(),
            p,
            &app.no_root_state_message(),
        );
        return;
    };

    if let Some(message) = app.empty_root_state_message(root_id) {
        app.paint_state_message(&painter, available, ui.style(), p, &message);
        return;
    }

    let canvas_changed = app.last_canvas_rect != Some(available);
    if canvas_changed {
        app.last_canvas_rect = Some(available);
        app.layout_dirty = true;
        app.last_layout_refresh = Instant::now()
            .checked_sub(LAYOUT_REFRESH_INTERVAL)
            .unwrap_or_else(Instant::now);
    }

    let should_refresh_layout = app.layout_dirty
        && (!app.scan.is_scanning()
            || app.last_layout_refresh.elapsed() >= LAYOUT_REFRESH_INTERVAL);

    if should_refresh_layout {
        let layout_start = Instant::now();
        layout_treemap(
            &mut app.tree,
            TreemapLayoutParams {
                root: root_id,
                canvas_rect: available,
                max_depth: app.max_depth,
                search_state: app.search.state(),
                filter_to_search: app.search_filter_enabled,
                out: &mut app.cached_visuals,
                scratch: &mut app.layout_scratch,
            },
        );
        app.layout_dirty = false;
        app.last_layout_refresh = Instant::now();
        app.scan.record_layout_recompute(layout_start.elapsed());
    }

    app.hovered_visual_kind =
        find_hovered_visual(&app.cached_visuals, response.hover_pos()).map(|visual| visual.kind);
    app.hovered_id = app.hovered_visual_kind.map(|kind| match kind {
        VisualKind::Node(node_id) => node_id,
    });

    if response.secondary_clicked() {
        app.context_menu_target_id = app.hovered_id;
    }

    for visual in &app.cached_visuals {
        app.paint_visual(ui, &painter, visual);
    }
    if response.double_clicked() {
        if let Some(node_id) = app.hovered_id {
            if !app.tree.node(node_id).children.is_empty() {
                app.enter_root(node_id, true);
            } else {
                app.navigation.set_selected_id(Some(node_id));
            }
        } else {
            app.navigation.set_selected_id(None);
        }
    } else if response.clicked() {
        if let Some(node_id) = app.hovered_id {
            app.navigation.set_selected_id(Some(node_id));
        } else {
            app.navigation.set_selected_id(None);
        }
    }

    response.context_menu(|ui| {
        if let Some(node_id) = app.context_menu_target_id {
            let p = palette(ui.ctx());
            let node_path = app.tree.node_real_path(node_id);
            let node = app.tree.node(node_id);
            let node_name = node.name.clone();
            let node_size = node.size;
            ui.set_min_width(CONTEXT_MENU_MIN_WIDTH);
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(truncate_middle(&node_name, CONTEXT_MENU_MAX_TITLE_CHARS))
                        .strong()
                        .color(p.text),
                );
                ui.label(
                    RichText::new(format_bytes(node_size))
                        .small()
                        .monospace()
                        .color(p.text_muted),
                );
                ui.separator();
                if ui
                    .add_enabled(
                        node_path.is_some(),
                        egui::Button::new("Open").min_size(Vec2::new(ui.available_width(), 24.0)),
                    )
                    .clicked()
                {
                    if let Some(path) = &node_path {
                        app.apply_platform_result("Open", open_path(path));
                    }
                    ui.close();
                }
                if ui
                    .add_enabled(
                        node_path.is_some(),
                        egui::Button::new(reveal_action_label())
                            .min_size(Vec2::new(ui.available_width(), 24.0)),
                    )
                    .clicked()
                {
                    if let Some(path) = &node_path {
                        app.apply_platform_result(
                            reveal_action_label(),
                            reveal_in_file_manager(path),
                        );
                    }
                    ui.close();
                }
                if ui
                    .add_enabled(
                        node_path.is_some(),
                        egui::Button::new("Copy Path")
                            .min_size(Vec2::new(ui.available_width(), 24.0)),
                    )
                    .clicked()
                {
                    if let Some(path) = &node_path {
                        ui.ctx().copy_text(path.display().to_string());
                    }
                    ui.close();
                }
                ui.separator();
                let trash_response = ui.add_enabled(
                    node_path.is_some(),
                    egui::Button::new("Move to Trash")
                        .min_size(Vec2::new(ui.available_width(), 24.0)),
                );
                let trash_response = if node_path.is_none() {
                    trash_response.on_hover_text("Virtual nodes cannot be moved to Trash")
                } else {
                    trash_response.on_hover_text("Move this item to Trash")
                };
                if trash_response.clicked() {
                    app.move_node_to_trash(node_id);
                    ui.close();
                }
            });
        }
    });

    let context_menu_open = response.context_menu_opened();

    if !context_menu_open {
        app.context_menu_target_id = None;
    }

    if !context_menu_open {
        if let Some(node_id) = app.hovered_id {
            if let Some(pos) = response.hover_pos() {
                app.show_hover_tooltip(ui, node_id, pos);
            }
        }
    }
}
