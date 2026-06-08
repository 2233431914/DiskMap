use super::DiskMapApp;
use crate::rules::{export_ruleset_to_dir, import_ruleset_from_path, preview_ruleset_import};
use std::path::{Path, PathBuf};

impl DiskMapApp {
    pub fn set_rule_enabled(&mut self, id: &str, enabled: bool) -> bool {
        let Some(rule) = self.rules.get(id) else {
            return false;
        };
        if rule.enabled == enabled {
            return false;
        }

        if enabled {
            self.rules.enable(id);
        } else {
            self.rules.disable(id);
        }
        self.last_rule_hits = None;
        self.persist_local_state();
        true
    }

    pub fn export_rules_to_current_dir(&mut self) {
        let dest = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        match export_ruleset_to_dir(&self.rules, &dest) {
            Ok(path) => {
                self.status = format!("Wrote rules: {}", path.display());
            }
            Err(error) => {
                self.record_error(format!("rules export failed: {error}"));
                self.status = format!("Rules export failed: {error}");
            }
        }
        self.pending_repaint = true;
    }

    pub fn preview_rules_import_from_input(&mut self) {
        let raw = self.rules_import_path.trim().to_string();
        if raw.is_empty() {
            self.pending_rules_import = None;
            self.status = "Type a path to import from".to_string();
            self.pending_repaint = true;
            return;
        }

        let path = Path::new(&raw);
        match import_ruleset_from_path(path) {
            Ok(ruleset) => {
                let preview = preview_ruleset_import(&self.rules, ruleset, PathBuf::from(&raw));
                self.status = format!(
                    "Preview rules import: {} rules, +{} / -{} / {} changed",
                    preview.incoming_rule_count,
                    preview.added_count,
                    preview.removed_count,
                    preview.changed_count
                );
                self.pending_rules_import = Some(preview);
            }
            Err(error) => {
                self.pending_rules_import = None;
                self.record_error(format!("rules import failed: {error}"));
                self.status = format!("Rules import failed: {error}");
            }
        }
        self.pending_repaint = true;
    }

    pub fn confirm_rules_import(&mut self) -> bool {
        let Some(preview) = self.pending_rules_import.take() else {
            self.status = "Rules import unavailable: no preview to apply".to_string();
            self.pending_repaint = true;
            return false;
        };
        let source = preview.source_path.display().to_string();
        self.rules = preview.ruleset;
        self.last_rule_hits = None;
        self.rules_import_path.clear();
        self.status = format!("Imported rules from {source}");
        self.persist_local_state();
        true
    }

    pub fn cancel_rules_import(&mut self) {
        if self.pending_rules_import.take().is_some() {
            self.status = "Cancelled rules import preview".to_string();
            self.pending_repaint = true;
        }
    }
}
