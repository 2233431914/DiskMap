use crate::scanner::{self, PerfStats, ProgressSnapshot, ScanHandle, ScanMessage, ScanOptions};
use crate::tree::{NodeKind, NodeRecord};
use crossbeam_channel::Sender;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ProgressSummary {
    pub files_scanned: u64,
    pub dirs_scanned: u64,
    pub bytes_seen: u64,
    pub current_path: PathBuf,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ScanIssueSummary {
    pub error_entries: u64,
    pub permission_errors: u64,
    pub skipped_paths: u64,
    pub symlinks: u64,
}

impl ScanIssueSummary {
    pub fn has_findings(&self) -> bool {
        self.error_entries > 0
            || self.permission_errors > 0
            || self.skipped_paths > 0
            || self.symlinks > 0
    }

    pub fn issue_count(&self) -> u64 {
        self.error_entries
    }
}

#[derive(Debug, Default)]
pub struct ScanSession {
    active_scan_id: u64,
    scan_counter: u64,
    handle: Option<ScanHandle>,
    scanning: bool,
    progress: Option<ProgressSummary>,
    issue_summary: ScanIssueSummary,
    perf_stats: PerfStats,
}

impl ScanSession {
    pub fn start(&mut self, path: PathBuf, options: ScanOptions, tx: Sender<ScanMessage>) -> u64 {
        let scan_id = self.begin_scan();
        self.handle = Some(scanner::start_scan(path, scan_id, options, tx));
        scan_id
    }

    pub fn cancel(&self) -> bool {
        let Some(handle) = &self.handle else {
            return false;
        };
        handle.cancel();
        true
    }

    pub fn accepts(&self, message: &ScanMessage) -> bool {
        scan_id_for_message(message) == self.active_scan_id
    }

    pub fn mark_started(&mut self) {
        self.scanning = true;
    }

    pub fn mark_finished(&mut self, perf_stats: PerfStats) {
        self.finish_with_perf_stats(perf_stats);
    }

    pub fn mark_cancelled(&mut self, perf_stats: PerfStats) {
        self.finish_with_perf_stats(perf_stats);
    }

    pub fn mark_error(&mut self, perf_stats: PerfStats) {
        self.finish_with_perf_stats(perf_stats);
    }

    pub fn apply_progress(&mut self, progress: ProgressSnapshot) {
        self.progress = Some(ProgressSummary {
            files_scanned: progress.files_scanned,
            dirs_scanned: progress.dirs_scanned,
            bytes_seen: progress.bytes_seen,
            current_path: progress.current_path,
        });
    }

    pub fn observe_node(&mut self, node: &NodeRecord) {
        match node.kind {
            NodeKind::Symlink => {
                self.issue_summary.symlinks += 1;
            }
            NodeKind::Error => {
                self.issue_summary.error_entries += 1;
                self.issue_summary.skipped_paths += 1;
                if is_permission_error(node.error.as_deref()) {
                    self.issue_summary.permission_errors += 1;
                }
            }
            NodeKind::File | NodeKind::Dir | NodeKind::Aggregate => {}
        }
    }

    pub fn progress(&self) -> Option<&ProgressSummary> {
        self.progress.as_ref()
    }

    pub fn issue_summary(&self) -> ScanIssueSummary {
        self.issue_summary
    }

    pub fn perf_stats(&self) -> &PerfStats {
        &self.perf_stats
    }

    pub fn is_scanning(&self) -> bool {
        self.scanning
    }

    #[cfg(test)]
    pub fn active_id(&self) -> u64 {
        self.active_scan_id
    }

    #[cfg(test)]
    pub fn has_handle(&self) -> bool {
        self.handle.is_some()
    }

    pub fn record_layout_recompute(&mut self, elapsed: Duration) {
        self.perf_stats.layout_recompute_count += 1;
        self.perf_stats.layout_total_ms += elapsed.as_secs_f64() * 1000.0;
    }

    pub fn record_search_rebuild(&mut self) {
        self.perf_stats.search_rebuild_count += 1;
    }

    pub fn record_search_incremental_updates(&mut self, updates: u64) {
        self.perf_stats.search_incremental_updates += updates;
    }

    fn begin_scan(&mut self) -> u64 {
        if let Some(handle) = &self.handle {
            handle.cancel();
        }

        self.scan_counter += 1;
        self.active_scan_id = self.scan_counter;
        self.handle = None;
        self.scanning = true;
        self.progress = None;
        self.issue_summary = ScanIssueSummary::default();
        self.perf_stats = PerfStats::default();
        self.active_scan_id
    }

    fn finish_with_perf_stats(&mut self, perf_stats: PerfStats) {
        self.scanning = false;
        self.handle = None;
        self.merge_scan_perf_stats(perf_stats);
    }

    fn merge_scan_perf_stats(&mut self, perf_stats: PerfStats) {
        let layout_recompute_count = self.perf_stats.layout_recompute_count;
        let layout_total_ms = self.perf_stats.layout_total_ms;
        let search_rebuild_count = self.perf_stats.search_rebuild_count;
        let search_incremental_updates = self.perf_stats.search_incremental_updates;

        self.perf_stats = perf_stats;
        self.perf_stats.layout_recompute_count = layout_recompute_count;
        self.perf_stats.layout_total_ms = layout_total_ms;
        self.perf_stats.search_rebuild_count = search_rebuild_count;
        self.perf_stats.search_incremental_updates = search_incremental_updates;
    }

    #[cfg(test)]
    pub fn set_active_id_for_test(&mut self, scan_id: u64) {
        self.active_scan_id = scan_id;
        self.scan_counter = self.scan_counter.max(scan_id);
    }
}

fn is_permission_error(error: Option<&str>) -> bool {
    let Some(error) = error else {
        return false;
    };
    let lower = error.to_ascii_lowercase();
    lower.contains("permission denied") || lower.contains("operation not permitted")
}

pub fn scan_id_for_message(message: &ScanMessage) -> u64 {
    match message {
        ScanMessage::Started { scan_id, .. }
        | ScanMessage::Batch { scan_id, .. }
        | ScanMessage::Finished { scan_id, .. }
        | ScanMessage::Cancelled { scan_id, .. }
        | ScanMessage::Error { scan_id, .. } => *scan_id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::TreeStore;
    use crate::tree::{NodeKind, NodeRecord};

    fn started(scan_id: u64) -> ScanMessage {
        ScanMessage::Started {
            scan_id,
            path: "/root".into(),
            root_node: TreeStore::root_record("root".into()),
        }
    }

    #[test]
    fn begin_scan_increments_active_scan_id_and_resets_runtime_state() {
        let mut session = ScanSession::default();
        session.apply_progress(ProgressSnapshot {
            files_scanned: 1,
            dirs_scanned: 1,
            bytes_seen: 8,
            current_path: "/root/file".into(),
        });

        assert_eq!(session.begin_scan(), 1);
        assert_eq!(session.begin_scan(), 2);
        assert_eq!(session.active_id(), 2);
        assert!(session.is_scanning());
        assert!(session.progress().is_none());
    }

    #[test]
    fn accepts_only_current_scan_messages() {
        let mut session = ScanSession::default();
        session.set_active_id_for_test(2);

        assert!(session.accepts(&started(2)));
        assert!(!session.accepts(&started(1)));
    }

    #[test]
    fn progress_snapshot_is_stored_as_summary() {
        let mut session = ScanSession::default();

        session.apply_progress(ProgressSnapshot {
            files_scanned: 3,
            dirs_scanned: 2,
            bytes_seen: 128,
            current_path: "/root/current/file.txt".into(),
        });

        let progress = session.progress().expect("progress summary");
        assert_eq!(progress.files_scanned, 3);
        assert_eq!(progress.dirs_scanned, 2);
        assert_eq!(progress.bytes_seen, 128);
        assert_eq!(
            progress.current_path,
            PathBuf::from("/root/current/file.txt")
        );
    }

    #[test]
    fn observe_node_tracks_error_permission_skip_and_symlink_counts() {
        let mut session = ScanSession::default();

        session.observe_node(&NodeRecord {
            name: "private".into(),
            kind: NodeKind::Error,
            size: 0,
            scanned: true,
            error: Some("Permission denied (os error 13)".into()),
        });
        session.observe_node(&NodeRecord {
            name: "linked".into(),
            kind: NodeKind::Symlink,
            size: 0,
            scanned: true,
            error: None,
        });

        let summary = session.issue_summary();
        assert_eq!(summary.error_entries, 1);
        assert_eq!(summary.permission_errors, 1);
        assert_eq!(summary.skipped_paths, 1);
        assert_eq!(summary.symlinks, 1);
        assert_eq!(summary.issue_count(), 1);
        assert!(summary.has_findings());
    }

    #[test]
    fn finishing_scan_preserves_ui_perf_counters() {
        let mut session = ScanSession::default();
        session.record_layout_recompute(Duration::from_millis(7));
        session.record_search_rebuild();
        session.record_search_incremental_updates(2);
        let scanner_stats = PerfStats {
            messages_sent: 5,
            entries_seen: 9,
            ..PerfStats::default()
        };

        session.mark_finished(scanner_stats);

        assert_eq!(session.perf_stats().messages_sent, 5);
        assert_eq!(session.perf_stats().entries_seen, 9);
        assert_eq!(session.perf_stats().layout_recompute_count, 1);
        assert_eq!(session.perf_stats().search_rebuild_count, 1);
        assert_eq!(session.perf_stats().search_incremental_updates, 2);
    }
}
