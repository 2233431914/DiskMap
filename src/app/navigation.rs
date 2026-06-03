use crate::tree::{NodeId, TreeStore};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationOutcome {
    Noop,
    ResetCameraOnly,
    FocusChanged { refresh_search: bool },
}

#[derive(Debug, Default, Clone)]
pub struct NavigationState {
    focused_root: Option<NodeId>,
    selected_id: Option<NodeId>,
    back_history: Vec<NodeId>,
    forward_history: Vec<NodeId>,
    breadcrumb_cache: String,
}

impl NavigationState {
    pub fn focused_root(&self) -> Option<NodeId> {
        self.focused_root
    }

    pub fn selected_id(&self) -> Option<NodeId> {
        self.selected_id
    }

    pub fn set_selected_id(&mut self, selected_id: Option<NodeId>) {
        self.selected_id = selected_id;
    }

    pub fn breadcrumb(&self) -> &str {
        &self.breadcrumb_cache
    }

    pub fn can_go_back(&self) -> bool {
        !self.back_history.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward_history.is_empty()
    }

    #[cfg(test)]
    pub fn back_history(&self) -> &[NodeId] {
        &self.back_history
    }

    #[cfg(test)]
    pub fn forward_history(&self) -> &[NodeId] {
        &self.forward_history
    }

    pub fn parent_of_focused_root(&self, tree: &TreeStore) -> Option<NodeId> {
        self.focused_root
            .and_then(|node_id| tree.node(node_id).parent)
    }

    pub fn can_go_up(&self, tree: &TreeStore) -> bool {
        self.parent_of_focused_root(tree).is_some()
    }

    pub fn can_return_to_scan_root(&self, tree: &TreeStore) -> bool {
        matches!(
            (self.focused_root, tree.root),
            (Some(focused), Some(scan_root)) if focused != scan_root
        )
    }

    pub fn enter_root(&mut self, node_id: NodeId, push_history: bool) -> NavigationOutcome {
        if self.focused_root == Some(node_id) {
            return NavigationOutcome::ResetCameraOnly;
        }

        if push_history {
            if let Some(current) = self.focused_root {
                self.back_history.push(current);
            }
            self.forward_history.clear();
        }

        self.focused_root = Some(node_id);
        self.selected_id = Some(node_id);
        NavigationOutcome::FocusChanged {
            refresh_search: true,
        }
    }

    pub fn navigate_back(&mut self) -> NavigationOutcome {
        let Some(previous) = self.back_history.pop() else {
            return NavigationOutcome::Noop;
        };
        if let Some(current) = self.focused_root {
            self.forward_history.push(current);
        }
        self.focused_root = Some(previous);
        self.selected_id = Some(previous);
        NavigationOutcome::FocusChanged {
            refresh_search: true,
        }
    }

    pub fn navigate_forward(&mut self) -> NavigationOutcome {
        let Some(next) = self.forward_history.pop() else {
            return NavigationOutcome::Noop;
        };
        if let Some(current) = self.focused_root {
            self.back_history.push(current);
        }
        self.focused_root = Some(next);
        self.selected_id = Some(next);
        NavigationOutcome::FocusChanged {
            refresh_search: true,
        }
    }

    pub fn return_to_scan_root(&mut self, tree: &TreeStore) -> NavigationOutcome {
        let Some(scan_root) = tree.root else {
            return NavigationOutcome::Noop;
        };
        if self.focused_root == Some(scan_root) {
            return NavigationOutcome::Noop;
        }
        self.enter_root(scan_root, true)
    }

    pub fn focus_search_match(&mut self, tree: &TreeStore, node_id: NodeId) -> NavigationOutcome {
        if node_id >= tree.len() {
            return NavigationOutcome::Noop;
        }

        let focus_id = if tree.node(node_id).children.is_empty() {
            tree.node(node_id).parent.unwrap_or(node_id)
        } else {
            node_id
        };

        self.focused_root = Some(focus_id);
        self.selected_id = Some(node_id);
        NavigationOutcome::FocusChanged {
            refresh_search: false,
        }
    }

    pub fn clear_for_new_scan(&mut self) {
        self.focused_root = None;
        self.selected_id = None;
        self.back_history.clear();
        self.forward_history.clear();
        self.breadcrumb_cache.clear();
    }

    pub fn set_scan_root(&mut self, root_id: Option<NodeId>) {
        self.focused_root = root_id;
    }

    pub fn prune_invalid(&mut self, tree: &TreeStore) -> bool {
        if self
            .selected_id
            .is_some_and(|selected_id| !node_is_reachable(tree, selected_id))
        {
            self.selected_id = None;
        }

        let mut focus_changed = false;
        if self
            .focused_root
            .is_some_and(|root_id| !node_is_reachable(tree, root_id))
        {
            self.focused_root = tree.root;
            focus_changed = true;
        }
        self.back_history.retain(|id| node_is_reachable(tree, *id));
        self.forward_history
            .retain(|id| node_is_reachable(tree, *id));
        if focus_changed {
            self.rebuild_breadcrumb_cache(tree);
        }
        focus_changed
    }

    pub fn rebuild_breadcrumb_cache(&mut self, tree: &TreeStore) {
        let Some(root_id) = self.focused_root else {
            self.breadcrumb_cache.clear();
            return;
        };

        self.breadcrumb_cache = tree
            .ancestors(root_id)
            .into_iter()
            .map(|id| tree.node(id).name.clone())
            .collect::<Vec<_>>()
            .join(" / ");
    }

    #[cfg(test)]
    pub fn push_back_for_test(&mut self, node_id: NodeId) {
        self.back_history.push(node_id);
    }
}

fn node_is_reachable(tree: &TreeStore, node_id: NodeId) -> bool {
    node_id < tree.len()
        && tree
            .root
            .is_some_and(|root_id| tree.is_descendant_or_same(node_id, root_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{NodeKind, NodeRecord};

    fn tree_with_child_dir_and_file() -> TreeStore {
        let mut tree = TreeStore::new();
        tree.push_node(None, TreeStore::root_record("root".into()));
        tree.push_node(
            Some(0),
            NodeRecord {
                name: "child-dir".into(),
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
                name: "child-file".into(),
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
    fn enter_same_root_requests_camera_reset_without_history_mutation() {
        let mut state = NavigationState::default();
        state.set_scan_root(Some(0));

        let outcome = state.enter_root(0, true);

        assert_eq!(outcome, NavigationOutcome::ResetCameraOnly);
        assert!(!state.can_go_back());
        assert_eq!(state.focused_root(), Some(0));
    }

    #[test]
    fn root_navigation_pushes_previous_focus_to_back_history() {
        let tree = tree_with_child_dir_and_file();
        let mut state = NavigationState::default();
        state.set_scan_root(Some(0));
        assert!(matches!(
            state.enter_root(1, false),
            NavigationOutcome::FocusChanged { .. }
        ));

        let outcome = state.return_to_scan_root(&tree);

        assert_eq!(
            outcome,
            NavigationOutcome::FocusChanged {
                refresh_search: true
            }
        );
        assert_eq!(state.focused_root(), Some(0));
        assert_eq!(state.selected_id(), Some(0));
        assert!(state.can_go_back());
        assert!(!state.can_go_forward());
    }

    #[test]
    fn back_and_forward_preserve_existing_history_semantics() {
        let mut state = NavigationState::default();
        state.set_scan_root(Some(0));
        state.enter_root(1, true);

        assert_eq!(
            state.navigate_back(),
            NavigationOutcome::FocusChanged {
                refresh_search: true
            }
        );
        assert_eq!(state.focused_root(), Some(0));
        assert!(state.can_go_forward());

        assert_eq!(
            state.navigate_forward(),
            NavigationOutcome::FocusChanged {
                refresh_search: true
            }
        );
        assert_eq!(state.focused_root(), Some(1));
        assert!(state.can_go_back());
    }

    #[test]
    fn file_search_match_focuses_parent_and_selects_file() {
        let tree = tree_with_child_dir_and_file();
        let mut state = NavigationState::default();
        state.set_scan_root(Some(0));

        let outcome = state.focus_search_match(&tree, 2);

        assert_eq!(
            outcome,
            NavigationOutcome::FocusChanged {
                refresh_search: false
            }
        );
        assert_eq!(state.focused_root(), Some(1));
        assert_eq!(state.selected_id(), Some(2));
    }
}
