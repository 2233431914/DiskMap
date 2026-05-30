use crate::tree::{NodeId, NodeKind, TreeStore};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateCandidate {
    pub name: String,
    pub size: u64,
    pub paths: Vec<String>,
    pub reclaimable_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateReport {
    pub root_path: PathBuf,
    pub group_count: usize,
    pub file_count: usize,
    pub total_reclaimable_bytes: u64,
    pub candidates: Vec<DuplicateCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CandidateKey {
    lower_name: String,
    size: u64,
}

#[derive(Debug, Clone)]
struct CandidateFile {
    name: String,
    path: String,
}

pub fn find_duplicate_candidates(
    tree: &mut TreeStore,
    root_id: NodeId,
    limit: usize,
) -> Option<DuplicateReport> {
    if root_id >= tree.len() {
        return None;
    }

    let root_path = tree.node_real_path(root_id)?;
    let mut groups = BTreeMap::<CandidateKey, Vec<CandidateFile>>::new();
    collect_files(tree, root_id, &mut groups);

    let mut candidates = groups
        .into_iter()
        .filter_map(|(key, mut files)| {
            if files.len() < 2 {
                return None;
            }
            files.sort_by(|left, right| left.path.cmp(&right.path));
            let reclaimable_bytes = key
                .size
                .saturating_mul(files.len().saturating_sub(1) as u64);
            Some(DuplicateCandidate {
                name: files
                    .first()
                    .map(|file| file.name.clone())
                    .unwrap_or_else(|| key.lower_name.clone()),
                size: key.size,
                paths: files.into_iter().map(|file| file.path).collect(),
                reclaimable_bytes,
            })
        })
        .collect::<Vec<_>>();

    candidates.sort_by(|left, right| {
        right
            .reclaimable_bytes
            .cmp(&left.reclaimable_bytes)
            .then_with(|| right.size.cmp(&left.size))
            .then_with(|| left.name.cmp(&right.name))
    });

    let group_count = candidates.len();
    let file_count = candidates
        .iter()
        .map(|candidate| candidate.paths.len())
        .sum();
    let total_reclaimable_bytes = candidates
        .iter()
        .map(|candidate| candidate.reclaimable_bytes)
        .sum();
    candidates.truncate(limit);

    Some(DuplicateReport {
        root_path,
        group_count,
        file_count,
        total_reclaimable_bytes,
        candidates,
    })
}

fn collect_files(
    tree: &mut TreeStore,
    node_id: NodeId,
    groups: &mut BTreeMap<CandidateKey, Vec<CandidateFile>>,
) {
    if node_id >= tree.len() {
        return;
    }

    tree.ensure_sorted_children(node_id);
    let (name, kind, size, children) = {
        let node = tree.node(node_id);
        (
            node.name.clone(),
            node.kind,
            node.size,
            node.children.clone(),
        )
    };

    if matches!(kind, NodeKind::File) && size > 0 {
        if let Some(path) = tree.node_real_path(node_id) {
            groups
                .entry(CandidateKey {
                    lower_name: name.to_lowercase(),
                    size,
                })
                .or_default()
                .push(CandidateFile {
                    name,
                    path: path.display().to_string(),
                });
        }
    }

    for child_id in children {
        collect_files(tree, child_id, groups);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_tree() -> TreeStore {
        let mut tree = TreeStore::new();
        let root = tree.add_node(None, "root".into(), NodeKind::Dir, 0);
        tree.set_root_path("/root".into());
        let a = tree.add_node(Some(root), "a".into(), NodeKind::Dir, 0);
        let b = tree.add_node(Some(root), "b".into(), NodeKind::Dir, 0);
        tree.add_node(Some(a), "same.bin".into(), NodeKind::File, 10);
        tree.add_node(Some(b), "SAME.bin".into(), NodeKind::File, 10);
        tree.add_node(Some(b), "same.bin".into(), NodeKind::File, 11);
        tree.add_node(Some(root), "empty.txt".into(), NodeKind::File, 0);
        tree.add_node(Some(root), "Other Files (2)".into(), NodeKind::Aggregate, 8);
        tree.apply_direct_size_delta(root, 31);
        tree.apply_direct_size_delta(a, 10);
        tree.apply_direct_size_delta(b, 21);
        tree
    }

    #[test]
    fn duplicate_report_groups_same_name_and_size_candidates() {
        let mut tree = sample_tree();

        let report = find_duplicate_candidates(&mut tree, 0, 8).expect("report");

        assert_eq!(report.root_path, PathBuf::from("/root"));
        assert_eq!(report.group_count, 1);
        assert_eq!(report.file_count, 2);
        assert_eq!(report.total_reclaimable_bytes, 10);
        assert_eq!(report.candidates[0].size, 10);
        assert_eq!(report.candidates[0].paths.len(), 2);
    }

    #[test]
    fn duplicate_report_limits_by_reclaimable_bytes() {
        let mut tree = TreeStore::new();
        let root = tree.add_node(None, "root".into(), NodeKind::Dir, 0);
        tree.set_root_path("/root".into());
        for dir_name in ["a", "b", "c"] {
            let dir = tree.add_node(Some(root), dir_name.into(), NodeKind::Dir, 0);
            tree.add_node(Some(dir), "large.bin".into(), NodeKind::File, 100);
            tree.add_node(Some(dir), "small.bin".into(), NodeKind::File, 5);
        }

        let report = find_duplicate_candidates(&mut tree, root, 1).expect("report");

        assert_eq!(report.group_count, 2);
        assert_eq!(report.candidates.len(), 1);
        assert_eq!(report.candidates[0].name, "large.bin");
        assert_eq!(report.candidates[0].reclaimable_bytes, 200);
    }

    #[test]
    fn duplicate_report_returns_none_for_invalid_or_virtual_roots() {
        let mut tree = sample_tree();
        let root = tree.root.expect("root");
        let aggregate = tree.add_node(Some(root), "Other Files (3)".into(), NodeKind::Aggregate, 3);

        assert!(find_duplicate_candidates(&mut tree, usize::MAX, 8).is_none());
        assert!(find_duplicate_candidates(&mut tree, aggregate, 8).is_none());
    }
}
