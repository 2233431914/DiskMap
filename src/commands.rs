//! Command registry for the command palette (Cmd+K).
//!
//! Each `Command` is a labelled action that takes a `&mut DiskMapApp`
//! and runs synchronously. The palette UI presents a filtered list
//! of these commands and runs the chosen one when the user presses
//! Enter.
//!
//! The registry is static and built once at app startup. We don't
//! load user-defined commands from disk — a self-use tool doesn't
//! need a plugin surface.
//!
//! Substring match on `id` + `label` is case-insensitive. That's not
//! full fuzzy match, but it covers the cases the user actually hits
//! ("scan", "apply", "open", "export") and avoids pulling in a fuzzy
//! dep.

use crate::app::DiskMapApp;

#[derive(Debug)]
pub struct Command {
    pub id: &'static str,
    pub label: &'static str,
    pub hint: &'static str,
    pub run: fn(&mut DiskMapApp),
}

impl Command {
    /// Substring match on `id` + `label` (case-insensitive). Empty
    /// query matches everything.
    pub fn matches(&self, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }
        let q = query.to_ascii_lowercase();
        self.id.to_ascii_lowercase().contains(&q)
            || self.label.to_ascii_lowercase().contains(&q)
    }
}

/// Build the built-in command list. Pure data — no side effects at
/// construction time. Called once at app startup.
pub fn builtin_commands() -> Vec<Command> {
    vec![
        Command {
            id: "scan",
            label: "Rescan current path",
            hint: "Re-run the scanner on the current focused path",
            run: |app| app.rescan_focused_subtree(),
        },
        Command {
            id: "scan-root",
            label: "Rescan from scan root",
            hint: "Re-run the scanner on the original scan root",
            run: |app| app.rescan_scan_root(),
        },
        Command {
            id: "go-home",
            label: "Go to scan root",
            hint: "Reset the focused root to the original scan root",
            run: |app| app.return_to_scan_root(),
        },
        Command {
            id: "go-up",
            label: "Go up one level",
            hint: "Focus the parent of the current node",
            run: |app| app.navigate_back(),
        },
        Command {
            id: "go-selected",
            label: "Enter selected directory",
            hint: "Drill into the currently selected directory",
            run: |app| {
                let _ = app.enter_selected_directory();
            },
        },
        Command {
            id: "apply-rules",
            label: "Apply Rules",
            hint: "Run all enabled rules against the focused subtree",
            run: |app| {
                let _ = app.evaluate_current_rules();
            },
        },
        Command {
            id: "analyze-duplicates",
            label: "Analyze duplicates",
            hint: "Find same-name same-size file groups in the focused subtree",
            run: |app| app.analyze_duplicate_candidates(),
        },
        Command {
            id: "analyze-insights",
            label: "Analyze insights",
            hint: "Compute age + type breakdown for the focused subtree",
            run: |app| app.analyze_file_insights(),
        },
        Command {
            id: "snapshot-diff",
            label: "Compare to last snapshot",
            hint: "Diff the current scan against the previous in-memory snapshot",
            run: |app| app.update_snapshot_comparison(),
        },
        Command {
            id: "export-diagnostics",
            label: "Export diagnostics bundle",
            hint: "Write a snapshot of app state to disk-map-diagnostics-<ts>/",
            run: |app| {
                let dest = std::env::current_dir()
                    .unwrap_or_else(|_| std::path::PathBuf::from("."));
                match app.export_diagnostics(&dest) {
                    Ok(path) => app.status = format!("Wrote diagnostics: {}", path.display()),
                    Err(e) => app.status = format!("Diagnostics export failed: {e}"),
                }
                app.pending_repaint = true;
            },
        },
        Command {
            id: "export-rules",
            label: "Export rules to file",
            hint: "Write the current ruleset to disk-map-rules-<ts>.json",
            run: |app| {
                let dest = std::env::current_dir()
                    .unwrap_or_else(|_| std::path::PathBuf::from("."));
                match crate::rules::export_ruleset_to_dir(&app.rules, &dest) {
                    Ok(path) => app.status = format!("Wrote rules: {}", path.display()),
                    Err(e) => app.status = format!("Rules export failed: {e}"),
                }
                app.pending_repaint = true;
            },
        },
        Command {
            id: "toggle-watch",
            label: "Toggle Watch",
            hint: "Enable or disable filesystem watch for the current scan",
            run: |app| {
                app.realtime_watch_enabled = !app.realtime_watch_enabled;
                app.status = format!(
                    "Watch {}",
                    if app.realtime_watch_enabled { "on" } else { "off" }
                );
                app.pending_repaint = true;
            },
        },
        Command {
            id: "toggle-color",
            label: "Toggle color by extension",
            hint: "Switch between directory-depth colors and extension-based colors",
            run: |app| {
                app.color_by_extension = !app.color_by_extension;
                app.status = format!(
                    "Color mode: {}",
                    if app.color_by_extension { "extension" } else { "depth" }
                );
                app.layout_dirty = true;
                app.pending_repaint = true;
            },
        },
        Command {
            id: "toggle-search-filter",
            label: "Toggle search filter",
            hint: "When on, only matched branches are visible in the treemap",
            run: |app| {
                app.search_filter_enabled = !app.search_filter_enabled;
                app.status = format!(
                    "Search filter: {}",
                    if app.search_filter_enabled { "on" } else { "off" }
                );
                app.layout_dirty = true;
                app.pending_repaint = true;
            },
        },
        Command {
            id: "clear-search",
            label: "Clear search",
            hint: "Empty the search query and show all nodes",
            run: |app| app.clear_search(),
        },
        Command {
            id: "increase-depth",
            label: "Increase treemap depth",
            hint: "Show one more level of nesting in the treemap",
            run: |app| {
                let _ = app.increase_depth();
            },
        },
        Command {
            id: "decrease-depth",
            label: "Decrease treemap depth",
            hint: "Show one less level of nesting in the treemap",
            run: |app| {
                let _ = app.decrease_depth();
            },
        },
        Command {
            id: "reset-camera",
            label: "Reset treemap camera",
            hint: "Pan/zoom back to the default view",
            run: |app| app.reset_camera(),
        },
    ]
}

/// Filter the registry to commands matching the query, preserving
/// the order from `builtin_commands()`.
pub fn filter_commands<'a>(query: &str, commands: &'a [Command]) -> Vec<&'a Command> {
    commands.iter().filter(|c| c.matches(query)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_run(_: &mut DiskMapApp) {}

    #[test]
    fn empty_query_matches_everything() {
        let cmds = vec![Command {
            id: "x",
            label: "Y",
            hint: "",
            run: dummy_run,
        }];
        assert_eq!(filter_commands("", &cmds).len(), 1);
        assert_eq!(filter_commands("anything", &cmds).len(), 0);
    }

    #[test]
    fn case_insensitive_substring_match_on_id() {
        let cmds = vec![Command {
            id: "export-rules",
            label: "Export rules to file",
            hint: "",
            run: dummy_run,
        }];
        assert_eq!(filter_commands("rules", &cmds).len(), 1);
        assert_eq!(filter_commands("EXPORT", &cmds).len(), 1);
        assert_eq!(filter_commands("exp-rt", &cmds).len(), 0);
    }

    #[test]
    fn case_insensitive_substring_match_on_label() {
        let cmds = vec![Command {
            id: "x",
            label: "Rescan current path",
            hint: "",
            run: dummy_run,
        }];
        assert_eq!(filter_commands("rescan", &cmds).len(), 1);
        assert_eq!(filter_commands("PATH", &cmds).len(), 1);
    }

    #[test]
    fn builtin_registry_includes_core_commands() {
        let cmds = builtin_commands();
        let ids: Vec<&str> = cmds.iter().map(|c| c.id).collect();
        for required in [
            "scan",
            "go-home",
            "apply-rules",
            "export-diagnostics",
            "toggle-watch",
        ] {
            assert!(
                ids.contains(&required),
                "builtin registry missing command: {required}"
            );
        }
        // No duplicates
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "duplicate command ids");
    }

    #[test]
    fn builtin_filter_for_typical_queries() {
        let cmds = builtin_commands();
        assert!(filter_commands("scan", &cmds).len() >= 2); // scan + scan-root
        assert!(filter_commands("export", &cmds).len() >= 2);
        assert!(filter_commands("toggle", &cmds).len() >= 3);
        assert!(filter_commands("zzznotacommand", &cmds).is_empty());
    }
}
