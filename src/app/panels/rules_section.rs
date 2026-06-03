//! Rules section: list + toggle + apply button. Read-only analysis
//! panel; the engine itself never mutates scan state except via
//! `ensure_sorted_children` (deterministic iteration order).

use super::super::DiskMapApp;
use super::super::Palette;
use crate::app::palette;
use crate::rules::{export_ruleset_to_dir, import_ruleset_from_path};
use eframe::egui::{self, RichText, Vec2};

const RULES_HIT_PREVIEW: usize = 8;

pub fn show_rules_section(ui: &mut egui::Ui, p: &Palette, app: &mut DiskMapApp) {
    ui.add_space(12.0);
    let enabled = app.rules.enabled_count();
    let total = app.rules.rules.len();
    ui.label(
        RichText::new(format!("RULES ({} of {} enabled)", enabled, total))
            .size(10.0)
            .strong()
            .color(p.text_faint),
    );
    ui.add_space(4.0);

    // Render each rule row. We iterate by index so the borrow checker
    // doesn't fight us over `app` access inside the closure.
    let mut i = 0;
    while i < app.rules.rules.len() {
        let (id, name, description) = {
            let r = &app.rules.rules[i];
            (r.id.clone(), r.name.clone(), r.description.clone())
        };
        ui.horizontal(|ui| {
            let mut enabled_flag = app.rules.rules[i].enabled;
            if ui.checkbox(&mut enabled_flag, "").changed() {
                if enabled_flag {
                    app.rules.enable(&id);
                } else {
                    app.rules.disable(&id);
                }
                app.pending_repaint = true;
            }
            ui.label(RichText::new(&name).color(p.text));
        });
        ui.label(
            RichText::new(&description)
                .small()
                .color(p.text_muted),
        );
        ui.add_space(2.0);
        i += 1;
    }

    ui.add_space(6.0);
    let button_width = ui.available_width();
    if ui
        .add(
            egui::Button::new("Apply Rules")
                .min_size(Vec2::new(button_width, 28.0)),
        )
        .on_hover_text("Run all enabled rules against the current focused subtree and cache the hits")
        .clicked()
    {
        app.evaluate_current_rules();
    }

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let w = ui.available_width() * 0.5 - 4.0;
        if ui
            .add(egui::Button::new("Export").min_size(Vec2::new(w, 24.0)))
            .on_hover_text("Write the current ruleset to disk-map-rules-<ts>.json in the current working directory")
            .clicked()
        {
            let dest = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            match export_ruleset_to_dir(&app.rules, &dest) {
                Ok(path) => {
                    app.status = format!("Wrote rules: {}", path.display());
                }
                Err(error) => {
                    app.record_error(format!("rules export failed: {error}"));
                    app.status = format!("Rules export failed: {error}");
                }
            }
            app.pending_repaint = true;
        }
        if ui
            .add(egui::Button::new("Import").min_size(Vec2::new(w, 24.0)))
            .on_hover_text("Load a ruleset JSON from the path below (replaces the current ruleset)")
            .clicked()
        {
            let raw = app.rules_import_path.trim().to_string();
            if raw.is_empty() {
                app.status = "Type a path to import from".to_string();
                app.pending_repaint = true;
            } else {
                match import_ruleset_from_path(std::path::Path::new(&raw)) {
                    Ok(ruleset) => {
                        app.rules = ruleset;
                        app.last_rule_hits = None;
                        app.rules_import_path.clear();
                        app.status = format!("Imported rules from {}", raw);
                    }
                    Err(error) => {
                        app.record_error(format!("rules import failed: {error}"));
                        app.status = format!("Rules import failed: {error}");
                    }
                }
                app.pending_repaint = true;
            }
        }
    });
    ui.add_sized(
        [ui.available_width(), 22.0],
        egui::TextEdit::singleline(&mut app.rules_import_path).hint_text("Path to rules JSON"),
    )
    .on_hover_text("Absolute or relative path to a rules JSON file. Cleared after a successful import.");

    if let Some(hits) = &app.last_rule_hits {
        ui.add_space(4.0);
        if hits.is_empty() {
            ui.label(
                RichText::new("No hits in current view")
                    .small()
                    .color(p.text_muted),
            );
        } else {
            ui.label(
                RichText::new(format!(
                    "{} hit(s) — top {}:",
                    hits.len(),
                    hits.len().min(RULES_HIT_PREVIEW)
                ))
                .small()
                .color(p.accent),
            );
            for hit in hits.iter().take(RULES_HIT_PREVIEW) {
                let label = match app.rules.get(&hit.rule_id) {
                    Some(rule) => format!("[{}] {}", rule.name, hit.reason),
                    None => format!("[{}] {}", hit.rule_id, hit.reason),
                };
                ui.label(
                    RichText::new(label)
                        .small()
                        .monospace()
                        .color(p.text_muted),
                );
            }
            if hits.len() > RULES_HIT_PREVIEW {
                ui.label(
                    RichText::new(format!("…and {} more", hits.len() - RULES_HIT_PREVIEW))
                        .small()
                        .color(p.text_faint),
                );
            }
        }
    }

    // Suppress unused-variable warnings if the helper ever shrinks.
    let _ = palette(ui.ctx());
}
