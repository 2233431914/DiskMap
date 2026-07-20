use crate::scanner::{
    self, PerfStats, ProgressSnapshot, ScanBatch, ScanHandle, ScanMessage, ScanOptions,
};
use crate::tree::{NodeKind, NodeRecord};
use crate::watcher::{WatchPoll, WatchSession};
use crossbeam_channel::{unbounded, Receiver, Sender};
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct ProgressSummary {
    pub files_scanned: u64,
    pub total_files: Option<u64>,
    pub dirs_scanned: u64,
    pub bytes_seen: u64,
    pub current_path: PathBuf,
}

impl ProgressSummary {
    pub fn file_progress_fraction(&self) -> Option<f32> {
        let total_files = self.total_files?;
        if total_files == 0 {
            return Some(1.0);
        }
        Some((self.files_scanned as f32 / total_files as f32).clamp(0.0, 1.0))
    }
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

#[derive(Debug)]
pub(super) enum ScanSessionEvent {
    Started {
        path: PathBuf,
        root_node: NodeRecord,
    },
    Batch(ScanBatch),
    Finished {
        total_bytes: u64,
        follow_up_rescan: Option<PathBuf>,
        watch_error: Option<String>,
    },
    Cancelled {
        watch_paused: bool,
    },
    Error {
        message: String,
        watch_paused: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ScanPhase {
    Idle,
    Running,
    Finished,
    Cancelled,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum WatchAction {
    Noop,
    Pending,
    Deferred { change_count: usize },
    Rescan { path: PathBuf, change_count: usize },
    Failed(String),
}

struct ActiveScan {
    handle: Option<ScanHandle>,
    started_at: Instant,
    cancelling: bool,
}

pub struct ScanSession {
    scan_counter: u64,
    tx: Sender<ScanMessage>,
    rx: Receiver<ScanMessage>,
    active_scan: Option<ActiveScan>,
    active_root: Option<PathBuf>,
    progress: Option<ProgressSummary>,
    issue_summary: ScanIssueSummary,
    perf_stats: PerfStats,
    terminal_perf_stats_available: bool,
    phase: ScanPhase,
    watch_enabled: bool,
    watcher: Option<WatchSession>,
    watch_rescan_pending: bool,
    allow_watch_follow_up: bool,
    last_successful_root: Option<PathBuf>,
}

impl Default for ScanSession {
    fn default() -> Self {
        let (tx, rx) = unbounded();
        Self {
            scan_counter: 0,
            tx,
            rx,
            active_scan: None,
            active_root: None,
            progress: None,
            issue_summary: ScanIssueSummary::default(),
            perf_stats: PerfStats::default(),
            terminal_perf_stats_available: false,
            phase: ScanPhase::Idle,
            watch_enabled: true,
            watcher: None,
            watch_rescan_pending: false,
            allow_watch_follow_up: true,
            last_successful_root: None,
        }
    }
}

impl ScanSession {
    pub fn start(&mut self, path: PathBuf, options: ScanOptions) -> u64 {
        self.start_inner(path, options, true)
    }

    pub(super) fn start_watch_follow_up(&mut self, path: PathBuf, options: ScanOptions) -> u64 {
        self.start_inner(path, options, false)
    }

    fn start_inner(
        &mut self,
        path: PathBuf,
        options: ScanOptions,
        allow_watch_follow_up: bool,
    ) -> u64 {
        if let Some(active) = &self.active_scan {
            if let Some(handle) = &active.handle {
                handle.cancel();
            }
        }

        let normalized_path: PathBuf = path.components().collect();
        if self
            .watcher
            .as_ref()
            .is_some_and(|watcher| watcher.root_path() != &normalized_path)
        {
            self.stop_watching();
        }

        self.scan_counter += 1;
        let scan_id = self.scan_counter;
        self.progress = None;
        self.issue_summary = ScanIssueSummary::default();
        self.perf_stats = PerfStats::default();
        self.terminal_perf_stats_available = false;
        self.phase = ScanPhase::Running;
        self.watch_rescan_pending = false;
        self.allow_watch_follow_up = allow_watch_follow_up;
        self.active_root = Some(normalized_path);
        let handle = scanner::start_scan(path, scan_id, options, self.tx.clone());
        self.active_scan = Some(ActiveScan {
            handle: Some(handle),
            started_at: Instant::now(),
            cancelling: false,
        });
        scan_id
    }

    pub fn cancel(&mut self) -> bool {
        let Some(active) = &mut self.active_scan else {
            return false;
        };
        if !active.cancelling {
            if let Some(handle) = &active.handle {
                handle.cancel();
            }
            active.cancelling = true;
        }
        true
    }

    pub(super) fn try_next_event(&mut self) -> Option<ScanSessionEvent> {
        loop {
            let message = self.rx.try_recv().ok()?;
            if let Some(event) = self.process_message(message) {
                return Some(event);
            }
        }
    }

    fn process_message(&mut self, message: ScanMessage) -> Option<ScanSessionEvent> {
        if scan_id_for_message(&message) != self.scan_counter {
            return None;
        }

        match message {
            ScanMessage::Started {
                path, root_node, ..
            } => {
                self.phase = ScanPhase::Running;
                if self.active_scan.is_none() {
                    self.active_scan = Some(ActiveScan {
                        handle: None,
                        started_at: Instant::now(),
                        cancelling: false,
                    });
                }
                self.terminal_perf_stats_available = false;
                self.active_root = Some(path.clone());
                self.observe_node(&root_node);
                Some(ScanSessionEvent::Started { path, root_node })
            }
            ScanMessage::Batch { batch, .. } => {
                for discovered in &batch.discovered_nodes {
                    self.observe_node(&discovered.node);
                }
                if let Some(progress) = &batch.progress {
                    self.apply_progress(progress);
                }
                Some(ScanSessionEvent::Batch(batch))
            }
            ScanMessage::Finished {
                total_bytes,
                perf_stats,
                ..
            } => {
                self.finish_with_perf_stats(perf_stats);
                self.phase = ScanPhase::Finished;
                self.last_successful_root = self.active_root.take();
                let watch_rescan_pending = std::mem::take(&mut self.watch_rescan_pending);
                let watch_error = self.sync_watcher().err();
                let follow_up_rescan = (self.watch_enabled
                    && self.allow_watch_follow_up
                    && watch_rescan_pending)
                    .then(|| self.last_successful_root.clone())
                    .flatten();
                Some(ScanSessionEvent::Finished {
                    total_bytes,
                    follow_up_rescan,
                    watch_error,
                })
            }
            ScanMessage::Cancelled { perf_stats, .. } => {
                self.finish_with_perf_stats(perf_stats);
                self.phase = ScanPhase::Cancelled;
                let watch_paused = self.pause_watch_after_terminal();
                Some(ScanSessionEvent::Cancelled { watch_paused })
            }
            ScanMessage::Error {
                message,
                perf_stats,
                ..
            } => {
                self.finish_with_perf_stats(perf_stats);
                self.phase = ScanPhase::Failed(message.clone());
                let watch_paused = self.pause_watch_after_terminal();
                Some(ScanSessionEvent::Error {
                    message,
                    watch_paused,
                })
            }
        }
    }

    pub(super) fn poll_watch(&mut self, now: Instant) -> WatchAction {
        let poll = match &mut self.watcher {
            Some(watcher) => watcher.poll(now),
            None => return WatchAction::Noop,
        };

        self.handle_watch_poll(poll)
    }

    fn handle_watch_poll(&mut self, poll: WatchPoll) -> WatchAction {
        match poll {
            WatchPoll::Noop => WatchAction::Noop,
            WatchPoll::Pending => WatchAction::Pending,
            WatchPoll::Ready(change) => self.handle_ready_watch_change(change.paths.len()),
            WatchPoll::Error(error) => {
                self.stop_watching();
                WatchAction::Failed(error)
            }
        }
    }

    fn handle_ready_watch_change(&mut self, change_count: usize) -> WatchAction {
        if !self.watch_enabled {
            return WatchAction::Noop;
        }
        if self.is_scanning() {
            self.watch_rescan_pending = true;
            return WatchAction::Deferred { change_count };
        }
        self.watcher
            .as_ref()
            .map(|watcher| WatchAction::Rescan {
                path: watcher.root_path().clone(),
                change_count,
            })
            .unwrap_or(WatchAction::Noop)
    }

    pub(super) fn set_watch_enabled(&mut self, enabled: bool) -> Result<(), String> {
        self.watch_enabled = enabled;
        if !enabled {
            self.watch_rescan_pending = false;
            self.stop_watching();
            return Ok(());
        }
        if self.is_scanning() {
            return Ok(());
        }
        self.sync_watcher()
    }

    fn apply_progress(&mut self, progress: &ProgressSnapshot) {
        self.progress = Some(ProgressSummary {
            files_scanned: progress.files_scanned,
            total_files: progress.total_files,
            dirs_scanned: progress.dirs_scanned,
            bytes_seen: progress.bytes_seen,
            current_path: progress.current_path.clone(),
        });
    }

    fn observe_node(&mut self, node: &NodeRecord) {
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

    #[cfg(test)]
    pub(super) fn terminal_perf_stats(&self) -> Option<&PerfStats> {
        self.terminal_perf_stats_available
            .then_some(&self.perf_stats)
    }

    pub fn is_scanning(&self) -> bool {
        self.active_scan.is_some()
    }

    pub(super) fn phase(&self) -> &ScanPhase {
        &self.phase
    }

    pub(super) fn watch_enabled(&self) -> bool {
        self.watch_enabled
    }

    pub(super) fn watch_active(&self) -> bool {
        self.watcher.is_some()
    }

    pub(super) fn pause_watching(&mut self) {
        self.stop_watching();
        self.watch_rescan_pending = false;
        self.last_successful_root = None;
    }

    pub fn elapsed(&self) -> Option<Duration> {
        if let Some(active) = &self.active_scan {
            return Some(active.started_at.elapsed());
        }
        if self.perf_stats.scan_elapsed_ms > 0.0 {
            return Some(Duration::from_secs_f64(
                self.perf_stats.scan_elapsed_ms / 1000.0,
            ));
        }
        None
    }

    #[cfg(test)]
    pub fn active_id(&self) -> u64 {
        self.scan_counter
    }

    #[cfg(test)]
    pub fn has_handle(&self) -> bool {
        self.active_scan
            .as_ref()
            .is_some_and(|active| active.handle.is_some())
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

    fn finish_with_perf_stats(&mut self, perf_stats: PerfStats) {
        self.active_scan = None;
        self.merge_scan_perf_stats(perf_stats);
        self.terminal_perf_stats_available = true;
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

    fn sync_watcher(&mut self) -> Result<(), String> {
        if !self.watch_enabled {
            self.stop_watching();
            return Ok(());
        }
        let Some(root_path) = self.last_successful_root.clone() else {
            self.stop_watching();
            return Ok(());
        };
        if self
            .watcher
            .as_ref()
            .is_some_and(|watcher| watcher.root_path() == &root_path)
        {
            return Ok(());
        }
        self.stop_watching();
        self.watcher = Some(WatchSession::start(root_path).map_err(|error| error.to_string())?);
        Ok(())
    }

    fn stop_watching(&mut self) {
        self.watcher = None;
    }

    fn pause_watch_after_terminal(&mut self) -> bool {
        let watch_paused = self.watch_enabled;
        self.pause_watching();
        self.active_root = None;
        watch_paused
    }

    #[cfg(test)]
    pub fn set_active_id_for_test(&mut self, scan_id: u64) {
        self.scan_counter = self.scan_counter.max(scan_id);
    }

    #[cfg(test)]
    pub(super) fn process_message_for_test(
        &mut self,
        message: ScanMessage,
    ) -> Option<ScanSessionEvent> {
        self.process_message(message)
    }

    #[cfg(test)]
    fn has_pending_watch_rescan_for_test(&self) -> bool {
        self.watch_rescan_pending
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
        started_at(scan_id, "/root")
    }

    fn started_at(scan_id: u64, path: impl Into<PathBuf>) -> ScanMessage {
        let path = path.into();
        ScanMessage::Started {
            scan_id,
            root_node: TreeStore::root_record(
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "root".into()),
            ),
            path,
        }
    }

    fn watch_fixture(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("disk-map-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("watch fixture should be created");
        root
    }

    #[test]
    fn begin_scan_increments_active_scan_id_and_resets_runtime_state() {
        let mut session = ScanSession::default();
        assert_eq!(session.phase(), &ScanPhase::Idle);
        session.apply_progress(&ProgressSnapshot {
            files_scanned: 1,
            total_files: Some(2),
            dirs_scanned: 1,
            bytes_seen: 8,
            current_path: "/root/file".into(),
        });

        assert_eq!(
            session.start("/missing-one".into(), ScanOptions::default()),
            1
        );
        assert_eq!(
            session.start("/missing-two".into(), ScanOptions::default()),
            2
        );
        assert_eq!(session.active_id(), 2);
        assert_eq!(session.phase(), &ScanPhase::Running);
        assert!(session.is_scanning());
        assert!(session.progress().is_none());
    }

    #[test]
    fn accepts_only_current_scan_messages() {
        let mut session = ScanSession::default();
        session.set_active_id_for_test(2);

        assert!(session.process_message_for_test(started(2)).is_some());
        assert!(session.process_message_for_test(started(1)).is_none());
    }

    #[test]
    fn progress_snapshot_is_stored_as_summary() {
        let mut session = ScanSession::default();

        session.apply_progress(&ProgressSnapshot {
            files_scanned: 3,
            total_files: Some(6),
            dirs_scanned: 2,
            bytes_seen: 128,
            current_path: "/root/current/file.txt".into(),
        });

        let progress = session.progress().expect("progress summary");
        assert_eq!(progress.files_scanned, 3);
        assert_eq!(progress.total_files, Some(6));
        assert_eq!(progress.file_progress_fraction(), Some(0.5));
        assert_eq!(progress.dirs_scanned, 2);
        assert_eq!(progress.bytes_seen, 128);
        assert_eq!(
            progress.current_path,
            PathBuf::from("/root/current/file.txt")
        );
    }

    #[test]
    fn elapsed_reports_running_or_finished_scan_duration() {
        let mut session = ScanSession::default();
        session.set_active_id_for_test(1);
        session.process_message_for_test(started(1));

        assert!(session.elapsed().is_some());

        session.process_message_for_test(ScanMessage::Finished {
            scan_id: 1,
            total_bytes: 0,
            perf_stats: PerfStats {
                scan_elapsed_ms: 1_250.0,
                ..PerfStats::default()
            },
        });

        assert_eq!(session.elapsed(), Some(Duration::from_millis(1250)));
    }

    #[test]
    fn observe_node_tracks_error_permission_skip_and_symlink_counts() {
        let mut session = ScanSession::default();

        session.observe_node(&NodeRecord {
            name: "private".into(),
            kind: NodeKind::Error,
            size: 0,
            modified_secs: None,
            scanned: true,
            error: Some("Permission denied (os error 13)".into()),
        });
        session.observe_node(&NodeRecord {
            name: "linked".into(),
            kind: NodeKind::Symlink,
            size: 0,
            modified_secs: None,
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

        session.set_active_id_for_test(1);
        session.process_message_for_test(ScanMessage::Finished {
            scan_id: 1,
            total_bytes: 0,
            perf_stats: scanner_stats,
        });

        assert_eq!(session.perf_stats().messages_sent, 5);
        assert_eq!(session.perf_stats().entries_seen, 9);
        assert_eq!(session.perf_stats().layout_recompute_count, 1);
        assert_eq!(session.perf_stats().search_rebuild_count, 1);
        assert_eq!(session.perf_stats().search_incremental_updates, 2);
    }

    #[test]
    fn deferred_watch_changes_coalesce_into_one_follow_up_rescan() {
        let mut session = ScanSession::default();
        let root = watch_fixture("watch-coalesce");
        session.set_active_id_for_test(1);
        session.process_message_for_test(started_at(1, root.clone()));

        assert!(matches!(
            session.handle_ready_watch_change(2),
            WatchAction::Deferred { change_count: 2 }
        ));
        assert!(matches!(
            session.handle_ready_watch_change(3),
            WatchAction::Deferred { change_count: 3 }
        ));

        let event = session
            .process_message_for_test(ScanMessage::Finished {
                scan_id: 1,
                total_bytes: 8,
                perf_stats: PerfStats::default(),
            })
            .expect("current scan should produce an event");

        assert!(matches!(
            event,
            ScanSessionEvent::Finished {
                follow_up_rescan: Some(path),
                ..
            } if path == root
        ));
        assert_eq!(session.phase(), &ScanPhase::Finished);
        session.pause_watching();
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn automatic_follow_up_scan_does_not_schedule_another_follow_up() {
        let mut session = ScanSession::default();
        let root = watch_fixture("watch-follow-up-guard");
        session.set_active_id_for_test(1);
        session.process_message_for_test(started_at(1, root.clone()));
        session.allow_watch_follow_up = false;
        session.handle_ready_watch_change(1);

        let event = session
            .process_message_for_test(ScanMessage::Finished {
                scan_id: 1,
                total_bytes: 8,
                perf_stats: PerfStats::default(),
            })
            .expect("scan should finish");

        assert!(matches!(
            event,
            ScanSessionEvent::Finished {
                follow_up_rescan: None,
                ..
            }
        ));
        session.pause_watching();
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn watcher_error_preserves_deferred_follow_up_rescan() {
        let mut session = ScanSession::default();
        let root = watch_fixture("watch-error-deferred-rescan");
        session.set_active_id_for_test(1);
        session.process_message_for_test(started_at(1, root.clone()));
        session.handle_ready_watch_change(1);

        session.handle_watch_poll(WatchPoll::Error("backend failed".into()));
        let event = session
            .process_message_for_test(ScanMessage::Finished {
                scan_id: 1,
                total_bytes: 8,
                perf_stats: PerfStats::default(),
            })
            .expect("current scan should produce an event");

        assert!(matches!(
            event,
            ScanSessionEvent::Finished {
                follow_up_rescan: Some(path),
                ..
            } if path == root
        ));
        session.pause_watching();
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn disabling_watch_discards_deferred_rescan_before_finish() {
        let mut session = ScanSession::default();
        session.set_active_id_for_test(1);
        session.process_message_for_test(started(1));
        session.handle_ready_watch_change(1);

        session
            .set_watch_enabled(false)
            .expect("disabling Watch should not fail");
        let event = session
            .process_message_for_test(ScanMessage::Finished {
                scan_id: 1,
                total_bytes: 8,
                perf_stats: PerfStats::default(),
            })
            .expect("current scan should produce an event");

        assert!(matches!(
            event,
            ScanSessionEvent::Finished {
                follow_up_rescan: None,
                ..
            }
        ));
    }

    #[test]
    fn cancelled_scan_pauses_watch_without_disabling_preference() {
        let mut session = ScanSession::default();
        session.set_active_id_for_test(1);
        session.process_message_for_test(started(1));
        session.handle_ready_watch_change(1);

        let event = session
            .process_message_for_test(ScanMessage::Cancelled {
                scan_id: 1,
                perf_stats: PerfStats::default(),
            })
            .expect("current scan should produce an event");

        assert!(matches!(
            event,
            ScanSessionEvent::Cancelled { watch_paused: true }
        ));
        assert!(session.watch_enabled());
        assert!(!session.watch_active());
        assert!(!session.has_pending_watch_rescan_for_test());
        assert_eq!(session.phase(), &ScanPhase::Cancelled);
    }

    #[test]
    fn errored_scan_pauses_watch_without_disabling_preference() {
        let mut session = ScanSession::default();
        session.set_active_id_for_test(1);
        session.process_message_for_test(started(1));
        session.handle_ready_watch_change(1);

        let event = session
            .process_message_for_test(ScanMessage::Error {
                scan_id: 1,
                message: "root unavailable".into(),
                perf_stats: PerfStats::default(),
            })
            .expect("current scan should produce an event");

        assert!(matches!(
            event,
            ScanSessionEvent::Error {
                watch_paused: true,
                ..
            }
        ));
        assert!(session.watch_enabled());
        assert!(!session.watch_active());
        assert!(!session.has_pending_watch_rescan_for_test());
        assert_eq!(
            session.phase(),
            &ScanPhase::Failed("root unavailable".into())
        );
    }

    #[test]
    fn successful_scan_resumes_watch_after_activation_failure() {
        let mut session = ScanSession::default();
        let missing = std::env::temp_dir().join("disk-map-missing-watch-root");
        let _ = std::fs::remove_dir_all(&missing);
        session.set_active_id_for_test(1);
        session.process_message_for_test(started_at(1, missing));
        let failed = session
            .process_message_for_test(ScanMessage::Finished {
                scan_id: 1,
                total_bytes: 0,
                perf_stats: PerfStats::default(),
            })
            .expect("current scan should produce an event");
        assert!(matches!(
            failed,
            ScanSessionEvent::Finished {
                watch_error: Some(_),
                ..
            }
        ));

        let root = watch_fixture("watch-recovery");
        session.set_active_id_for_test(2);
        session.process_message_for_test(started_at(2, root.clone()));
        session.process_message_for_test(ScanMessage::Finished {
            scan_id: 2,
            total_bytes: 0,
            perf_stats: PerfStats::default(),
        });

        assert!(session.watch_active());
        session.pause_watching();
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn starting_same_root_keeps_watcher_and_new_root_stops_it() {
        let root_a = watch_fixture("watch-root-a");
        let root_b = watch_fixture("watch-root-b");
        let mut session = ScanSession::default();
        session.set_active_id_for_test(1);
        session.process_message_for_test(started_at(1, root_a.clone()));
        session.process_message_for_test(ScanMessage::Finished {
            scan_id: 1,
            total_bytes: 0,
            perf_stats: PerfStats::default(),
        });
        assert!(session.watch_active());

        session.start(root_a.clone(), ScanOptions::default());
        assert!(session.watch_active());
        session.start(root_b.clone(), ScanOptions::default());
        assert!(!session.watch_active());

        session.cancel();
        session.pause_watching();
        let _ = std::fs::remove_dir_all(root_a);
        let _ = std::fs::remove_dir_all(root_b);
    }

    #[test]
    fn batch_event_updates_progress_and_issue_summary_before_ui_consumes_it() {
        let mut session = ScanSession::default();
        session.set_active_id_for_test(1);
        session.process_message_for_test(started(1));
        let batch = crate::scanner::ScanBatch {
            discovered_nodes: vec![crate::scanner::DiscoveredNode {
                node_id: 1,
                parent_id: 0,
                node: NodeRecord {
                    name: "blocked".into(),
                    kind: NodeKind::Error,
                    size: 0,
                    modified_secs: None,
                    scanned: true,
                    error: Some("Permission denied".into()),
                },
            }],
            progress: Some(ProgressSnapshot {
                files_scanned: 4,
                total_files: None,
                dirs_scanned: 2,
                bytes_seen: 16,
                current_path: "/root/blocked".into(),
            }),
            ..Default::default()
        };

        let event = session
            .process_message_for_test(ScanMessage::Batch { scan_id: 1, batch })
            .expect("current scan should produce an event");

        assert!(matches!(event, ScanSessionEvent::Batch(_)));
        assert_eq!(
            session.progress().map(|progress| progress.files_scanned),
            Some(4)
        );
        assert_eq!(session.issue_summary().permission_errors, 1);
    }
}
