use super::{
    cleanup_target_inaccessible_status, cleanup_target_missing_status, protected_path_status,
    DiskMapApp, StatusLevel, StatusSource,
};
use crate::cleanup::{
    normalize_cleanup_path, parse_protected_paths, validate_cleanup_target, CleanupTargetStatus,
};
#[cfg(test)]
use crate::cleanup::{protected_path_reason_with_deny_list, CleanupCandidate, QueueAddResult};
use crate::format::format_bytes;
use crate::platform::move_to_trash;
use crate::tree::NodeId;
use std::path::{Path, PathBuf};

impl DiskMapApp {
    pub(super) fn clear_trash_confirmation(&mut self) {
        self.trash_confirm_target_id = None;
        self.trash_confirm_path = None;
    }

    #[cfg(test)]
    pub(super) fn queue_cleanup_candidate(&mut self, node_id: NodeId) {
        let Some(raw_path) = self.tree.node_real_path(node_id) else {
            self.set_status(
                StatusSource::Cleanup,
                StatusLevel::Warning,
                "Cleanup queue unavailable for virtual nodes",
            );
            self.pending_repaint = true;
            return;
        };
        let path = normalize_cleanup_path(&raw_path);

        if let Some(reason) = self.protected_path_reason(&path) {
            self.set_status(
                StatusSource::Cleanup,
                StatusLevel::Error,
                protected_path_status(reason, &path),
            );
            self.pending_repaint = true;
            return;
        }

        let node = self.tree.node(node_id);
        let candidate = CleanupCandidate {
            node_id,
            name: node.name.clone(),
            path: path.clone(),
            size: node.size,
            item_count: self.cleanup_item_count(node_id),
            kind: node.kind,
        };
        let status = match self.cleanup_queue.add(candidate) {
            QueueAddResult::Added => format!("Queued cleanup candidate: {}", path.display()),
            QueueAddResult::AlreadyQueued => {
                format!("Cleanup candidate already queued: {}", path.display())
            }
        };
        self.set_status(StatusSource::Cleanup, StatusLevel::Info, status);
        self.pending_repaint = true;
    }

    pub(super) fn move_node_to_trash(&mut self, node_id: NodeId) {
        let Some(path) = self.tree.node_real_path(node_id) else {
            self.set_status(
                StatusSource::Cleanup,
                StatusLevel::Warning,
                "Move to Trash unavailable for virtual nodes",
            );
            self.pending_repaint = true;
            return;
        };

        self.arm_or_confirm_trash(node_id, normalize_cleanup_path(&path));
    }

    #[cfg(test)]
    pub(super) fn arm_or_confirm_queued_trash(&mut self, node_id: NodeId) {
        let Some(candidate) = self.cleanup_queue.get(node_id).cloned() else {
            self.set_status(
                StatusSource::Cleanup,
                StatusLevel::Warning,
                "Move to Trash unavailable: candidate is not queued",
            );
            self.pending_repaint = true;
            return;
        };

        self.arm_or_confirm_trash(node_id, normalize_cleanup_path(&candidate.path));
    }

    fn arm_or_confirm_trash(&mut self, node_id: NodeId, path: PathBuf) {
        match self.cleanup_target_status(&path) {
            CleanupTargetStatus::Ready => {}
            CleanupTargetStatus::Protected(reason) => {
                self.trash_confirm_target_id = None;
                self.trash_confirm_path = None;
                self.set_status(
                    StatusSource::Cleanup,
                    StatusLevel::Error,
                    protected_path_status(reason, &path),
                );
                self.pending_repaint = true;
                return;
            }
            CleanupTargetStatus::Missing => {
                self.trash_confirm_target_id = None;
                self.trash_confirm_path = None;
                self.cleanup_queue.remove(node_id);
                self.set_status(
                    StatusSource::Cleanup,
                    StatusLevel::Error,
                    cleanup_target_missing_status(&path),
                );
                self.pending_repaint = true;
                return;
            }
            CleanupTargetStatus::Inaccessible(error) => {
                self.trash_confirm_target_id = None;
                self.trash_confirm_path = None;
                self.set_status(
                    StatusSource::Cleanup,
                    StatusLevel::Error,
                    cleanup_target_inaccessible_status(&path, &error),
                );
                self.pending_repaint = true;
                return;
            }
        }

        if self.trash_confirm_target_id != Some(node_id)
            || self.trash_confirm_path.as_ref() != Some(&path)
        {
            let node = self.tree.node(node_id);
            let size = node.size;
            let kind = super::describe_node_kind(node.kind, !node.children.is_empty());
            let item_count = self.cleanup_item_count(node_id);
            self.trash_confirm_target_id = Some(node_id);
            self.trash_confirm_path = Some(path.clone());
            self.set_status(
                StatusSource::Cleanup,
                StatusLevel::Confirmation,
                format!(
                    "Confirm Move to Trash: {} · {} · {} · {}",
                    path.display(),
                    kind,
                    format_bytes(size),
                    super::pluralize(item_count as u64, "item", "items")
                ),
            );
            self.pending_repaint = true;
            return;
        }

        self.move_path_to_trash(node_id, path);
    }

    fn move_path_to_trash(&mut self, node_id: NodeId, path: PathBuf) {
        if self.trash_confirm_target_id != Some(node_id)
            || self.trash_confirm_path.as_ref() != Some(&path)
        {
            self.arm_or_confirm_trash(node_id, path);
            return;
        }

        match self.cleanup_target_status(&path) {
            CleanupTargetStatus::Ready => {}
            CleanupTargetStatus::Protected(reason) => {
                self.trash_confirm_target_id = None;
                self.trash_confirm_path = None;
                self.set_status(
                    StatusSource::Cleanup,
                    StatusLevel::Error,
                    protected_path_status(reason, &path),
                );
                self.pending_repaint = true;
                return;
            }
            CleanupTargetStatus::Missing => {
                self.trash_confirm_target_id = None;
                self.trash_confirm_path = None;
                self.cleanup_queue.remove(node_id);
                self.set_status(
                    StatusSource::Cleanup,
                    StatusLevel::Error,
                    cleanup_target_missing_status(&path),
                );
                self.pending_repaint = true;
                return;
            }
            CleanupTargetStatus::Inaccessible(error) => {
                self.trash_confirm_target_id = None;
                self.trash_confirm_path = None;
                self.set_status(
                    StatusSource::Cleanup,
                    StatusLevel::Error,
                    cleanup_target_inaccessible_status(&path, &error),
                );
                self.pending_repaint = true;
                return;
            }
        }

        if self.trash_confirm_target_id == Some(node_id) {
            self.trash_confirm_target_id = None;
            self.trash_confirm_path = None;
        }

        match move_to_trash(&path) {
            Ok(()) => {
                self.cleanup_queue.remove(node_id);
                let view_updated = self.remove_deleted_node_from_view(node_id);
                let status = if view_updated {
                    format!("Moved to Trash: {}", path.display())
                } else {
                    format!("Moved to Trash: {}. Rescan to refresh.", path.display())
                };
                self.set_status(StatusSource::Cleanup, StatusLevel::Success, status);
            }
            Err(error) => {
                self.set_status(
                    StatusSource::Cleanup,
                    StatusLevel::Error,
                    format!("Move to Trash failed: {error}"),
                );
            }
        }
        self.pending_repaint = true;
    }

    fn remove_deleted_node_from_view(&mut self, node_id: NodeId) -> bool {
        if !self.tree.contains_id(node_id) {
            return false;
        }

        let focused_removed = self
            .navigation
            .focused_root()
            .is_some_and(|id| self.tree.is_descendant_or_same(id, node_id));
        let selected_removed = self
            .navigation
            .selected_id()
            .is_some_and(|id| self.tree.is_descendant_or_same(id, node_id));

        let Some(detached) = self.tree.detach_subtree(node_id) else {
            return false;
        };

        if let Some(parent_id) = detached.parent {
            self.tree.repair_sorted_children(&detached.dirty_nodes);
            if focused_removed {
                self.navigation.enter_root(parent_id, false);
            } else if selected_removed {
                self.navigation.set_selected_id(Some(parent_id));
            }
            self.navigation.prune_invalid(&self.tree);
            self.navigation.rebuild_breadcrumb_cache(&self.tree);
            self.refresh_search_matches();
        } else {
            self.stop_watching();
            self.navigation.clear_for_new_scan();
            self.search.clear(0);
        }

        self.hovered_id = None;
        self.context_menu_target_id = None;
        self.hovered_visual_kind = None;
        self.trash_confirm_target_id = None;
        self.trash_confirm_path = None;
        self.treemap.clear();
        true
    }

    fn cleanup_item_count(&self, node_id: NodeId) -> usize {
        if !self.tree.contains_id(node_id) {
            return 0;
        }

        let mut count = 1usize;
        let mut stack = self.tree.node(node_id).children.clone();
        while let Some(id) = stack.pop() {
            if !self.tree.contains_id(id) {
                continue;
            }
            count += 1;
            stack.extend(self.tree.node(id).children.iter().copied());
        }
        count
    }

    #[cfg(test)]
    fn protected_path_reason(&self, path: &Path) -> Option<crate::cleanup::ProtectedPathReason> {
        let user_deny_list = parse_protected_paths(&self.protected_paths_input);
        protected_path_reason_with_deny_list(path, &user_deny_list)
    }

    fn cleanup_target_status(&self, path: &Path) -> CleanupTargetStatus {
        let user_deny_list = parse_protected_paths(&self.protected_paths_input);
        validate_cleanup_target(path, &user_deny_list)
    }
}
