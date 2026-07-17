use crossbeam_channel::{unbounded, Receiver, Sender};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const WATCH_DEBOUNCE_MIN: Duration = Duration::from_millis(300);
pub const WATCH_DEBOUNCE_MAX: Duration = Duration::from_millis(1000);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchChange {
    pub paths: Vec<PathBuf>,
}

#[derive(Debug)]
enum WatchMessage {
    Changed(WatchChange),
    Error(String),
}

pub struct WatchSession {
    _watcher: RecommendedWatcher,
    rx: Receiver<WatchMessage>,
    root_path: PathBuf,
    pending: Option<PendingChange>,
}

#[derive(Debug)]
struct PendingChange {
    first_seen: Instant,
    last_seen: Instant,
    paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchPoll {
    Noop,
    Pending,
    Ready(WatchChange),
    Error(String),
}

impl WatchSession {
    pub fn start(root_path: PathBuf) -> notify::Result<Self> {
        let root_path: PathBuf = root_path.components().collect();
        let (tx, rx) = unbounded();
        let mut watcher = create_watcher(tx)?;
        let mode = if root_is_directory_without_following_symlinks(&root_path) {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };
        watcher.watch(&root_path, mode)?;

        Ok(Self {
            _watcher: watcher,
            rx,
            root_path,
            pending: None,
        })
    }

    pub fn root_path(&self) -> &PathBuf {
        &self.root_path
    }

    pub fn poll(&mut self, now: Instant) -> WatchPoll {
        let mut saw_message = false;
        while let Ok(message) = self.rx.try_recv() {
            saw_message = true;
            match message {
                WatchMessage::Changed(change) => self.ingest_change(change, now),
                WatchMessage::Error(error) => return WatchPoll::Error(error),
            }
        }

        if self.pending_ready(now) {
            let pending = self.pending.take().expect("pending change");
            return WatchPoll::Ready(WatchChange {
                paths: pending.paths,
            });
        }

        if self.pending.is_some() || saw_message {
            WatchPoll::Pending
        } else {
            WatchPoll::Noop
        }
    }

    fn ingest_change(&mut self, change: WatchChange, now: Instant) {
        let Some(pending) = &mut self.pending else {
            self.pending = Some(PendingChange {
                first_seen: now,
                last_seen: now,
                paths: dedup_paths(change.paths),
            });
            return;
        };

        pending.last_seen = now;
        pending.paths.extend(change.paths);
        pending.paths = dedup_paths(std::mem::take(&mut pending.paths));
    }

    fn pending_ready(&self, now: Instant) -> bool {
        let Some(pending) = &self.pending else {
            return false;
        };

        debounce_ready(pending.first_seen, pending.last_seen, now)
    }
}

fn root_is_directory_without_following_symlinks(path: &Path) -> bool {
    let path: PathBuf = path.components().collect();
    std::fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_dir())
        .unwrap_or(false)
}

fn create_watcher(tx: Sender<WatchMessage>) -> notify::Result<RecommendedWatcher> {
    notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        let message = match result {
            Ok(event) => WatchMessage::Changed(WatchChange { paths: event.paths }),
            Err(error) => WatchMessage::Error(error.to_string()),
        };
        let _ = tx.send(message);
    })
}

fn dedup_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for path in paths {
        if !out.iter().any(|existing| existing == &path) {
            out.push(path);
        }
    }
    out
}

fn debounce_ready(first_seen: Instant, last_seen: Instant, now: Instant) -> bool {
    now.duration_since(last_seen) >= WATCH_DEBOUNCE_MIN
        || now.duration_since(first_seen) >= WATCH_DEBOUNCE_MAX
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debounce_waits_for_minimum_quiet_window() {
        let start = Instant::now();
        let pending = PendingChange {
            first_seen: start,
            last_seen: start + Duration::from_millis(100),
            paths: vec![PathBuf::from("/root/file")],
        };
        let session_ready = debounce_ready(
            pending.first_seen,
            pending.last_seen,
            start + Duration::from_millis(400),
        );
        let session_pending = debounce_ready(
            pending.first_seen,
            pending.last_seen,
            start + Duration::from_millis(250),
        );

        assert!(session_ready);
        assert!(!session_pending);
    }

    #[test]
    fn debounce_forces_ready_after_maximum_window() {
        let start = Instant::now();
        let pending = PendingChange {
            first_seen: start,
            last_seen: start + Duration::from_millis(900),
            paths: vec![PathBuf::from("/root/file")],
        };

        assert!(debounce_ready(
            pending.first_seen,
            pending.last_seen,
            start + Duration::from_millis(1000),
        ));
    }

    #[test]
    fn dedup_paths_preserves_first_seen_order() {
        let paths = dedup_paths(vec![
            PathBuf::from("/a"),
            PathBuf::from("/b"),
            PathBuf::from("/a"),
        ]);

        assert_eq!(paths, vec![PathBuf::from("/a"), PathBuf::from("/b")]);
    }

    #[cfg(unix)]
    #[test]
    fn directory_symlink_root_is_not_classified_as_recursive_directory() {
        use std::os::unix::fs::symlink;

        let root = std::env::current_dir()
            .expect("test current dir should be available")
            .join("target/test-temp/watcher-symlink-root");
        std::fs::create_dir_all(&root).expect("watcher symlink fixture should be created");
        let target = root.join("target");
        let link = root.join("link");
        std::fs::create_dir(&target).expect("watcher target should be created");
        symlink(&target, &link).expect("watcher symlink should be created");

        assert!(!root_is_directory_without_following_symlinks(&link));

        let _ = std::fs::remove_file(link);
        let _ = std::fs::remove_dir_all(root);
    }
}
