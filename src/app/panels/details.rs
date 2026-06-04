//! Sidebar details panel: selected node metadata, primary/utility actions,
//! export and analysis entry points. Extracted from `app.rs` so the panel
//! layout reads top-to-bottom in one file.

use super::super::DiskMapApp;
use crate::app::{accent_button, describe_node_kind, palette, section_divider};
use crate::export::ExportFormat;
use crate::format::format_bytes;
use crate::platform::{open_path, reveal_in_finder};
use crate::scanner::{size_basis_detail, size_basis_label};
use eframe::egui::{self, Color32, CornerRadius, Margin, RichText, Stroke, Vec2};

pub fn show(ui: &mut egui::Ui, app: &mut DiskMapApp) {
    let p = palette(ui.ctx());
    ui.add_space(4.0);
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
        app.show_search_section(ui, p);
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
        if cols[1]
            .add_enabled(
                path_available,
                egui::Button::new("Reveal").min_size(Vec2::new(w1, 32.0)),
            )
            .clicked()
        {
            if let Some(path) = &node_path {
                app.apply_platform_result("Reveal", reveal_in_finder(path));
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
    ui.add_space(4.0);
    ui.add_sized(
        [ui.available_width(), 26.0],
        egui::TextEdit::singleline(&mut app.protected_paths_input)
            .hint_text("Protected paths"),
    )
    .on_hover_text("Extra protected roots; comma, semicolon, or newline separated");

    let trash_width = ui.available_width();
    let trash_response = ui.add_enabled(
        path_available,
        egui::Button::new("Move to Trash").min_size(Vec2::new(trash_width, 28.0)),
    );
    let trash_response = if !path_available {
        trash_response.on_hover_text("Virtual nodes cannot be moved to Trash")
    } else {
        trash_response.on_hover_text("Move this item to Trash")
    };
    if trash_response.clicked() {
        app.move_node_to_trash(node_id);
    }
    ui.add_space(4.0);
    let focused_export_id = app.navigation.focused_root();
    let scan_root_export_id = app.tree.root;
    let scan_root_is_focused =
        focused_export_id.is_some() && focused_export_id == scan_root_export_id;
    ui.columns(2, |cols| {
        let can_export = focused_export_id.is_some();
        let w0 = cols[0].available_width();
        if cols[0]
            .add_enabled(
                can_export,
                egui::Button::new("Export View CSV").min_size(Vec2::new(w0, 28.0)),
            )
            .clicked()
        {
            app.export_focused_subtree(ExportFormat::Csv);
        }
        let w1 = cols[1].available_width();
        if cols[1]
            .add_enabled(
                can_export,
                egui::Button::new("Export View JSON").min_size(Vec2::new(w1, 28.0)),
            )
            .clicked()
        {
            app.export_focused_subtree(ExportFormat::Json);
        }
    });
    if !scan_root_is_focused {
        ui.add_space(4.0);
        ui.columns(2, |cols| {
            let can_export = scan_root_export_id.is_some();
            let w0 = cols[0].available_width();
            if cols[0]
                .add_enabled(
                    can_export,
                    egui::Button::new("Export Root CSV").min_size(Vec2::new(w0, 28.0)),
                )
                .clicked()
            {
                app.export_scan_root(ExportFormat::Csv);
            }
            let w1 = cols[1].available_width();
            if cols[1]
                .add_enabled(
                    can_export,
                    egui::Button::new("Export Root JSON").min_size(Vec2::new(w1, 28.0)),
                )
                .clicked()
            {
                app.export_scan_root(ExportFormat::Json);
            }
        });
    }
    ui.add_space(4.0);
    let report_width = ui.available_width();
    if ui
        .add_enabled(
            focused_export_id.is_some(),
            egui::Button::new("Export Report JSON").min_size(Vec2::new(report_width, 28.0)),
        )
        .on_hover_text("Export current view data plus metadata needed to reproduce this view")
        .clicked()
    {
        app.export_focused_report_json();
    }
    ui.add_space(4.0);
    let duplicate_width = ui.available_width();
    if ui
        .add_enabled(
            focused_export_id.is_some() && !app.scan.is_scanning(),
            egui::Button::new("Analyze Duplicates").min_size(Vec2::new(duplicate_width, 28.0)),
        )
        .on_hover_text("Read-only heuristic: same file name and same size in the current view")
        .clicked()
    {
        app.analyze_duplicate_candidates();
    }
    ui.add_space(4.0);
    let insight_width = ui.available_width();
    if ui
        .add_enabled(
            focused_export_id.is_some() && !app.scan.is_scanning(),
            egui::Button::new("Analyze Insights").min_size(Vec2::new(insight_width, 28.0)),
        )
        .on_hover_text("Read-only age buckets and extension category summary for this view")
        .clicked()
    {
        app.analyze_file_insights();
    }

    // Per-root scan option profile controls. Only meaningful when
    // there's a real path (not a virtual aggregate node) and that path
    // is the focused root (or scan root) — we key profiles by the
    // path string the user typed.
    if let Some(view_root_path) = node_path.as_ref() {
        let view_root = view_root_path.to_string_lossy().to_string();
        ui.add_space(10.0);
        ui.label(
            RichText::new("VIEW")
                .size(10.0)
                .strong()
                .color(p.text_faint),
        );
        ui.add_space(4.0);
        let saved_view = app.views.get(&view_root);
        if let Some(view) = saved_view {
            ui.label(
                RichText::new(format!(
                    "Saved: depth {}, filter={}, last={}{}",
                    view.depth,
                    if view.search_filter_enabled { "on" } else { "off" },
                    view.last_report_mode,
                    if view.search_query.is_empty() {
                        String::new()
                    } else {
                        format!(", q=\"{}\"", view.search_query)
                    },
                ))
                .small()
                .color(p.text_muted),
            );
        } else {
            ui.label(
                RichText::new("No saved view for this root")
                    .small()
                    .color(p.text_muted),
            );
        }
        ui.columns(2, |cols| {
            let w0 = cols[0].available_width();
            if cols[0]
                .add(egui::Button::new("Save current view").min_size(Vec2::new(w0, 24.0)))
                .on_hover_text("Capture depth, search query, focused/selected node, color mode, and last opened report panel under this root")
                .clicked()
            {
                app.save_current_view(&view_root);
            }
            let w1 = cols[1].available_width();
            let has_view = app.views.get(&view_root).is_some();
            if cols[1]
                .add_enabled(
                    has_view,
                    egui::Button::new("Apply saved view").min_size(Vec2::new(w1, 24.0)),
                )
                .clicked()
            {
                app.apply_saved_view(&view_root);
            }
        });
    }

    if let Some(profile_root_path) = node_path.as_ref() {
        let profile_root = profile_root_path.to_string_lossy().to_string();
        ui.add_space(10.0);
        ui.label(
            RichText::new("PROFILE")
                .size(10.0)
                .strong()
                .color(p.text_faint),
        );
        ui.add_space(4.0);
        let profile_count = app.profiles.len();
        if profile_count > 0 {
            ui.label(
                RichText::new(format!(
                    "{} profile(s) stored (this root: {})",
                    profile_count,
                    if app.profiles.get(&profile_root).is_some() {
                        "saved"
                    } else {
                        "not saved"
                    }
                ))
                .small()
                .color(p.text_muted),
            );
        } else {
            ui.label(
                RichText::new("No profiles yet — saved options are remembered per root")
                    .small()
                    .color(p.text_muted),
            );
        }
        ui.columns(2, |cols| {
            let w0 = cols[0].available_width();
            if cols[0]
                .add(
                    egui::Button::new("Save for this root")
                        .min_size(Vec2::new(w0, 24.0)),
                )
                .clicked()
            {
                app.save_current_as_profile(&profile_root);
            }
            let w1 = cols[1].available_width();
            let has_profile = app.profiles.get(&profile_root).is_some();
            if cols[1]
                .add_enabled(
                    has_profile,
                    egui::Button::new("Apply profile")
                        .min_size(Vec2::new(w1, 24.0)),
                )
                .clicked()
            {
                app.apply_profile_to_ui(&profile_root);
            }
        });
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
    app.show_cleanup_queue_section(ui, p);
    app.show_snapshot_diff_section(ui, p);
    app.show_duplicate_report_section(ui, p);
    app.show_insight_report_section(ui, p);
    app.show_rules_section(ui, p);
    app.show_diagnostics_section(ui, p);
    app.show_search_section(ui, p);
}
