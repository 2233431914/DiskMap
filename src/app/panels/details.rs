//! Sidebar details panel: search, selected-node metadata, and core file actions.

use super::super::search_nav::SearchDirection;
use super::super::DiskMapApp;
use crate::app::{icon_text_button, palette, section_divider};
use crate::format::format_bytes;
use crate::i18n::TextKey;
use crate::platform::{open_path, reveal_in_file_manager};
use crate::scanner::{size_basis_detail, size_basis_label};
use eframe::egui::{self, Color32, CornerRadius, Margin, RichText, Stroke, Vec2};

pub fn show(ui: &mut egui::Ui, app: &mut DiskMapApp) {
    let p = palette(ui.ctx());
    ui.add_space(4.0);
    ui.label(
        RichText::new(app.text(TextKey::Controls).to_uppercase())
            .size(11.0)
            .strong()
            .color(p.text_muted),
    );
    ui.add_space(2.0);
    section_divider(ui, p);
    ui.add_space(8.0);
    show_controls_section(ui, app, p);
    super::analysis::show(ui, app);
    ui.add_space(12.0);
    ui.label(
        RichText::new(app.text(TextKey::Details).to_uppercase())
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
    if app
        .trash_confirm_target_id
        .is_some_and(|confirmed_id| confirmed_id != node_id)
    {
        app.clear_trash_confirmation();
    }

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
    let kind_label = app.locale.node_kind(node_kind, child_count > 0);

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
                format!("{kind_label} · {}", app.locale.item_count(child_count))
            } else {
                kind_label.to_string()
            };
            ui.label(RichText::new(meta).small().color(p.text_muted));
            if !node_scanned {
                ui.label(
                    RichText::new(app.text(TextKey::ScanningInProgress))
                        .small()
                        .color(p.accent),
                );
            }
            if !app.search.query().is_empty() {
                let (txt, color) = if matched {
                    (app.text(TextKey::MatchesSearch), p.accent)
                } else {
                    (app.text(TextKey::NoSearchMatch), p.text_faint)
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
                ui.label(
                    RichText::new(format!("{}: {err}", app.text(TextKey::Error))).color(p.danger),
                );
            });
    }

    ui.add_space(12.0);
    ui.label(
        RichText::new(app.text(TextKey::Primary).to_uppercase())
            .size(10.0)
            .strong()
            .color(p.text_faint),
    );
    ui.add_space(4.0);
    let path_available = node_path.is_some();
    ui.columns(2, |cols| {
        let w0 = cols[0].available_width();
        if icon_text_button(
            &mut cols[0],
            path_available,
            egui_phosphor::regular::FOLDER_OPEN,
            app.text(TextKey::Open),
            w0,
        )
        .clicked()
        {
            if let Some(path) = &node_path {
                let action = app.text(TextKey::Open);
                app.apply_platform_result(action, open_path(path));
            }
        }
        let w1 = cols[1].available_width();
        let reveal_response = cols[1].add_enabled(
            path_available,
            egui::Button::new(format!(
                "{}  {}",
                egui_phosphor::regular::FOLDER_OPEN,
                app.text(TextKey::Reveal)
            ))
            .min_size(Vec2::new(w1, 32.0)),
        );
        let reveal_response = reveal_response.on_hover_text(app.reveal_action_text());
        if reveal_response.clicked() {
            if let Some(path) = &node_path {
                let action = app.reveal_action_text();
                app.apply_platform_result(action, reveal_in_file_manager(path));
            }
        }
    });

    ui.add_space(10.0);
    ui.label(
        RichText::new(app.text(TextKey::Utility).to_uppercase())
            .size(10.0)
            .strong()
            .color(p.text_faint),
    );
    ui.add_space(4.0);
    let copy_width = ui.available_width();
    if ui
        .add_enabled(
            path_available,
            egui::Button::new(format!(
                "{}  {}",
                egui_phosphor::regular::CLIPBOARD,
                app.text(TextKey::CopyPath)
            ))
            .min_size(Vec2::new(copy_width, 30.0)),
        )
        .clicked()
    {
        if let Some(path) = &node_path {
            ui.ctx().copy_text(path.display().to_string());
        }
    }
    ui.add_space(4.0);
    let trash_width = ui.available_width();
    let trash_label = if app.trash_confirm_target_id == Some(node_id) {
        app.text(TextKey::ConfirmMoveToTrash)
    } else {
        app.text(TextKey::MoveToTrash)
    };
    let trash_response = ui.add_enabled(
        path_available,
        egui::Button::new(format!("{}  {trash_label}", egui_phosphor::regular::TRASH))
            .min_size(Vec2::new(trash_width, 30.0))
            .fill(Color32::from_rgba_unmultiplied(
                p.danger.r(),
                p.danger.g(),
                p.danger.b(),
                34,
            )),
    );
    let trash_response = if !path_available {
        trash_response.on_hover_text(app.text(TextKey::VirtualNodeNoTrash))
    } else {
        trash_response.on_hover_text(app.text(TextKey::MoveToTrashHint))
    };
    if trash_response.clicked() {
        app.move_node_to_trash(node_id);
    }

    if let Some(parent) = node_parent {
        ui.add_space(10.0);
        ui.label(
            RichText::new(app.text(TextKey::Parent).to_uppercase())
                .size(10.0)
                .strong()
                .color(p.text_faint),
        );
        ui.add_space(4.0);
        let parent_name = app.tree.node(parent).name.clone();
        if ui
            .add(
                egui::Button::new(
                    RichText::new(format!(
                        "{}  {parent_name}",
                        egui_phosphor::regular::ARROW_UP
                    ))
                    .color(p.text),
                )
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
        RichText::new(app.text(TextKey::Search).to_uppercase())
            .size(10.0)
            .strong()
            .color(p.text_faint),
    );
    ui.add_space(4.0);
    let search_hint = app.text(TextKey::SearchHint);
    let search_response = ui.add_sized(
        [ui.available_width(), 28.0],
        egui::TextEdit::singleline(app.search.input_mut()).hint_text(search_hint),
    );
    if app.search_focus_requested {
        search_response.request_focus();
        app.search_focus_requested = false;
    }
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
                egui::Button::new(egui_phosphor::regular::ARROW_UP).min_size(Vec2::new(30.0, 26.0)),
            )
            .on_hover_text(app.text(TextKey::PreviousMatch))
            .clicked()
        {
            app.navigate_search_match(SearchDirection::Previous);
        }
        if ui
            .add_enabled(
                can_nav,
                egui::Button::new(egui_phosphor::regular::ARROW_DOWN)
                    .min_size(Vec2::new(30.0, 26.0)),
            )
            .on_hover_text(app.text(TextKey::NextMatch))
            .clicked()
        {
            app.navigate_search_match(SearchDirection::Next);
        }
        if ui
            .add_enabled(
                !app.search.input().is_empty(),
                egui::Button::new(format!(
                    "{}  {}",
                    egui_phosphor::regular::X,
                    app.text(TextKey::ClearSearch)
                ))
                .min_size(Vec2::new(64.0, 26.0)),
            )
            .on_hover_text(app.text(TextKey::ClearSearch))
            .clicked()
        {
            app.clear_search();
        }
        let filter_label = app.text(TextKey::Filter);
        let filter_hint = app.text(TextKey::FilterSearch);
        if ui
            .checkbox(&mut app.search_filter_enabled, filter_label)
            .on_hover_text(filter_hint)
            .changed()
        {
            app.mark_layout_dirty_now();
        }
    });
    let match_text = if app.search.query().is_empty() {
        app.text(TextKey::NoSearchQuery).to_string()
    } else if app.search.is_dirty() {
        format!(
            "{} {} · {}",
            app.search.state().match_count(),
            app.text(TextKey::Matches),
            app.text(TextKey::Updating)
        )
    } else if let Some(index) = app.search.active_match() {
        format!(
            "{} / {} {}",
            index + 1,
            app.search.state().match_count(),
            app.text(TextKey::Matches)
        )
    } else {
        format!(
            "{} {}",
            app.search.state().match_count(),
            app.text(TextKey::Matches)
        )
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
        RichText::new(app.text(TextKey::View).to_uppercase())
            .size(10.0)
            .strong()
            .color(p.text_faint),
    );
    ui.add_space(4.0);
    let color_label = app.text(TextKey::ColorByExtension);
    if ui
        .checkbox(&mut app.color_by_extension, color_label)
        .on_hover_text(color_label)
        .changed()
    {
        app.mark_layout_dirty_now();
    }
}
