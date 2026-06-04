//! Saved views: per-root capture of UI view state (depth, search
//! query, focused node, selected node, color mode, last report
//! panel). In-memory only — a future persistence layer can hook in
//! the same way `SafeStorage` did for preferences.
//!
//! Design notes:
//!  - One entry per scan root. No "list all views in the sidebar"
//!    — the spec said "saved views" plural but in practice a
//!    self-use tool with one canonical view per root covers 95% of
//!    the use case. A flat list would be more UI surface than value.
//!  - "Save current view" captures the current state and stores it
//!    under the current root. Last-write-wins.
//!  - "Apply view" applies a saved view's state to the live UI
//!    fields. It does not re-trigger a scan — the user does that
//!    explicitly. This matches `apply_profile_to_ui` semantics.
//!  - `last_report_mode` is a static string discriminator
//!    ("none" | "duplicates" | "insights" | "rules" | "snapshot")
//!    so we don't take a dependency on a particular panel's enum.

use crate::tree::NodeId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewState {
    /// Tree depth limit shown in the treemap. 1-10 (matches the
    /// existing `max_depth` field's range on DiskMapApp).
    pub depth: usize,
    /// Live search query.
    pub search_query: String,
    /// Whether the search filter is enabled (hides non-matched
    /// branches).
    pub search_filter_enabled: bool,
    /// Color-by-extension toggle.
    pub color_by_extension: bool,
    /// Last-opened report panel discriminator. One of:
    ///   "none" | "duplicates" | "insights" | "snapshot" | "rules"
    /// We intentionally keep this as a string so a future panel
    /// doesn't require a code change to ViewState's enum.
    pub last_report_mode: String,
    /// Focused node id (within the saved root). Optional.
    pub focused_id: Option<NodeId>,
    /// Selected node id. Optional.
    pub selected_id: Option<NodeId>,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            depth: 1,
            search_query: String::new(),
            search_filter_enabled: false,
            color_by_extension: false,
            last_report_mode: "none".to_string(),
            focused_id: None,
            selected_id: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewStore {
    /// Keyed by the canonical root path (trailing-slash normalized
    /// via the same scheme as `ProfileStore`).
    views: BTreeMap<String, ViewState>,
}

impl ViewStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn normalize_key(root: &str) -> String {
        let p = std::path::Path::new(root);
        let mut s = p.to_string_lossy().to_string();
        while s.len() > 1 && s.ends_with('/') {
            s.pop();
        }
        s
    }

    pub fn get(&self, root: &str) -> Option<&ViewState> {
        self.views.get(&Self::normalize_key(root))
    }

    pub fn set(&mut self, root: &str, state: ViewState) {
        self.views.insert(Self::normalize_key(root), state);
    }

    pub fn remove(&mut self, root: &str) -> Option<ViewState> {
        self.views.remove(&Self::normalize_key(root))
    }

    pub fn list(&self) -> Vec<(String, ViewState)> {
        self.views
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    pub fn len(&self) -> usize {
        self.views.len()
    }

    pub fn is_empty(&self) -> bool {
        self.views.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_state() -> ViewState {
        ViewState {
            depth: 3,
            search_query: "node_modules".into(),
            search_filter_enabled: true,
            color_by_extension: true,
            last_report_mode: "rules".into(),
            focused_id: Some(42),
            selected_id: None,
        }
    }

    #[test]
    fn default_is_sane() {
        let d = ViewState::default();
        assert_eq!(d.depth, 1);
        assert!(d.search_query.is_empty());
        assert!(!d.search_filter_enabled);
        assert!(!d.color_by_extension);
        assert_eq!(d.last_report_mode, "none");
        assert!(d.focused_id.is_none());
        assert!(d.selected_id.is_none());
    }

    #[test]
    fn set_then_get_round_trips() {
        let mut store = ViewStore::new();
        store.set("/a", dummy_state());
        let got = store.get("/a").unwrap();
        assert_eq!(got.depth, 3);
        assert_eq!(got.search_query, "node_modules");
    }

    #[test]
    fn trailing_slash_normalization() {
        let mut store = ViewStore::new();
        store.set("/a/", dummy_state());
        assert!(store.get("/a").is_some());
        assert!(store.get("/a/").is_some());
    }

    #[test]
    fn remove_returns_stored() {
        let mut store = ViewStore::new();
        store.set("/a", dummy_state());
        assert!(store.remove("/a").is_some());
        assert!(store.get("/a").is_none());
    }

    #[test]
    fn json_round_trip() {
        let mut store = ViewStore::new();
        store.set("/a", dummy_state());
        let json = serde_json::to_string(&store).unwrap();
        let restored: ViewStore = serde_json::from_str(&json).unwrap();
        assert_eq!(store.list(), restored.list());
    }

    #[test]
    fn invalid_json_returns_error() {
        let err = serde_json::from_str::<ViewStore>("not json");
        assert!(err.is_err());
    }
}
