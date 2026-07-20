use lru::LruCache;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

pub type NodeId = u32;

#[inline]
pub fn node_index(id: NodeId) -> usize {
    id as usize
}

#[inline]
pub fn node_id_from_index(index: usize) -> NodeId {
    NodeId::try_from(index).expect("node count exceeded u32::MAX")
}

#[inline]
pub fn node_id_in_len(id: NodeId, len: usize) -> bool {
    node_index(id) < len
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetachedSubtree {
    pub parent: Option<NodeId>,
    pub dirty_nodes: Vec<NodeId>,
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

    pub fn contains_id(&self, id: NodeId) -> bool {
        node_id_in_len(id, self.nodes.len())
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
        let id = node_id_from_index(self.nodes.len());
        self.insert_node(id, parent, record);
        id
    }

    pub fn insert_node(&mut self, id: NodeId, parent: Option<NodeId>, record: NodeRecord) {
        assert_eq!(
            node_index(id),
            self.nodes.len(),
            "incremental nodes must append in order"
        );

        self.nodes.push(Node {
            parent,
            name: record.name,
            kind: record.kind,
            size: record.size,
            modified_secs: record.modified_secs,
            children: Vec::new(),
            scanned: record.scanned,
            error: record.error,
            sort_dirty: true,
        });

        if let Some(parent_id) = parent {
            let parent_index = node_index(parent_id);
            self.nodes[parent_index].children.push(id);
            self.nodes[parent_index].sort_dirty = true;
        } else {
            self.root = Some(id);
            self.root_path.clear();
        }
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[node_index(id)]
    }

    pub fn node_mut(&mut self, id: NodeId) -> &mut Node {
        let index = node_index(id);
        &mut self.nodes[index]
    }

    pub fn node_name_matches_query(
        &self,
        id: NodeId,
        query_lower: &str,
        lowercase_scratch: &mut String,
    ) -> bool {
        name_matches_query(
            &self.nodes[node_index(id)].name,
            query_lower,
            lowercase_scratch,
        )
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
            current = self.nodes[node_index(id)].parent;
        }
    }

    pub fn apply_direct_size_delta(&mut self, node_id: NodeId, delta: u64) {
        let index = node_index(node_id);
        self.nodes[index].size += delta;
        self.nodes[index].sort_dirty = true;
    }

    pub fn mark_scanned(&mut self, node_id: NodeId) {
        self.nodes[node_index(node_id)].scanned = true;
    }

    pub fn detach_subtree(&mut self, node_id: NodeId) -> Option<DetachedSubtree> {
        let target_index = node_index(node_id);
        if target_index >= self.nodes.len() {
            return None;
        }

        if self.root == Some(node_id) {
            self.clear();
            return Some(DetachedSubtree {
                parent: None,
                dirty_nodes: Vec::new(),
            });
        }

        let parent_id = self.nodes[target_index].parent?;
        let parent_index = node_index(parent_id);
        let removed_size = self.nodes[target_index].size;
        if let Some(index) = self.nodes[parent_index]
            .children
            .iter()
            .position(|child_id| *child_id == node_id)
        {
            self.nodes[parent_index].children.remove(index);
        }
        self.nodes[target_index].parent = None;

        let mut dirty_nodes = Vec::new();
        let mut current = Some(parent_id);
        while let Some(id) = current {
            let index = node_index(id);
            self.nodes[index].size = self.nodes[index].size.saturating_sub(removed_size);
            self.nodes[index].sort_dirty = true;
            dirty_nodes.push(id);
            current = self.nodes[index].parent;
        }

        self.path_cache.clear();
        Some(DetachedSubtree {
            parent: Some(parent_id),
            dirty_nodes,
        })
    }

    pub fn repair_sorted_children(&mut self, dirty_nodes: &[NodeId]) {
        for &id in dirty_nodes {
            let index = node_index(id);
            if index >= self.nodes.len() || !self.nodes[index].sort_dirty {
                continue;
            }
            self.rebuild_sorted_children(id);
        }
    }

    pub fn ensure_sorted_children(&mut self, id: NodeId) {
        if self.nodes[node_index(id)].sort_dirty {
            self.rebuild_sorted_children(id);
        }
    }

    pub fn sorted_children(&self, id: NodeId) -> &[NodeId] {
        &self.nodes[node_index(id)].children
    }

    fn rebuild_sorted_children(&mut self, id: NodeId) {
        let index = node_index(id);
        let mut children = std::mem::take(&mut self.nodes[index].children);
        children.sort_by(|left, right| {
            let left_node = &self.nodes[node_index(*left)];
            let right_node = &self.nodes[node_index(*right)];
            let left_is_dir = matches!(left_node.kind, NodeKind::Dir);
            let right_is_dir = matches!(right_node.kind, NodeKind::Dir);
            right_is_dir
                .cmp(&left_is_dir)
                .then_with(|| right_node.size.cmp(&left_node.size))
                .then_with(|| left_node.name.cmp(&right_node.name))
        });
        self.nodes[index].children = children;
        self.nodes[index].sort_dirty = false;
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
            let node = &self.nodes[node_index(node_id)];
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

    pub fn find_node_by_real_path(&mut self, target: &Path) -> Option<NodeId> {
        let root_id = self.root?;
        let mut pending = vec![root_id];
        while let Some(node_id) = pending.pop() {
            if self.node_real_path(node_id).as_deref() == Some(target) {
                return Some(node_id);
            }
            pending.extend(self.node(node_id).children.iter().copied());
        }
        None
    }

    pub fn is_descendant_or_same(&self, node_id: NodeId, ancestor_id: NodeId) -> bool {
        let mut current = Some(node_id);
        while let Some(id) = current {
            if id == ancestor_id {
                return true;
            }
            current = self.nodes[node_index(id)].parent;
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

fn name_matches_query(name: &str, query_lower: &str, lowercase_scratch: &mut String) -> bool {
    if query_lower.is_empty() {
        return true;
    }
    if name.is_ascii() && query_lower.is_ascii() {
        return ascii_contains_case_insensitive(name.as_bytes(), query_lower.as_bytes());
    }

    lowercase_scratch.clear();
    lowercase_scratch.extend(name.chars().flat_map(char::to_lowercase));
    lowercase_scratch.contains(query_lower)
}

fn ascii_contains_case_insensitive(haystack: &[u8], needle_lower: &[u8]) -> bool {
    if needle_lower.is_empty() {
        return true;
    }
    if needle_lower.len() > haystack.len() {
        return false;
    }

    haystack.windows(needle_lower.len()).any(|window| {
        window
            .iter()
            .zip(needle_lower)
            .all(|(haystack_byte, needle_byte)| haystack_byte.to_ascii_lowercase() == *needle_byte)
    })
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
    fn find_node_by_real_path_returns_file_and_ignores_missing_paths() {
        let mut tree = TreeStore::new();
        let root = tree.add_node(None, "root".into(), NodeKind::Dir, 1);
        tree.set_root_path("/root".into());
        let file = tree.add_node(Some(root), "report.txt".into(), NodeKind::File, 1);

        assert_eq!(
            tree.find_node_by_real_path(Path::new("/root/report.txt")),
            Some(file)
        );
        assert_eq!(
            tree.find_node_by_real_path(Path::new("/root/missing.txt")),
            None
        );
    }

    #[test]
    fn node_name_query_matching_reuses_temporary_lowercase_buffer() {
        let mut tree = TreeStore::new();
        let root = tree.add_node(None, "root".into(), NodeKind::Dir, 0);
        let ascii = tree.add_node(Some(root), "Report.TXT".into(), NodeKind::File, 1);
        let cjk = tree.add_node(Some(root), "项目文件.txt".into(), NodeKind::File, 1);
        let mut scratch = String::new();

        assert!(tree.node_name_matches_query(ascii, "report", &mut scratch));
        assert!(tree.node_name_matches_query(cjk, "项目", &mut scratch));
        assert!(!tree.node_name_matches_query(cjk, "missing", &mut scratch));
    }

    #[test]
    fn detach_subtree_removes_child_and_updates_ancestor_sizes() {
        let mut tree = TreeStore::new();
        let root = tree.add_node(None, "root".into(), NodeKind::Dir, 30);
        tree.set_root_path("/root".into());
        let dir = tree.add_node(Some(root), "dir".into(), NodeKind::Dir, 10);
        let file = tree.add_node(Some(dir), "file.txt".into(), NodeKind::File, 10);
        let sibling = tree.add_node(Some(root), "sibling.txt".into(), NodeKind::File, 20);

        let detached = tree.detach_subtree(dir).expect("detached subtree");

        assert_eq!(detached.parent, Some(root));
        assert_eq!(detached.dirty_nodes, vec![root]);
        assert_eq!(tree.node(root).size, 20);
        assert_eq!(tree.node(dir).parent, None);
        assert!(tree.node(dir).children.contains(&file));
        assert_eq!(tree.node(file).parent, Some(dir));
        assert_eq!(tree.node(root).children, vec![sibling]);
    }

    #[test]
    fn detach_subtree_clears_tree_when_root_is_removed() {
        let mut tree = TreeStore::new();
        let root = tree.add_node(None, "root".into(), NodeKind::Dir, 1);
        tree.set_root_path("/root".into());
        tree.add_node(Some(root), "file.txt".into(), NodeKind::File, 1);

        let detached = tree.detach_subtree(root).expect("detached root");

        assert_eq!(detached.parent, None);
        assert!(tree.is_empty());
        assert_eq!(tree.root, None);
    }

    #[test]
    fn node_records_preserve_modified_time_through_insert() {
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

        assert_eq!(tree.node(file).modified_secs, Some(123));
    }
}
