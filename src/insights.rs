use crate::tree::{NodeId, NodeKind, TreeStore};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const SECONDS_PER_DAY: u64 = 24 * 60 * 60;
pub const OLD_FILE_AGE_DAYS: u64 = 365;
pub const INSIGHT_REPORT_LIMIT: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AgeBucket {
    Last30Days,
    Days31To180,
    Days181To365,
    OlderThan365,
    Unknown,
}

impl AgeBucket {
    pub fn label(self) -> &'static str {
        match self {
            Self::Last30Days => "<=30d",
            Self::Days31To180 => "31-180d",
            Self::Days181To365 => "181-365d",
            Self::OlderThan365 => ">365d",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgeBucketSummary {
    pub bucket: AgeBucket,
    pub file_count: usize,
    pub total_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTypeSummary {
    pub category: String,
    pub extension: String,
    pub file_count: usize,
    pub total_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OldLargeFile {
    pub path: String,
    pub size: u64,
    pub age_days: u64,
    pub category: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsightReport {
    pub root_path: PathBuf,
    pub file_count: usize,
    pub known_mtime_count: usize,
    pub total_size: u64,
    pub type_summaries: Vec<FileTypeSummary>,
    pub age_buckets: Vec<AgeBucketSummary>,
    pub old_large_files: Vec<OldLargeFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct TypeKey {
    category: &'static str,
    extension: String,
}

#[derive(Debug, Clone, Default)]
struct TypeAccumulator {
    file_count: usize,
    total_size: u64,
}

pub fn analyze_insights(
    tree: &mut TreeStore,
    root_id: NodeId,
    now_secs: u64,
    limit: usize,
) -> Option<InsightReport> {
    if !tree.contains_id(root_id) {
        return None;
    }

    let root_path = tree.node_real_path(root_id)?;
    let mut report = InsightBuilder::new(root_path, now_secs, limit);
    collect_files(tree, root_id, &mut report);
    Some(report.finish())
}

struct InsightBuilder {
    root_path: PathBuf,
    now_secs: u64,
    limit: usize,
    file_count: usize,
    known_mtime_count: usize,
    total_size: u64,
    type_summaries: BTreeMap<TypeKey, TypeAccumulator>,
    age_buckets: BTreeMap<AgeBucket, TypeAccumulator>,
    old_large_files: Vec<OldLargeFile>,
}

impl InsightBuilder {
    fn new(root_path: PathBuf, now_secs: u64, limit: usize) -> Self {
        Self {
            root_path,
            now_secs,
            limit,
            file_count: 0,
            known_mtime_count: 0,
            total_size: 0,
            type_summaries: BTreeMap::new(),
            age_buckets: BTreeMap::new(),
            old_large_files: Vec::new(),
        }
    }

    fn observe_file(&mut self, path: String, name: &str, size: u64, modified_secs: Option<u64>) {
        self.file_count += 1;
        self.total_size = self.total_size.saturating_add(size);

        let extension = extension_key(name);
        let category = category_for_extension(&extension);
        let type_entry = self
            .type_summaries
            .entry(TypeKey {
                category,
                extension: extension.clone(),
            })
            .or_default();
        type_entry.file_count += 1;
        type_entry.total_size = type_entry.total_size.saturating_add(size);

        let bucket = age_bucket(modified_secs, self.now_secs);
        let age_entry = self.age_buckets.entry(bucket).or_default();
        age_entry.file_count += 1;
        age_entry.total_size = age_entry.total_size.saturating_add(size);

        let Some(modified_secs) = modified_secs else {
            return;
        };
        self.known_mtime_count += 1;
        let age_days = self
            .now_secs
            .saturating_sub(modified_secs)
            .checked_div(SECONDS_PER_DAY)
            .unwrap_or(0);
        if age_days >= OLD_FILE_AGE_DAYS {
            self.old_large_files.push(OldLargeFile {
                path,
                size,
                age_days,
                category: category.to_string(),
            });
        }
    }

    fn finish(mut self) -> InsightReport {
        let mut type_summaries = self
            .type_summaries
            .into_iter()
            .map(|(key, summary)| FileTypeSummary {
                category: key.category.to_string(),
                extension: key.extension,
                file_count: summary.file_count,
                total_size: summary.total_size,
            })
            .collect::<Vec<_>>();
        type_summaries.sort_by(|left, right| {
            right
                .total_size
                .cmp(&left.total_size)
                .then_with(|| right.file_count.cmp(&left.file_count))
                .then_with(|| left.category.cmp(&right.category))
                .then_with(|| left.extension.cmp(&right.extension))
        });
        type_summaries.truncate(self.limit);

        let age_buckets = [
            AgeBucket::Last30Days,
            AgeBucket::Days31To180,
            AgeBucket::Days181To365,
            AgeBucket::OlderThan365,
            AgeBucket::Unknown,
        ]
        .into_iter()
        .map(|bucket| {
            let summary = self.age_buckets.remove(&bucket).unwrap_or_default();
            AgeBucketSummary {
                bucket,
                file_count: summary.file_count,
                total_size: summary.total_size,
            }
        })
        .collect();

        self.old_large_files.sort_by(|left, right| {
            right
                .size
                .cmp(&left.size)
                .then_with(|| right.age_days.cmp(&left.age_days))
                .then_with(|| left.path.cmp(&right.path))
        });
        self.old_large_files.truncate(self.limit);

        InsightReport {
            root_path: self.root_path,
            file_count: self.file_count,
            known_mtime_count: self.known_mtime_count,
            total_size: self.total_size,
            type_summaries,
            age_buckets,
            old_large_files: self.old_large_files,
        }
    }
}

fn collect_files(tree: &mut TreeStore, node_id: NodeId, report: &mut InsightBuilder) {
    if !tree.contains_id(node_id) {
        return;
    }

    tree.ensure_sorted_children(node_id);
    let path = tree.node_real_path(node_id);
    let (name, kind, size, modified_secs, children) = {
        let node = tree.node(node_id);
        (
            node.name.clone(),
            node.kind,
            node.size,
            node.modified_secs,
            node.children.clone(),
        )
    };

    if matches!(kind, NodeKind::File) {
        if let Some(path) = path {
            report.observe_file(path.display().to_string(), &name, size, modified_secs);
        }
    }

    for child_id in children {
        collect_files(tree, child_id, report);
    }
}

fn age_bucket(modified_secs: Option<u64>, now_secs: u64) -> AgeBucket {
    let Some(modified_secs) = modified_secs else {
        return AgeBucket::Unknown;
    };
    let age_days = now_secs.saturating_sub(modified_secs) / SECONDS_PER_DAY;
    match age_days {
        0..=30 => AgeBucket::Last30Days,
        31..=180 => AgeBucket::Days31To180,
        181..=365 => AgeBucket::Days181To365,
        _ => AgeBucket::OlderThan365,
    }
}

fn extension_key(name: &str) -> String {
    std::path::Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_lowercase())
        .filter(|extension| !extension.is_empty())
        .unwrap_or_else(|| "(none)".to_string())
}

fn category_for_extension(extension: &str) -> &'static str {
    match extension {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "svg" => "Images",
        "mp4" | "mov" | "mkv" | "avi" | "webm" => "Video",
        "mp3" | "wav" | "aac" | "flac" | "m4a" => "Audio",
        "zip" | "tar" | "gz" | "7z" | "rar" | "dmg" | "pkg" => "Archives",
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "txt" | "md" => "Documents",
        "rs" | "js" | "ts" | "py" | "java" | "go" | "swift" | "kt" | "c" | "cpp" | "h" | "hpp" => {
            "Code"
        }
        "(none)" => "Other",
        _ => "Other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_700_000_000;

    fn sample_tree() -> TreeStore {
        let mut tree = TreeStore::new();
        let root = tree.add_node(None, "root".into(), NodeKind::Dir, 0);
        tree.set_root_path("/root".into());
        let media = tree.add_node(Some(root), "media".into(), NodeKind::Dir, 0);
        tree.add_node_with_modified(
            Some(media),
            "photo.JPG".into(),
            NodeKind::File,
            100,
            Some(NOW - 10 * SECONDS_PER_DAY),
        );
        tree.add_node_with_modified(
            Some(media),
            "movie.mp4".into(),
            NodeKind::File,
            300,
            Some(NOW - 400 * SECONDS_PER_DAY),
        );
        tree.add_node_with_modified(
            Some(root),
            "archive.zip".into(),
            NodeKind::File,
            200,
            Some(NOW - 200 * SECONDS_PER_DAY),
        );
        tree.add_node_with_modified(Some(root), "README".into(), NodeKind::File, 10, None);
        tree.add_node(Some(root), "Other Files (2)".into(), NodeKind::Aggregate, 8);
        tree.apply_direct_size_delta(root, 610);
        tree.apply_direct_size_delta(media, 400);
        tree
    }

    #[test]
    fn insight_report_summarizes_type_categories_by_size() {
        let mut tree = sample_tree();

        let report = analyze_insights(&mut tree, 0, NOW, 8).expect("report");

        assert_eq!(report.file_count, 4);
        assert_eq!(report.known_mtime_count, 3);
        assert_eq!(report.total_size, 610);
        assert_eq!(report.type_summaries[0].category, "Video");
        assert_eq!(report.type_summaries[0].extension, "mp4");
        assert_eq!(report.type_summaries[0].total_size, 300);
        assert!(report
            .type_summaries
            .iter()
            .any(|summary| summary.category == "Other" && summary.extension == "(none)"));
    }

    #[test]
    fn insight_report_counts_age_buckets_and_unknown_mtimes() {
        let mut tree = sample_tree();

        let report = analyze_insights(&mut tree, 0, NOW, 8).expect("report");

        let bucket = |bucket| {
            report
                .age_buckets
                .iter()
                .find(|summary| summary.bucket == bucket)
                .expect("age bucket")
        };
        assert_eq!(bucket(AgeBucket::Last30Days).file_count, 1);
        assert_eq!(bucket(AgeBucket::Days181To365).file_count, 1);
        assert_eq!(bucket(AgeBucket::OlderThan365).file_count, 1);
        assert_eq!(bucket(AgeBucket::Unknown).file_count, 1);
    }

    #[test]
    fn insight_report_lists_old_large_files_by_size() {
        let mut tree = sample_tree();

        let report = analyze_insights(&mut tree, 0, NOW, 8).expect("report");

        assert_eq!(report.old_large_files.len(), 1);
        assert_eq!(report.old_large_files[0].path, "/root/media/movie.mp4");
        assert_eq!(report.old_large_files[0].size, 300);
        assert_eq!(report.old_large_files[0].age_days, 400);
    }

    #[test]
    fn insight_report_rejects_invalid_or_virtual_roots() {
        let mut tree = sample_tree();
        let aggregate = tree
            .nodes
            .iter()
            .position(|node| matches!(node.kind, NodeKind::Aggregate))
            .map(crate::tree::node_id_from_index)
            .expect("aggregate");

        assert!(analyze_insights(&mut tree, NodeId::MAX, NOW, 8).is_none());
        assert!(analyze_insights(&mut tree, aggregate, NOW, 8).is_none());
    }
}
