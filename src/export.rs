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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusedReportMetadata {
    pub generated_at_unix_secs: u64,
    pub scan_root_path: String,
    pub focused_path: String,
    pub size_basis: &'static str,
    pub max_depth: usize,
    pub search_query: String,
    pub search_filter_enabled: bool,
    pub color_mode: &'static str,
    pub include_hidden: bool,
    pub follow_symlinks: bool,
    pub stay_on_filesystem: bool,
    pub sqlite_cache_enabled: bool,
    pub realtime_watch_enabled: bool,
    pub exclude_patterns: Vec<String>,
}

pub fn export_subtree(tree: &mut TreeStore, root_id: NodeId, format: ExportFormat) -> String {
    let mut records = Vec::new();
    collect_records(tree, root_id, &mut records);

    match format {
        ExportFormat::Csv => records_to_csv(&records),
        ExportFormat::Json => records_to_json(&records),
    }
}

pub fn export_focused_report(
    tree: &mut TreeStore,
    root_id: NodeId,
    metadata: &FocusedReportMetadata,
) -> String {
    let mut records = Vec::new();
    collect_records(tree, root_id, &mut records);
    report_to_json(metadata, &records)
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

fn report_to_json(metadata: &FocusedReportMetadata, records: &[ExportRecord]) -> String {
    let mut out = String::from("{\n  \"metadata\":{");
    out.push_str("\"generated_at_unix_secs\":");
    out.push_str(&metadata.generated_at_unix_secs.to_string());
    out.push_str(",\"scan_root_path\":\"");
    out.push_str(&json_escape(&metadata.scan_root_path));
    out.push_str("\",\"focused_path\":\"");
    out.push_str(&json_escape(&metadata.focused_path));
    out.push_str("\",\"size_basis\":\"");
    out.push_str(&json_escape(metadata.size_basis));
    out.push_str("\",\"max_depth\":");
    out.push_str(&metadata.max_depth.to_string());
    out.push_str(",\"search_query\":\"");
    out.push_str(&json_escape(&metadata.search_query));
    out.push_str("\",\"search_filter_enabled\":");
    out.push_str(bool_json(metadata.search_filter_enabled));
    out.push_str(",\"color_mode\":\"");
    out.push_str(&json_escape(metadata.color_mode));
    out.push_str("\",\"scan_options\":{");
    out.push_str("\"include_hidden\":");
    out.push_str(bool_json(metadata.include_hidden));
    out.push_str(",\"follow_symlinks\":");
    out.push_str(bool_json(metadata.follow_symlinks));
    out.push_str(",\"stay_on_filesystem\":");
    out.push_str(bool_json(metadata.stay_on_filesystem));
    out.push_str(",\"sqlite_cache_enabled\":");
    out.push_str(bool_json(metadata.sqlite_cache_enabled));
    out.push_str(",\"realtime_watch_enabled\":");
    out.push_str(bool_json(metadata.realtime_watch_enabled));
    out.push_str(",\"exclude_patterns\":[");
    for (index, pattern) in metadata.exclude_patterns.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push('"');
        out.push_str(&json_escape(pattern));
        out.push('"');
    }
    out.push_str("]}}");
    out.push_str(",\n  \"entries\":");
    out.push_str(&records_to_json(records));
    out.push_str("}\n");
    out
}

fn bool_json(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
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

    #[test]
    fn focused_report_includes_metadata_and_entries() {
        let mut tree = sample_tree();
        let metadata = FocusedReportMetadata {
            generated_at_unix_secs: 123,
            scan_root_path: "/root".into(),
            focused_path: "/root/dir".into(),
            size_basis: "Allocated size",
            max_depth: 3,
            search_query: "file".into(),
            search_filter_enabled: true,
            color_mode: "extension",
            include_hidden: false,
            follow_symlinks: true,
            stay_on_filesystem: true,
            sqlite_cache_enabled: false,
            realtime_watch_enabled: true,
            exclude_patterns: vec![".git".into(), "target".into()],
        };

        let report = export_focused_report(&mut tree, 1, &metadata);

        assert!(report.contains("\"generated_at_unix_secs\":123"));
        assert!(report.contains("\"focused_path\":\"/root/dir\""));
        assert!(report.contains("\"search_filter_enabled\":true"));
        assert!(report.contains("\"exclude_patterns\":[\".git\",\"target\"]"));
        assert!(report.contains(
            "\"path\":\"/root/dir/a,file.txt\",\"size\":10,\"kind\":\"File\",\"error\":null"
        ));
        assert!(!report.contains("\"path\":\"/root/bad\""));
    }
}
