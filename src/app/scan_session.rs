use crate::scanner::{self, PerfStats, ProgressSnapshot, ScanHandle, ScanMessage, ScanOptions};
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

#[derive(Debug, Default)]
pub struct ScanSession {
    active_scan_id: u64,
    scan_counter: u64,
    handle: Option<ScanHandle>,
    scanning: bool,
    progress: Option<ProgressSummary>,
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

    pub fn progress(&self) -> Option<&ProgressSummary> {
        self.progress.as_ref()
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
