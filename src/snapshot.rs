use crate::tree::{NodeId, NodeKind, TreeStore};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotKind {
    File,
    Directory,
    Symlink,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotEntry {
    pub size: u64,
    pub kind: SnapshotKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanSnapshot {
    pub root_path: PathBuf,
    pub total_size: u64,
    pub entries: BTreeMap<String, SnapshotEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotChange {
    pub path: String,
    pub previous_size: u64,
    pub current_size: u64,
    pub delta: i128,
    pub kind: SnapshotKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotDiff {
    pub root_path: PathBuf,
    pub previous_total: u64,
    pub current_total: u64,
    pub added: Vec<SnapshotChange>,
    pub grown: Vec<SnapshotChange>,
    pub shrunk: Vec<SnapshotChange>,
    pub removed: Vec<SnapshotChange>,
}

impl SnapshotDiff {
    pub fn total_delta(&self) -> i128 {
        self.current_total as i128 - self.previous_total as i128
    }

    pub fn has_changes(&self) -> bool {
        !self.added.is_empty()
            || !self.grown.is_empty()
            || !self.shrunk.is_empty()
            || !self.removed.is_empty()
    }
}

pub fn capture_snapshot(tree: &mut TreeStore, root_id: NodeId) -> Option<ScanSnapshot> {
    if !tree.contains_id(root_id) {
        return None;
    }

    let root_path = tree.node_real_path(root_id)?;
    let total_size = tree.node(root_id).size;
    let mut entries = BTreeMap::new();
    collect_entries(tree, root_id, &mut entries);

    Some(ScanSnapshot {
        root_path,
        total_size,
        entries,
    })
}

pub fn compare_snapshots(
    previous: &ScanSnapshot,
    current: &ScanSnapshot,
    limit_per_group: usize,
) -> SnapshotDiff {
    let mut added = Vec::new();
    let mut grown = Vec::new();
    let mut shrunk = Vec::new();
    let mut removed = Vec::new();

    for (path, current_entry) in &current.entries {
        if path == &current.root_path.display().to_string() {
            continue;
        }
        match previous.entries.get(path) {
            Some(previous_entry) if previous_entry.size < current_entry.size => {
                grown.push(change(
                    path,
                    previous_entry,
                    current_entry,
                    current_entry.size as i128 - previous_entry.size as i128,
                ));
            }
            Some(previous_entry) if previous_entry.size > current_entry.size => {
                shrunk.push(change(
                    path,
                    previous_entry,
                    current_entry,
                    current_entry.size as i128 - previous_entry.size as i128,
                ));
            }
            Some(_) => {}
            None => {
                added.push(SnapshotChange {
                    path: path.clone(),
                    previous_size: 0,
                    current_size: current_entry.size,
                    delta: current_entry.size as i128,
                    kind: current_entry.kind,
                });
            }
        }
    }

    for (path, previous_entry) in &previous.entries {
        if path == &previous.root_path.display().to_string() {
            continue;
        }
        if current.entries.contains_key(path) {
            continue;
        }
        removed.push(SnapshotChange {
            path: path.clone(),
            previous_size: previous_entry.size,
            current_size: 0,
            delta: -(previous_entry.size as i128),
            kind: previous_entry.kind,
        });
    }

    sort_and_limit(&mut added, limit_per_group);
    sort_and_limit(&mut grown, limit_per_group);
    sort_and_limit(&mut shrunk, limit_per_group);
    sort_and_limit(&mut removed, limit_per_group);

    SnapshotDiff {
        root_path: current.root_path.clone(),
        previous_total: previous.total_size,
        current_total: current.total_size,
        added,
        grown,
        shrunk,
        removed,
    }
}

fn collect_entries(
    tree: &mut TreeStore,
    node_id: NodeId,
    entries: &mut BTreeMap<String, SnapshotEntry>,
) {
    if !tree.contains_id(node_id) {
        return;
    }

    tree.ensure_sorted_children(node_id);
    let path = tree.node_real_path(node_id);
    let (kind, size, children) = {
        let node = tree.node(node_id);
        (snapshot_kind(node.kind), node.size, node.children.clone())
    };

    if let (Some(path), Some(kind)) = (path, kind) {
        entries.insert(path.display().to_string(), SnapshotEntry { size, kind });
    }

    for child_id in children {
        collect_entries(tree, child_id, entries);
    }
}

fn snapshot_kind(kind: NodeKind) -> Option<SnapshotKind> {
    match kind {
        NodeKind::File => Some(SnapshotKind::File),
        NodeKind::Dir => Some(SnapshotKind::Directory),
        NodeKind::Symlink => Some(SnapshotKind::Symlink),
        NodeKind::Error => Some(SnapshotKind::Error),
        NodeKind::Aggregate => None,
    }
}

fn change(
    path: &str,
    previous: &SnapshotEntry,
    current: &SnapshotEntry,
    delta: i128,
) -> SnapshotChange {
    SnapshotChange {
        path: path.to_string(),
        previous_size: previous.size,
        current_size: current.size,
        delta,
        kind: current.kind,
    }
}

fn sort_and_limit(changes: &mut Vec<SnapshotChange>, limit: usize) {
    changes.sort_by(|left, right| {
        delta_magnitude(right.delta)
            .cmp(&delta_magnitude(left.delta))
            .then_with(|| left.path.cmp(&right.path))
    });
    changes.truncate(limit);
}

fn delta_magnitude(delta: i128) -> u128 {
    delta.unsigned_abs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree_with_files(root_path: &str, files: &[(&str, u64)]) -> TreeStore {
        let mut tree = TreeStore::new();
        let root = tree.add_node(None, "root".into(), NodeKind::Dir, 0);
        tree.set_root_path(root_path.into());
        for (name, size) in files {
            tree.add_node(Some(root), (*name).into(), NodeKind::File, *size);
            tree.apply_direct_size_delta(root, *size);
        }
        tree
    }

    #[test]
    fn capture_snapshot_collects_real_paths_and_skips_aggregates() {
        let mut tree = tree_with_files("/root", &[("a.txt", 10)]);
        let root = tree.root.expect("root");
        tree.add_node(Some(root), "Other Files (2)".into(), NodeKind::Aggregate, 8);

        let snapshot = capture_snapshot(&mut tree, root).expect("snapshot");

        assert_eq!(snapshot.root_path, PathBuf::from("/root"));
        assert!(snapshot.entries.contains_key("/root"));
        assert!(snapshot.entries.contains_key("/root/a.txt"));
        assert!(!snapshot.entries.contains_key(""));
    }

    #[test]
    fn compare_snapshots_groups_changes_by_size_direction() {
        let mut previous_tree = tree_with_files("/root", &[("a.txt", 10), ("old.txt", 9)]);
        let mut current_tree = tree_with_files("/root", &[("a.txt", 15), ("new.txt", 6)]);
        let previous = capture_snapshot(&mut previous_tree, 0).expect("previous");
        let current = capture_snapshot(&mut current_tree, 0).expect("current");

        let diff = compare_snapshots(&previous, &current, 8);

        assert_eq!(diff.total_delta(), 2);
        assert_eq!(diff.grown[0].path, "/root/a.txt");
        assert_eq!(diff.added[0].path, "/root/new.txt");
        assert_eq!(diff.removed[0].path, "/root/old.txt");
    }

    #[test]
    fn compare_snapshots_limits_each_group_by_largest_delta() {
        let mut previous_tree = tree_with_files("/root", &[("a.txt", 1), ("b.txt", 1)]);
        let mut current_tree = tree_with_files("/root", &[("a.txt", 9), ("b.txt", 4)]);
        let previous = capture_snapshot(&mut previous_tree, 0).expect("previous");
        let current = capture_snapshot(&mut current_tree, 0).expect("current");

        let diff = compare_snapshots(&previous, &current, 1);

        assert_eq!(diff.grown.len(), 1);
        assert_eq!(diff.grown[0].path, "/root/a.txt");
    }
}
