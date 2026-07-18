//! Treemap rendering: fixed-area layout caching, selection, context menu, and
//! hover tooltip. Extracted from `app.rs` so the per-frame render path reads
//! top-to-bottom in one file.
//!
//! Helper paint routines (`paint_visual`, `extension_color_for_visual`,
//! `show_hover_tooltip`) stay as methods on `DiskMapApp` in `app.rs` because
//! they are also reused by other code paths; this module calls them through
//! the `app` argument.

use super::super::treemap_state::TreemapLayoutRequest;
use super::super::DiskMapApp;
use crate::app::{
    find_hovered_visual, palette, truncate_middle, CONTEXT_MENU_MAX_TITLE_CHARS,
    CONTEXT_MENU_MIN_WIDTH,
};
use crate::format::format_bytes;
use crate::i18n::TextKey;
use crate::platform::{open_path, reveal_in_file_manager};
use crate::treemap::VisualKind;
use eframe::egui::{self, RichText, Sense, Vec2};

pub fn show(ui: &mut egui::Ui, app: &mut DiskMapApp) {
    let p = palette(ui.ctx());
    let available = ui.max_rect().intersect(ui.clip_rect());
    if available.width() <= 0.0 || available.height() <= 0.0 {
        app.treemap.clear();
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

    if let Some(elapsed) = app.treemap.layout_if_due(
        &mut app.tree,
        TreemapLayoutRequest {
            root: root_id,
            canvas_rect: available,
            max_depth: app.max_depth,
            search_state: app.search.state(),
            filter_to_search: app.search_filter_enabled,
            scanning: app.scan.is_scanning(),
        },
    ) {
        app.scan.record_layout_recompute(elapsed);
    }

    app.hovered_visual_kind =
        find_hovered_visual(app.treemap.visuals(), response.hover_pos()).map(|visual| visual.kind);
    app.hovered_id = app.hovered_visual_kind.and_then(|kind| match kind {
        VisualKind::Node(node_id) => Some(node_id),
        VisualKind::SmallFiles { .. } => None,
    });
    if response.secondary_clicked() {
        app.context_menu_target_id = app.hovered_id;
    }

    for visual in app.treemap.visuals() {
        app.paint_visual(ui, &painter, visual);
    }
    if response.double_clicked() {
        match app.hovered_visual_kind {
            Some(VisualKind::Node(node_id)) => {
                if !app.tree.node(node_id).children.is_empty() {
                    app.enter_root(node_id, true);
                } else {
                    app.navigation.set_selected_id(Some(node_id));
                }
            }
            Some(VisualKind::SmallFiles { parent_id, .. }) => {
                app.navigation.set_selected_id(Some(parent_id));
            }
            None => app.navigation.set_selected_id(None),
        }
    } else if response.clicked() {
        match app.hovered_visual_kind {
            Some(VisualKind::Node(node_id)) => app.navigation.set_selected_id(Some(node_id)),
            Some(VisualKind::SmallFiles { parent_id, .. }) => {
                app.navigation.set_selected_id(Some(parent_id));
            }
            None => app.navigation.set_selected_id(None),
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
                        egui::Button::new(format!(
                            "{}  {}",
                            egui_phosphor::regular::FOLDER_OPEN,
                            app.text(TextKey::Open)
                        ))
                        .min_size(Vec2::new(ui.available_width(), 28.0)),
                    )
                    .clicked()
                {
                    if let Some(path) = &node_path {
                        let action = app.text(TextKey::Open);
                        app.apply_platform_result(action, open_path(path));
                    }
                    ui.close();
                }
                if ui
                    .add_enabled(
                        node_path.is_some(),
                        egui::Button::new(format!(
                            "{}  {}",
                            egui_phosphor::regular::FOLDER_OPEN,
                            app.reveal_action_text()
                        ))
                        .min_size(Vec2::new(ui.available_width(), 28.0)),
                    )
                    .clicked()
                {
                    if let Some(path) = &node_path {
                        let action = app.reveal_action_text();
                        app.apply_platform_result(action, reveal_in_file_manager(path));
                    }
                    ui.close();
                }
                if ui
                    .add_enabled(
                        node_path.is_some(),
                        egui::Button::new(format!(
                            "{}  {}",
                            egui_phosphor::regular::CLIPBOARD,
                            app.text(TextKey::CopyPath)
                        ))
                        .min_size(Vec2::new(ui.available_width(), 28.0)),
                    )
                    .clicked()
                {
                    if let Some(path) = &node_path {
                        ui.ctx().copy_text(path.display().to_string());
                    }
                    ui.close();
                }
                ui.separator();
                let trash_label = if app.trash_confirm_target_id == Some(node_id) {
                    app.text(TextKey::ConfirmMoveToTrash)
                } else {
                    app.text(TextKey::MoveToTrash)
                };
                let trash_response = ui.add_enabled(
                    node_path.is_some(),
                    egui::Button::new(format!("{}  {trash_label}", egui_phosphor::regular::TRASH))
                        .min_size(Vec2::new(ui.available_width(), 28.0)),
                );
                let trash_response = if node_path.is_none() {
                    trash_response.on_hover_text(app.text(TextKey::VirtualNodeNoTrash))
                } else {
                    trash_response.on_hover_text(app.text(TextKey::MoveToTrashHint))
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
        if let Some(kind) = app.hovered_visual_kind {
            if let Some(pos) = response.hover_pos() {
                match kind {
                    VisualKind::Node(node_id) => app.show_hover_tooltip(ui, node_id, pos),
                    VisualKind::SmallFiles {
                        parent_id,
                        count,
                        size,
                    } => app.show_small_files_hover_tooltip(ui, parent_id, count, size, pos),
                }
            }
        }
    }
}
