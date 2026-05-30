use lru::LruCache;
use std::num::NonZeroUsize;
use std::path::PathBuf;

pub type NodeId = usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    File,
    Dir,
    Symlink,
    Error,
    Aggregate,
}

#[derive(Debug, Clone)]
pub struct Node {
    pub parent: Option<NodeId>,
    pub name: String,
    pub kind: NodeKind,
    pub size: u64,
    pub modified_secs: Option<u64>,
    pub children: Vec<NodeId>,
    pub scanned: bool,
    pub error: Option<String>,
    pub lower_name: String,
    sort_dirty: bool,
}

#[derive(Debug, Clone)]
pub struct NodeRecord {
    pub name: String,
    pub kind: NodeKind,
    pub size: u64,
    pub modified_secs: Option<u64>,
    pub scanned: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TreeStore {
    pub nodes: Vec<Node>,
    pub root: Option<NodeId>,
    pub hidden_root: bool,
    root_path: PathBuf,
    path_cache: LruCache<NodeId, PathBuf>,
}

impl Default for TreeStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TreeStore {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            root: None,
            hidden_root: false,
            root_path: PathBuf::new(),
            path_cache: LruCache::new(NonZeroUsize::new(256).expect("non-zero")),
        }
    }

    pub fn clear(&mut self) {
        self.nodes.clear();
        self.root = None;
        self.hidden_root = false;
        self.root_path.clear();
        self.path_cache.clear();
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn add_node(
        &mut self,
        parent: Option<NodeId>,
        name: String,
        kind: NodeKind,
        size: u64,
    ) -> NodeId {
        self.add_node_with_modified(parent, name, kind, size, None)
    }

    pub fn add_node_with_modified(
        &mut self,
        parent: Option<NodeId>,
        name: String,
        kind: NodeKind,
        size: u64,
        modified_secs: Option<u64>,
    ) -> NodeId {
        let record = NodeRecord {
            name,
            kind,
            size,
            modified_secs,
            scanned: false,
            error: None,
        };
        self.push_node(parent, record)
    }

    pub fn push_node(&mut self, parent: Option<NodeId>, record: NodeRecord) -> NodeId {
        let id = self.nodes.len();
        self.insert_node(id, parent, record);
        id
    }

    pub fn insert_node(&mut self, id: NodeId, parent: Option<NodeId>, record: NodeRecord) {
        assert_eq!(
            id,
            self.nodes.len(),
            "incremental nodes must append in order"
        );

        self.nodes.push(Node {
            parent,
            name: record.name.clone(),
            lower_name: String::new(),
            kind: record.kind,
            size: record.size,
            modified_secs: record.modified_secs,
            children: Vec::new(),
            scanned: record.scanned,
            error: record.error,
            sort_dirty: true,
        });

        if let Some(parent_id) = parent {
            self.nodes[parent_id].children.push(id);
            self.nodes[parent_id].sort_dirty = true;
        } else {
            self.root = Some(id);
            self.root_path.clear();
        }
        self.path_cache.pop(&id);
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id]
    }

    pub fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id]
    }

    pub fn node_name_matches_query(&mut self, id: NodeId, query_lower: &str) -> bool {
        let node = &mut self.nodes[id];
        if node.lower_name.is_empty() && !node.name.is_empty() {
            node.lower_name = node.name.to_lowercase();
        }
        node.lower_name.contains(query_lower)
    }

    pub fn ancestors(&self, mut id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        loop {
            out.push(id);
            let Some(parent) = self.node(id).parent else {
                break;
            };
            id = parent;
        }
        out.reverse();
        out
    }

    pub fn apply_size_delta(&mut self, node_id: NodeId, delta: u64) {
        let mut current = Some(node_id);
        while let Some(id) = current {
            self.apply_direct_size_delta(id, delta);
            current = self.nodes[id].parent;
        }
    }

    pub fn apply_direct_size_delta(&mut self, node_id: NodeId, delta: u64) {
        self.nodes[node_id].size += delta;
        self.nodes[node_id].sort_dirty = true;
    }

    pub fn mark_scanned(&mut self, node_id: NodeId) {
        self.nodes[node_id].scanned = true;
    }

    pub fn replace_children_from(
        &mut self,
        target_id: NodeId,
        source: &TreeStore,
    ) -> Option<Vec<NodeId>> {
        if target_id >= self.nodes.len() || !matches!(self.nodes[target_id].kind, NodeKind::Dir) {
            return None;
        }

        let source_root = source.root?;
        let source_root_node = source.node(source_root);
        let old_size = self.nodes[target_id].size;
        let new_size = source_root_node.size;

        self.nodes[target_id].children.clear();
        self.nodes[target_id].size = new_size;
        self.nodes[target_id].scanned = source_root_node.scanned;
        self.nodes[target_id].error = source_root_node.error.clone();
        self.nodes[target_id].sort_dirty = true;

        let mut appended_ids = Vec::new();
        for source_child in source_root_node.children.clone() {
            self.append_subtree_from(source, source_child, target_id, &mut appended_ids);
        }

        self.adjust_ancestor_sizes(target_id, old_size, new_size);
        self.path_cache.clear();
        Some(appended_ids)
    }

    pub fn repair_sorted_children(&mut self, dirty_nodes: &[NodeId]) {
        for &id in dirty_nodes {
            if id >= self.nodes.len() || !self.nodes[id].sort_dirty {
                continue;
            }
            self.rebuild_sorted_children(id);
        }
    }

    pub fn ensure_sorted_children(&mut self, id: NodeId) {
        if self.nodes[id].sort_dirty {
            self.rebuild_sorted_children(id);
        }
    }

    pub fn sorted_children(&self, id: NodeId) -> &[NodeId] {
        &self.nodes[id].children
    }

    fn rebuild_sorted_children(&mut self, id: NodeId) {
        let mut children = std::mem::take(&mut self.nodes[id].children);
        children.sort_by(|left, right| {
            let left_node = &self.nodes[*left];
            let right_node = &self.nodes[*right];
            let left_is_dir = matches!(left_node.kind, NodeKind::Dir);
            let right_is_dir = matches!(right_node.kind, NodeKind::Dir);
            right_is_dir
                .cmp(&left_is_dir)
                .then_with(|| right_node.size.cmp(&left_node.size))
                .then_with(|| left_node.name.cmp(&right_node.name))
        });
        self.nodes[id].children = children;
        self.nodes[id].sort_dirty = false;
    }

    fn append_subtree_from(
        &mut self,
        source: &TreeStore,
        source_id: NodeId,
        parent_id: NodeId,
        appended_ids: &mut Vec<NodeId>,
    ) {
        let source_node = source.node(source_id);
        let record = NodeRecord {
            name: source_node.name.clone(),
            kind: source_node.kind,
            size: source_node.size,
            modified_secs: source_node.modified_secs,
            scanned: source_node.scanned,
            error: source_node.error.clone(),
        };
        let new_id = self.push_node(Some(parent_id), record);
        appended_ids.push(new_id);

        for child_id in source_node.children.clone() {
            self.append_subtree_from(source, child_id, new_id, appended_ids);
        }
    }

    fn adjust_ancestor_sizes(&mut self, node_id: NodeId, old_size: u64, new_size: u64) {
        let mut current = self.nodes[node_id].parent;
        while let Some(id) = current {
            if new_size >= old_size {
                self.nodes[id].size = self.nodes[id].size.saturating_add(new_size - old_size);
            } else {
                self.nodes[id].size = self.nodes[id].size.saturating_sub(old_size - new_size);
            }
            self.nodes[id].sort_dirty = true;
            current = self.nodes[id].parent;
        }
    }

    pub fn set_root_path(&mut self, path: PathBuf) {
        self.root_path = path;
        self.path_cache.clear();
    }

    pub fn node_path(&mut self, id: NodeId) -> PathBuf {
        if let Some(cached) = self.path_cache.get(&id) {
            return cached.clone();
        }

        let Some(root_id) = self.root else {
            return PathBuf::new();
        };
        if id == root_id {
            return self.root_path.clone();
        }

        let mut components = Vec::new();
        let mut current = Some(id);
        while let Some(node_id) = current {
            if node_id == root_id {
                break;
            }
            let node = &self.nodes[node_id];
            components.push(node.name.as_str());
            current = node.parent;
        }

        let mut path = self.root_path.clone();
        for component in components.iter().rev() {
            path.push(component);
        }
        self.path_cache.put(id, path.clone());
        path
    }

    pub fn node_real_path(&mut self, id: NodeId) -> Option<PathBuf> {
        if matches!(self.node(id).kind, NodeKind::Aggregate) {
            None
        } else {
            Some(self.node_path(id))
        }
    }

    pub fn is_descendant_or_same(&self, node_id: NodeId, ancestor_id: NodeId) -> bool {
        let mut current = Some(node_id);
        while let Some(id) = current {
            if id == ancestor_id {
                return true;
            }
            current = self.nodes[id].parent;
        }
        false
    }

    pub fn root_record(name: String) -> NodeRecord {
        NodeRecord {
            name,
            kind: NodeKind::Dir,
            size: 0,
            modified_secs: None,
            scanned: false,
            error: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_path_rebuilds_from_root_path_and_uses_cache() {
        let mut tree = TreeStore::new();
        let root = tree.add_node(None, "root".into(), NodeKind::Dir, 0);
        tree.set_root_path("/root".into());
        let child = tree.add_node(Some(root), "child".into(), NodeKind::Dir, 0);
        let file = tree.add_node(Some(child), "file.txt".into(), NodeKind::File, 1);

        let first = tree.node_path(file);
        let second = tree.node_path(file);

        assert_eq!(first, PathBuf::from("/root/child/file.txt"));
        assert_eq!(second, first);
    }

    #[test]
    fn aggregate_nodes_do_not_have_real_paths() {
        let mut tree = TreeStore::new();
        let root = tree.add_node(None, "root".into(), NodeKind::Dir, 0);
        tree.set_root_path("/root".into());
        let aggregate = tree.add_node(
            Some(root),
            "Other Files (2)".into(),
            NodeKind::Aggregate,
            12,
        );

        assert!(tree.node_real_path(aggregate).is_none());
    }

    #[test]
    fn replace_children_from_keeps_target_id_and_updates_ancestor_sizes() {
        let mut tree = TreeStore::new();
        let root = tree.add_node(None, "root".into(), NodeKind::Dir, 30);
        tree.set_root_path("/root".into());
        let target = tree.add_node(Some(root), "target".into(), NodeKind::Dir, 10);
        let old_file = tree.add_node(Some(target), "old.txt".into(), NodeKind::File, 10);
        let sibling = tree.add_node(Some(root), "sibling.txt".into(), NodeKind::File, 20);

        let mut replacement = TreeStore::new();
        let replacement_root = replacement.add_node(None, "target".into(), NodeKind::Dir, 5);
        replacement.set_root_path("/replacement".into());
        replacement.add_node(Some(replacement_root), "new.txt".into(), NodeKind::File, 5);

        let appended = tree
            .replace_children_from(target, &replacement)
            .expect("replacement");

        assert_eq!(tree.root, Some(root));
        assert_eq!(tree.node(root).size, 25);
        assert_eq!(tree.node(target).size, 5);
        assert_eq!(tree.node(target).children, appended);
        assert_eq!(tree.node(appended[0]).name, "new.txt");
        assert_eq!(tree.node(appended[0]).parent, Some(target));
        assert_eq!(tree.node(old_file).parent, Some(target));
        assert!(!tree.node(target).children.contains(&old_file));
        assert!(tree.node(root).children.contains(&sibling));
    }

    #[test]
    fn replace_children_from_rejects_non_directory_targets() {
        let mut tree = TreeStore::new();
        let root = tree.add_node(None, "root".into(), NodeKind::Dir, 1);
        let file = tree.add_node(Some(root), "file.txt".into(), NodeKind::File, 1);
        let mut replacement = TreeStore::new();
        replacement.add_node(None, "file.txt".into(), NodeKind::Dir, 0);

        assert!(tree.replace_children_from(file, &replacement).is_none());
    }

    #[test]
    fn node_records_preserve_modified_time_through_insert_and_replacement() {
        let mut tree = TreeStore::new();
        let root = tree.add_node(None, "root".into(), NodeKind::Dir, 0);
        let file = tree.add_node_with_modified(
            Some(root),
            "file.txt".into(),
            NodeKind::File,
            4,
            Some(123),
        );

        assert_eq!(tree.node(file).modified_secs, Some(123));

        let mut replacement = TreeStore::new();
        let replacement_root = replacement.add_node(None, "root".into(), NodeKind::Dir, 0);
        replacement.add_node_with_modified(
            Some(replacement_root),
            "new.txt".into(),
            NodeKind::File,
            5,
            Some(456),
        );

        let appended = tree
            .replace_children_from(root, &replacement)
            .expect("replacement");

        assert_eq!(tree.node(appended[0]).modified_secs, Some(456));
    }
}
