use crate::tree::{NodeId, NodeKind, TreeStore};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Csv,
    Json,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Json => "json",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Csv => "CSV",
            Self::Json => "JSON",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExportRecord {
    path: String,
    size: u64,
    kind: &'static str,
    error: Option<String>,
}

pub fn export_subtree(tree: &mut TreeStore, root_id: NodeId, format: ExportFormat) -> String {
    let mut records = Vec::new();
    collect_records(tree, root_id, &mut records);

    match format {
        ExportFormat::Csv => records_to_csv(&records),
        ExportFormat::Json => records_to_json(&records),
    }
}

fn collect_records(tree: &mut TreeStore, node_id: NodeId, records: &mut Vec<ExportRecord>) {
    if node_id >= tree.len() {
        return;
    }

    tree.ensure_sorted_children(node_id);
    let path = tree
        .node_real_path(node_id)
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    let (size, kind, error, children) = {
        let node = tree.node(node_id);
        (
            node.size,
            kind_label(node.kind),
            node.error.clone(),
            node.children.clone(),
        )
    };

    records.push(ExportRecord {
        path,
        size,
        kind,
        error,
    });

    for child_id in children {
        collect_records(tree, child_id, records);
    }
}

fn records_to_csv(records: &[ExportRecord]) -> String {
    let mut out = String::from("path,size,kind,error\n");
    for record in records {
        out.push_str(&csv_cell(&record.path));
        out.push(',');
        out.push_str(&record.size.to_string());
        out.push(',');
        out.push_str(record.kind);
        out.push(',');
        out.push_str(&csv_cell(record.error.as_deref().unwrap_or_default()));
        out.push('\n');
    }
    out
}

fn records_to_json(records: &[ExportRecord]) -> String {
    let mut out = String::from("[\n");
    for (index, record) in records.iter().enumerate() {
        if index > 0 {
            out.push_str(",\n");
        }
        out.push_str("  {\"path\":\"");
        out.push_str(&json_escape(&record.path));
        out.push_str("\",\"size\":");
        out.push_str(&record.size.to_string());
        out.push_str(",\"kind\":\"");
        out.push_str(record.kind);
        out.push_str("\",\"error\":");
        if let Some(error) = &record.error {
            out.push('"');
            out.push_str(&json_escape(error));
            out.push('"');
        } else {
            out.push_str("null");
        }
        out.push('}');
    }
    out.push_str("\n]\n");
    out
}

fn csv_cell(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn json_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out
}

fn kind_label(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::File => "File",
        NodeKind::Dir => "Directory",
        NodeKind::Symlink => "Symlink",
        NodeKind::Error => "Error",
        NodeKind::Aggregate => "Aggregate",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_tree() -> TreeStore {
        let mut tree = TreeStore::new();
        let root = tree.add_node(None, "root".into(), NodeKind::Dir, 0);
        tree.set_root_path("/root".into());
        let dir = tree.add_node(Some(root), "dir".into(), NodeKind::Dir, 0);
        tree.add_node(Some(dir), "a,file.txt".into(), NodeKind::File, 10);
        tree.add_node(Some(dir), "Other Files (2)".into(), NodeKind::Aggregate, 8);
        let err = tree.add_node(Some(root), "bad".into(), NodeKind::Error, 0);
        tree.node_mut(err).error = Some("permission \"denied\"".into());
        tree.apply_direct_size_delta(dir, 18);
        tree.apply_direct_size_delta(root, 18);
        tree
    }

    #[test]
    fn csv_export_includes_path_size_kind_and_error_fields() {
        let mut tree = sample_tree();

        let csv = export_subtree(&mut tree, 0, ExportFormat::Csv);

        assert!(csv.starts_with("path,size,kind,error\n"));
        assert!(csv.contains("/root,18,Directory,"));
        assert!(csv.contains("\"/root/dir/a,file.txt\",10,File,"));
        assert!(csv.contains(",8,Aggregate,"));
        assert!(csv.contains("/root/bad,0,Error,\"permission \"\"denied\"\"\""));
    }

    #[test]
    fn json_export_escapes_values_and_null_errors() {
        let mut tree = sample_tree();

        let json = export_subtree(&mut tree, 0, ExportFormat::Json);

        assert!(json.contains(
            "\"path\":\"/root/dir/a,file.txt\",\"size\":10,\"kind\":\"File\",\"error\":null"
        ));
        assert!(json.contains("\"path\":\"/root/bad\",\"size\":0,\"kind\":\"Error\",\"error\":\"permission \\\"denied\\\"\""));
    }

    #[test]
    fn subtree_export_starts_at_requested_node() {
        let mut tree = sample_tree();

        let csv = export_subtree(&mut tree, 1, ExportFormat::Csv);

        assert!(csv.contains("/root/dir,18,Directory,"));
        assert!(!csv.contains("/root,bad"));
        assert!(!csv.contains("/root,18,Directory,"));
    }
}
