use std::path::PathBuf;

pub type NodeId = usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    File,
    Dir,
    Symlink,
    Error,
}

#[derive(Debug, Clone)]
pub struct Node {
    pub parent: Option<NodeId>,
    pub name: String,
    pub kind: NodeKind,
    pub size: u64,
    pub children: Vec<NodeId>,
    pub scanned: bool,
    pub error: Option<String>,
    pub lower_name: String,
    sort_dirty: bool,
}

#[derive(Debug, Clone)]
pub struct NodeRecord {
    pub name: String,
    pub path: PathBuf,
    pub kind: NodeKind,
    pub size: u64,
    pub scanned: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TreeStore {
    pub nodes: Vec<Node>,
    pub root: Option<NodeId>,
    pub hidden_root: bool,
    root_path: PathBuf,
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
        }
    }

    pub fn clear(&mut self) {
        self.nodes.clear();
        self.root = None;
        self.hidden_root = false;
        self.root_path.clear();
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
        path: PathBuf,
        kind: NodeKind,
        size: u64,
    ) -> NodeId {
        let record = NodeRecord {
            name,
            path,
            kind,
            size,
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
        assert_eq!(id, self.nodes.len(), "incremental nodes must append in order");

        self.nodes.push(Node {
            parent,
            name: record.name.clone(),
            lower_name: String::new(),
            kind: record.kind,
            size: record.size,
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
            self.root_path = record.path;
        }
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

    pub fn node_path(&self, id: NodeId) -> PathBuf {
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
        path
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

    pub fn root_record(path: PathBuf, name: String) -> NodeRecord {
        NodeRecord {
            name,
            path,
            kind: NodeKind::Dir,
            size: 0,
            scanned: false,
            error: None,
        }
    }
}
