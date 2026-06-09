use crate::tree::{node_index, NodeId, TreeStore};
use crate::treemap::SearchState;
use std::time::{Duration, Instant};

pub const SEARCH_REFRESH_INTERVAL: Duration = Duration::from_millis(150);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchDirection {
    Previous,
    Next,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchRefresh {
    pub match_count: usize,
    pub active_match: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct SearchController {
    input: String,
    state: SearchState,
    active_match: Option<usize>,
    dirty: bool,
    last_refresh: Instant,
}

impl Default for SearchController {
    fn default() -> Self {
        Self {
            input: String::new(),
            state: SearchState::default(),
            active_match: None,
            dirty: false,
            last_refresh: Instant::now(),
        }
    }
}

impl SearchController {
    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn input_mut(&mut self) -> &mut String {
        &mut self.input
    }

    pub fn query(&self) -> &str {
        self.input.trim()
    }

    pub fn state(&self) -> &SearchState {
        &self.state
    }

    pub fn active_match(&self) -> Option<usize> {
        self.active_match
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn clear(&mut self, tree_len: usize) {
        self.input.clear();
        self.state.clear(tree_len);
        self.active_match = None;
        self.dirty = false;
    }

    pub fn mark_dirty(&mut self) {
        self.active_match = None;
        self.dirty = true;
        self.last_refresh = self
            .last_refresh
            .checked_sub(SEARCH_REFRESH_INTERVAL)
            .unwrap_or_else(Instant::now);
    }

    pub fn maybe_refresh_due(&self, scanning: bool) -> bool {
        self.dirty && (self.last_refresh.elapsed() >= SEARCH_REFRESH_INTERVAL || !scanning)
    }

    pub fn refresh(&mut self, tree: &mut TreeStore, focused_root: Option<NodeId>) -> SearchRefresh {
        let previous_match = self.active_match_id();
        self.dirty = false;
        self.last_refresh = Instant::now();
        let query = self.input.trim();
        self.state.rebuild(tree, focused_root, query);
        self.active_match = previous_match
            .and_then(|node_id| self.state.matches().iter().position(|id| *id == node_id));
        SearchRefresh {
            match_count: self.state.match_count(),
            active_match: self.active_match,
        }
    }

    pub fn can_navigate(&self) -> bool {
        !self.dirty && self.state.match_count() > 0
    }

    pub fn next_match(&mut self, direction: SearchDirection, tree: &TreeStore) -> Option<NodeId> {
        if self.query().is_empty() {
            return None;
        }

        let match_count = self.state.match_count();
        if match_count == 0 {
            self.active_match = None;
            return None;
        }

        let next_index = match direction {
            SearchDirection::Next => self
                .active_match
                .map(|index| (index + 1) % match_count)
                .unwrap_or(0),
            SearchDirection::Previous => self
                .active_match
                .map(|index| {
                    if index == 0 {
                        match_count - 1
                    } else {
                        index - 1
                    }
                })
                .unwrap_or(match_count - 1),
        };

        let node_id = self.state.matches().get(next_index).copied()?;
        if node_index(node_id) >= tree.len() {
            self.active_match = None;
            return None;
        }

        self.active_match = Some(next_index);
        Some(node_id)
    }

    pub fn ingest_new_nodes(&mut self, tree: &mut TreeStore, node_ids: &[NodeId]) -> usize {
        if self.query().is_empty() {
            return 0;
        }
        self.state.ingest_new_nodes(tree, node_ids)
    }

    pub fn active_match_id(&self) -> Option<NodeId> {
        self.active_match
            .and_then(|index| self.state.matches().get(index).copied())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{NodeKind, NodeRecord};

    fn tree_with_matches() -> TreeStore {
        let mut tree = TreeStore::new();
        tree.push_node(None, TreeStore::root_record("root".into()));
        tree.push_node(
            Some(0),
            NodeRecord {
                name: "match-dir".into(),
                kind: NodeKind::Dir,
                size: 10,
                modified_secs: None,
                scanned: true,
                error: None,
            },
        );
        tree.push_node(
            Some(1),
            NodeRecord {
                name: "match-file".into(),
                kind: NodeKind::File,
                size: 1,
                modified_secs: None,
                scanned: true,
                error: None,
            },
        );
        tree.push_node(
            Some(0),
            NodeRecord {
                name: "other".into(),
                kind: NodeKind::File,
                size: 1,
                modified_secs: None,
                scanned: true,
                error: None,
            },
        );
        tree
    }

    #[test]
    fn refresh_preserves_active_match_when_still_valid() {
        let mut tree = tree_with_matches();
        let mut search = SearchController::default();
        *search.input_mut() = "match".into();
        search.refresh(&mut tree, Some(0));

        assert_eq!(search.next_match(SearchDirection::Next, &tree), Some(1));
        let refresh = search.refresh(&mut tree, Some(0));

        assert_eq!(refresh.active_match, Some(0));
        assert_eq!(search.active_match_id(), Some(1));
    }

    #[test]
    fn refresh_resets_active_match_when_scope_changes() {
        let mut tree = tree_with_matches();
        let mut search = SearchController::default();
        *search.input_mut() = "match".into();
        search.refresh(&mut tree, Some(0));
        assert_eq!(search.next_match(SearchDirection::Next, &tree), Some(1));

        let refresh = search.refresh(&mut tree, Some(2));

        assert_eq!(refresh.active_match, None);
        assert_eq!(search.state().matches(), &[2]);
    }

    #[test]
    fn next_and_previous_cycle_through_matches() {
        let mut tree = tree_with_matches();
        let mut search = SearchController::default();
        *search.input_mut() = "match".into();
        search.refresh(&mut tree, Some(0));

        assert_eq!(search.next_match(SearchDirection::Next, &tree), Some(1));
        assert_eq!(search.next_match(SearchDirection::Next, &tree), Some(2));
        assert_eq!(search.next_match(SearchDirection::Next, &tree), Some(1));
        assert_eq!(search.next_match(SearchDirection::Previous, &tree), Some(2));
    }

    #[test]
    fn clear_resets_input_state_and_cursor() {
        let mut tree = tree_with_matches();
        let mut search = SearchController::default();
        *search.input_mut() = "match".into();
        search.refresh(&mut tree, Some(0));
        search.next_match(SearchDirection::Next, &tree);

        search.clear(tree.len());

        assert!(search.input().is_empty());
        assert!(search.state().matches().is_empty());
        assert_eq!(search.active_match(), None);
        assert!(!search.is_dirty());
    }
}
