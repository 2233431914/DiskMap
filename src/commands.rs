//! Command registry for core app actions.
//!
//! Each `Command` is a labelled action that takes a `&mut DiskMapApp`
//! and runs synchronously. The GUI no longer exposes a command palette,
//! but these lightweight actions remain useful for tests and any future
//! keyboard command surface.
//!
//! The registry is static and built once at app startup. We don't
//! load user-defined commands from disk — a self-use tool doesn't
//! need a plugin surface.
//!
//! Substring match on `id` + `label` is case-insensitive. That's not
//! full fuzzy match, but it covers the cases the user actually hits
//! ("scan", "home", "depth") and avoids pulling in a fuzzy
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
        self.id.to_ascii_lowercase().contains(&q) || self.label.to_ascii_lowercase().contains(&q)
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
            id: "toggle-color",
            label: "Toggle color by extension",
            hint: "Switch between directory-depth colors and extension-based colors",
            run: |app| {
                app.color_by_extension = !app.color_by_extension;
                app.status = format!(
                    "Color mode: {}",
                    if app.color_by_extension {
                        "extension"
                    } else {
                        "depth"
                    }
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
                    if app.search_filter_enabled {
                        "on"
                    } else {
                        "off"
                    }
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
            id: "refresh-treemap-layout",
            label: "Refresh treemap layout",
            hint: "Recompute the fixed treemap layout for the current view",
            run: |app| app.refresh_treemap_layout(),
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
            id: "scan-root",
            label: "Rescan from scan root",
            hint: "",
            run: dummy_run,
        }];
        assert_eq!(filter_commands("root", &cmds).len(), 1);
        assert_eq!(filter_commands("SCAN", &cmds).len(), 1);
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
        for required in ["scan", "scan-root", "go-home", "go-up", "go-selected"] {
            assert!(
                ids.contains(&required),
                "builtin registry missing command: {required}"
            );
        }
        for removed in [
            "apply-rules",
            "analyze-duplicates",
            "analyze-insights",
            "snapshot-diff",
            "export-diagnostics",
            "export-rules",
            "toggle-watch",
        ] {
            assert!(
                !ids.contains(&removed),
                "non-core command still registered: {removed}"
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
        assert_eq!(filter_commands("export", &cmds).len(), 0);
        assert!(filter_commands("toggle", &cmds).len() >= 2);
        assert!(filter_commands("zzznotacommand", &cmds).is_empty());
    }
}
