use crate::db::ScanDb;
use crate::tree::{NodeId, NodeKind, NodeRecord, TreeStore};

use crossbeam_channel::Sender;
use jwalk::{ClientState, WalkDirGeneric};
use std::collections::HashMap;
use std::fs::Metadata;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheMode {
    Disabled,
    Enabled,
}

#[derive(Debug, Clone, Copy)]
pub struct ScanOptions {
    pub batch_flush_interval: Duration,
    pub max_pending_nodes: usize,
    pub max_pending_size_deltas: usize,
    pub cache_mode: CacheMode,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            batch_flush_interval: Duration::from_millis(33),
            max_pending_nodes: 2_048,
            max_pending_size_deltas: 4_096,
            cache_mode: CacheMode::Disabled,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct PerfStats {
    pub messages_sent: u64,
    pub batches_sent: u64,
    pub entries_seen: u64,
    pub nodes_discovered: u64,
    pub size_delta_merges: u64,
    pub parent_stack_hits: u64,
    pub parent_lookup_fallbacks: u64,
    pub progress_snapshots_sent: u64,
    pub metadata_total_ms: f64,
    pub size_measure_total_ms: f64,
    pub batch_flush_total_ms: f64,
    pub scan_elapsed_ms: f64,
    pub layout_recompute_count: u64,
    pub layout_total_ms: f64,
    pub search_rebuild_count: u64,
    pub search_incremental_updates: u64,
    pub db_cache_hits: u64,
    pub db_cache_misses: u64,
    pub db_flush_count: u64,
}

#[derive(Debug, Clone)]
pub struct ProgressSnapshot {
    pub files_scanned: u64,
    pub dirs_scanned: u64,
    pub bytes_seen: u64,
    pub current_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct DiscoveredNode {
    pub node_id: NodeId,
    pub parent_id: NodeId,
    pub node: NodeRecord,
}

#[derive(Debug, Clone, Default)]
pub struct ScanBatch {
    pub discovered_nodes: Vec<DiscoveredNode>,
    pub size_deltas: Vec<(NodeId, u64)>,
    pub scanned_nodes: Vec<NodeId>,
    pub progress: Option<ProgressSnapshot>,
}

#[derive(Debug, Clone)]
pub enum ScanMessage {
    Started {
        scan_id: u64,
        path: PathBuf,
        root_node: NodeRecord,
    },
    Batch {
        scan_id: u64,
        batch: ScanBatch,
    },
    Finished {
        scan_id: u64,
        total_bytes: u64,
        perf_stats: PerfStats,
    },
    Cancelled {
        scan_id: u64,
        perf_stats: PerfStats,
    },
    Error {
        scan_id: u64,
        message: String,
        perf_stats: PerfStats,
    },
}

#[derive(Debug, Clone)]
pub struct ScanHandle {
    cancel: Arc<AtomicBool>,
}

#[derive(Debug, Default)]
struct ScanWalkState;

impl ClientState for ScanWalkState {
    type ReadDirState = ();
    type DirEntryState = EntryClientState;
}

#[derive(Debug, Default)]
struct EntryClientState {
    prefetched_file: Option<Result<PrefetchedFileInfo, String>>,
}

#[derive(Debug)]
struct PrefetchedFileInfo {
    metadata: Metadata,
    mtime: u64,
    measured_size: u64,
}

#[derive(Debug, Default)]
struct PrefetchPerfCounters {
    metadata_ns: AtomicU64,
    size_ns: AtomicU64,
}

const PREFETCH_MAX_FILES_PER_DIR: usize = 256;
const PREFETCH_MAX_TIME_PER_DIR: Duration = Duration::from_millis(8);

impl ScanHandle {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

#[derive(Debug, Default)]
struct BatchAccumulator {
    discovered_nodes: Vec<DiscoveredNode>,
    size_deltas: HashMap<NodeId, u64>,
    scanned_nodes: Vec<NodeId>,
    progress: Option<ProgressSnapshot>,
}

impl BatchAccumulator {
    fn new(options: ScanOptions) -> Self {
        Self {
            discovered_nodes: Vec::with_capacity(options.max_pending_nodes.min(256)),
            size_deltas: HashMap::with_capacity(options.max_pending_size_deltas.min(512)),
            scanned_nodes: Vec::with_capacity(options.max_pending_nodes.min(256)),
            progress: None,
        }
    }

    fn is_empty(&self) -> bool {
        self.discovered_nodes.is_empty()
            && self.size_deltas.is_empty()
            && self.scanned_nodes.is_empty()
            && self.progress.is_none()
    }

    fn should_flush(&self, options: ScanOptions, last_flush: Instant) -> bool {
        self.discovered_nodes.len() >= options.max_pending_nodes
            || self.size_deltas.len() >= options.max_pending_size_deltas
            || last_flush.elapsed() >= options.batch_flush_interval
    }

    fn into_batch(self) -> ScanBatch {
        ScanBatch {
            discovered_nodes: self.discovered_nodes,
            size_deltas: self.size_deltas.into_iter().collect(),
            scanned_nodes: self.scanned_nodes,
            progress: self.progress,
        }
    }
}

pub fn start_scan(
    path: PathBuf,
    scan_id: u64,
    options: ScanOptions,
    tx: Sender<ScanMessage>,
) -> ScanHandle {
    let cancel = Arc::new(AtomicBool::new(false));
    let thread_cancel = Arc::clone(&cancel);

    thread::spawn(move || {
        run_scan(path, scan_id, options, tx, thread_cancel);
    });

    ScanHandle { cancel }
}

fn run_scan(
    path: PathBuf,
    scan_id: u64,
    options: ScanOptions,
    tx: Sender<ScanMessage>,
    cancel: Arc<AtomicBool>,
) {
    let mut perf_stats = PerfStats::default();

    if !path.exists() {
        perf_stats.messages_sent += 1;
        let _ = tx.send(ScanMessage::Error {
            scan_id,
            message: format!("Path does not exist: {}", path.display()),
            perf_stats,
        });
        return;
    }

    let start = Instant::now();
    let root_name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string());
    let root_node = TreeStore::root_record(path.clone(), root_name);
    perf_stats.messages_sent += 1;
    let _ = tx.send(ScanMessage::Started {
        scan_id,
        path: path.clone(),
        root_node: root_node.clone(),
    });

    let mut shadow_tree = TreeStore::new();
    let root_id = shadow_tree.push_node(None, root_node);

    let mut parent_lookup = ParentLookup::new(path.clone(), root_id);

    let mut db = match options.cache_mode {
        CacheMode::Disabled => None,
        CacheMode::Enabled => ScanDb::new(&std::env::temp_dir().join("disk-map.db")).ok(),
    };
    let mut files_scanned = 0_u64;
    let mut dirs_scanned = 0_u64;
    let mut bytes_seen = 0_u64;
    let mut batch = BatchAccumulator::new(options);
    let mut last_flush = Instant::now();
    let mut last_seen_path = path.clone();

    let prefetch_perf = Arc::new(PrefetchPerfCounters::default());
    let walker_prefetch_perf = Arc::clone(&prefetch_perf);
    let walker = WalkDirGeneric::<ScanWalkState>::new(&path)
        .skip_hidden(false)
        .follow_links(false)
        .process_read_dir(move |_depth, _path, _state, children| {
            let dir_prefetch_start = Instant::now();
            let mut prefetched_files = 0usize;
            for child in children.iter_mut() {
                if prefetched_files >= PREFETCH_MAX_FILES_PER_DIR
                    || dir_prefetch_start.elapsed() >= PREFETCH_MAX_TIME_PER_DIR
                {
                    break;
                }
                let Ok(dir_entry) = child.as_mut() else {
                    continue;
                };
                if !dir_entry.file_type().is_file() {
                    continue;
                }
                prefetched_files += 1;

                let metadata_start = Instant::now();
                let metadata = match dir_entry.metadata() {
                    Ok(metadata) => metadata,
                    Err(err) => {
                        walker_prefetch_perf.metadata_ns.fetch_add(
                            metadata_start.elapsed().as_nanos() as u64,
                            Ordering::Relaxed,
                        );
                        dir_entry.client_state.prefetched_file = Some(Err(err.to_string()));
                        continue;
                    }
                };
                walker_prefetch_perf.metadata_ns.fetch_add(
                    metadata_start.elapsed().as_nanos() as u64,
                    Ordering::Relaxed,
                );

                let mtime = metadata
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);

                let size_start = Instant::now();
                let measured_size = size_on_disk_bytes(&metadata);
                walker_prefetch_perf.size_ns.fetch_add(
                    size_start.elapsed().as_nanos() as u64,
                    Ordering::Relaxed,
                );

                dir_entry.client_state.prefetched_file = Some(Ok(PrefetchedFileInfo {
                    metadata,
                    mtime,
                    measured_size,
                }));
            }
        })
        .into_iter()
        .filter_map(|e| e.ok());

    for entry in walker {
        if cancel.load(Ordering::Relaxed) {
            perf_stats.scan_elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
            flush_batch(
                &tx,
                scan_id,
                &mut batch,
                &mut perf_stats,
                &mut last_flush,
                files_scanned,
                dirs_scanned,
                bytes_seen,
                &last_seen_path,
            );
            perf_stats.messages_sent += 1;
            let _ = tx.send(ScanMessage::Cancelled { scan_id, perf_stats });
            return;
        }

        perf_stats.entries_seen += 1;
        let entry_path = entry.path();
        if entry_path == path {
            continue;
        }

        let entry_depth = entry.depth();
        let name = entry.file_name().to_string_lossy().to_string();
        let parent_id = parent_lookup.parent_id_for(
            entry_depth,
            entry.parent_path(),
            &entry_path,
            &mut perf_stats,
        );

        let file_type = entry.file_type();
        let mut node_error = None;
        let kind = if entry.path_is_symlink() || file_type.is_symlink() {
            NodeKind::Symlink
        } else if file_type.is_dir() {
            NodeKind::Dir
        } else if file_type.is_file() {
            NodeKind::File
        } else {
            NodeKind::Error
        };

        let size = if kind == NodeKind::File {
            match entry.client_state.prefetched_file {
                Some(Ok(prefetched)) => {
                    let size_start = Instant::now();
                    let size = measured_size_for_file(
                        &entry_path,
                        &prefetched.metadata,
                        prefetched.mtime,
                        prefetched.measured_size,
                        db.as_mut(),
                        &mut perf_stats,
                    );
                    perf_stats.size_measure_total_ms += size_start.elapsed().as_secs_f64() * 1000.0;
                    size
                }
                Some(Err(err)) => {
                    node_error = Some(err);
                    0
                }
                None => {
                    let metadata_start = Instant::now();
                    let metadata = match entry.metadata() {
                        Ok(metadata) => metadata,
                        Err(err) => {
                            perf_stats.metadata_total_ms += metadata_start.elapsed().as_secs_f64() * 1000.0;
                            node_error = Some(err.to_string());
                            let node_id = shadow_tree.len();
                            let node = NodeRecord {
                                name,
                                path: entry_path.to_path_buf(),
                                kind: NodeKind::Error,
                                size: 0,
                                scanned: true,
                                error: node_error.clone(),
                            };
                            shadow_tree.insert_node(node_id, Some(parent_id), node.clone());
                            perf_stats.nodes_discovered += 1;
                            batch.discovered_nodes.push(DiscoveredNode {
                                node_id,
                                parent_id,
                                node,
                            });
                            batch.scanned_nodes.push(node_id);
                            last_seen_path = entry_path;
                            if batch.should_flush(options, last_flush) {
                                flush_batch(
                                    &tx,
                                    scan_id,
                                    &mut batch,
                                    &mut perf_stats,
                                    &mut last_flush,
                                    files_scanned,
                                    dirs_scanned,
                                    bytes_seen,
                                    &last_seen_path,
                                );
                            }
                            continue;
                        }
                    };
                    perf_stats.metadata_total_ms += metadata_start.elapsed().as_secs_f64() * 1000.0;
                    let mtime = metadata
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let measured_size = size_on_disk_bytes(&metadata);
                    let size_start = Instant::now();
                    let size = measured_size_for_file(
                        &entry_path,
                        &metadata,
                        mtime,
                        measured_size,
                        db.as_mut(),
                        &mut perf_stats,
                    );
                    perf_stats.size_measure_total_ms += size_start.elapsed().as_secs_f64() * 1000.0;
                    size
                }
            }
        } else {
            0
        };

        let node_id = shadow_tree.len();
        let node = NodeRecord {
            name,
            path: entry_path.to_path_buf(),
            kind: if node_error.is_some() { NodeKind::Error } else { kind },
            size,
            scanned: matches!(kind, NodeKind::File | NodeKind::Symlink | NodeKind::Error) || node_error.is_some(),
            error: node_error,
        };

        shadow_tree.insert_node(node_id, Some(parent_id), node.clone());
        perf_stats.nodes_discovered += 1;
        batch.discovered_nodes.push(DiscoveredNode {
            node_id,
            parent_id,
            node,
        });

        if kind == NodeKind::Dir {
            parent_lookup.record_directory(
                entry_depth,
                entry.read_children_path.as_deref(),
                &entry_path,
                node_id,
            );
            dirs_scanned += 1;
        } else if kind == NodeKind::File {
            files_scanned += 1;
            bytes_seen += size;
            shadow_tree.apply_size_delta(parent_id, size);
            let mut current = Some(parent_id);
            while let Some(ancestor_id) = current {
                let entry = batch.size_deltas.entry(ancestor_id).or_insert(0);
                if *entry > 0 {
                    perf_stats.size_delta_merges += 1;
                }
                *entry += size;
                current = shadow_tree.node(ancestor_id).parent;
            }
        }

        if matches!(kind, NodeKind::File | NodeKind::Symlink | NodeKind::Error) {
            batch.scanned_nodes.push(node_id);
        }

        last_seen_path = entry_path;

        if batch.should_flush(options, last_flush) {
            flush_batch(
                &tx,
                scan_id,
                &mut batch,
                &mut perf_stats,
                &mut last_flush,
                files_scanned,
                dirs_scanned,
                bytes_seen,
                &last_seen_path,
            );
        }
    }

    for node_id in 0..shadow_tree.len() {
        if matches!(shadow_tree.node(node_id).kind, NodeKind::Dir) {
            shadow_tree.mark_scanned(node_id);
            batch.scanned_nodes.push(node_id);
        }
    }

    if let Some(db) = db.as_mut() {
        let _ = db.flush();
        perf_stats.db_flush_count += 1;
    }

    flush_batch(
        &tx,
        scan_id,
        &mut batch,
        &mut perf_stats,
        &mut last_flush,
        files_scanned,
        dirs_scanned,
        bytes_seen,
        &last_seen_path,
    );

    let elapsed = start.elapsed();
    perf_stats.metadata_total_ms += prefetch_perf.metadata_ns.load(Ordering::Relaxed) as f64 / 1_000_000.0;
    perf_stats.size_measure_total_ms += prefetch_perf.size_ns.load(Ordering::Relaxed) as f64 / 1_000_000.0;
    perf_stats.scan_elapsed_ms = elapsed.as_secs_f64() * 1000.0;
    eprintln!("Scan {scan_id} completed in {:?}", elapsed);

    let total_bytes = shadow_tree.node(root_id).size;
    perf_stats.messages_sent += 1;
    let _ = tx.send(ScanMessage::Finished {
        scan_id,
        total_bytes,
        perf_stats,
    });
}

struct ParentLookup {
    root_id: NodeId,
    directory_stack: Vec<NodeId>,
    directory_ids: HashMap<PathBuf, NodeId>,
}

impl ParentLookup {
    fn new(root_path: PathBuf, root_id: NodeId) -> Self {
        let mut directory_ids = HashMap::with_capacity(1024);
        directory_ids.insert(root_path.clone(), root_id);
        Self {
            root_id,
            directory_stack: vec![root_id],
            directory_ids,
        }
    }

    fn parent_id_for(
        &mut self,
        depth: usize,
        parent_path: &Path,
        entry_path: &Path,
        perf_stats: &mut PerfStats,
    ) -> NodeId {
        if depth == 0 {
            perf_stats.parent_stack_hits += 1;
            return self.root_id;
        }

        if self.directory_stack.len() > depth {
            self.directory_stack.truncate(depth);
        }

        if let Some(&parent_id) = self.directory_stack.get(depth.saturating_sub(1)) {
            perf_stats.parent_stack_hits += 1;
            return parent_id;
        }

        perf_stats.parent_lookup_fallbacks += 1;
        self.directory_ids
            .get(parent_path)
            .copied()
            .or_else(|| entry_path.parent().and_then(|parent| self.directory_ids.get(parent).copied()))
            .unwrap_or(self.root_id)
    }

    fn record_directory(
        &mut self,
        depth: usize,
        read_children_path: Option<&Path>,
        entry_path: &Path,
        node_id: NodeId,
    ) {
        if self.directory_stack.len() <= depth {
            self.directory_stack.resize(depth + 1, self.root_id);
        }
        self.directory_stack[depth] = node_id;

        if let Some(path) = read_children_path {
            self.directory_ids.insert(path.to_path_buf(), node_id);
        } else {
            self.directory_ids.insert(entry_path.to_path_buf(), node_id);
        }
    }
}

fn flush_batch(
    tx: &Sender<ScanMessage>,
    scan_id: u64,
    batch: &mut BatchAccumulator,
    perf_stats: &mut PerfStats,
    last_flush: &mut Instant,
    files_scanned: u64,
    dirs_scanned: u64,
    bytes_seen: u64,
    current_path: &Path,
) {
    if batch.is_empty() {
        return;
    }

    let flush_start = Instant::now();
    batch.progress = Some(ProgressSnapshot {
        files_scanned,
        dirs_scanned,
        bytes_seen,
        current_path: current_path.to_path_buf(),
    });
    let outgoing = std::mem::take(batch).into_batch();
    perf_stats.messages_sent += 1;
    perf_stats.batches_sent += 1;
    perf_stats.progress_snapshots_sent += 1;
    let _ = tx.send(ScanMessage::Batch {
        scan_id,
        batch: outgoing,
    });
    perf_stats.batch_flush_total_ms += flush_start.elapsed().as_secs_f64() * 1000.0;
    *last_flush = Instant::now();
}

fn measured_size_for_file(
    path: &Path,
    _metadata: &Metadata,
    mtime: u64,
    measured_size: u64,
    db: Option<&mut ScanDb>,
    perf_stats: &mut PerfStats,
) -> u64 {
    if let Some(db) = db {
        if let Some(cached_size) = db.get_cached(path, mtime) {
            perf_stats.db_cache_hits += 1;
            return cached_size;
        }
        perf_stats.db_cache_misses += 1;
        let _ = db.insert(path, measured_size, mtime);
    }
    measured_size
}

fn size_on_disk_bytes(metadata: &Metadata) -> u64 {
    #[cfg(unix)]
    {
        // st_blocks is reported in 512-byte units and reflects allocated disk usage,
        // which avoids treating sparse/cloned files as fully materialized bytes.
        let allocated = metadata.blocks().saturating_mul(512);
        if allocated > 0 || metadata.len() == 0 {
            return allocated;
        }
    }

    metadata.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::write;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("disk-map-{label}-{nanos}"))
    }

    #[test]
    fn started_to_finished_messages_describe_incremental_tree() {
        let root = TreeStore::root_record(PathBuf::from("/root"), "root".into());
        let messages = [
            ScanMessage::Started {
                scan_id: 1,
                path: PathBuf::from("/root"),
                root_node: root.clone(),
            },
            ScanMessage::Batch {
                scan_id: 1,
                batch: ScanBatch {
                    discovered_nodes: vec![DiscoveredNode {
                        node_id: 1,
                        parent_id: 0,
                        node: NodeRecord {
                            name: "child".into(),
                            path: PathBuf::from("/root/child"),
                            kind: NodeKind::File,
                            size: 42,
                            scanned: true,
                            error: None,
                        },
                    }],
                    size_deltas: vec![(0, 42)],
                    scanned_nodes: vec![1],
                    progress: None,
                },
            },
            ScanMessage::Finished {
                scan_id: 1,
                total_bytes: 42,
                perf_stats: PerfStats::default(),
            },
        ];

        assert!(matches!(messages.first(), Some(ScanMessage::Started { .. })));
        assert!(matches!(messages.get(1), Some(ScanMessage::Batch { .. })));
        assert!(matches!(
            messages.last(),
            Some(ScanMessage::Finished {
                total_bytes: 42,
                ..
            })
        ));
    }

    #[test]
    fn tree_size_delta_updates_all_ancestors() {
        let mut tree = TreeStore::new();
        let root = tree.add_node(None, "root".into(), PathBuf::from("/root"), NodeKind::Dir, 0);
        let child = tree.add_node(Some(root), "child".into(), PathBuf::from("/root/child"), NodeKind::Dir, 0);
        tree.add_node(Some(child), "file".into(), PathBuf::from("/root/child/file"), NodeKind::File, 42);

        tree.apply_size_delta(child, 42);

        assert_eq!(tree.node(root).size, 42);
        assert_eq!(tree.node(child).size, 42);
    }

    #[test]
    fn batch_accumulator_merges_size_deltas_by_node() {
        let options = ScanOptions::default();
        let mut batch = BatchAccumulator::new(options);

        *batch.size_deltas.entry(1).or_insert(0) += 10;
        *batch.size_deltas.entry(1).or_insert(0) += 32;
        *batch.size_deltas.entry(2).or_insert(0) += 5;

        let outgoing = batch.into_batch();
        let deltas: HashMap<NodeId, u64> = outgoing.size_deltas.into_iter().collect();

        assert_eq!(deltas.get(&1), Some(&42));
        assert_eq!(deltas.get(&2), Some(&5));
    }

    #[cfg(unix)]
    #[test]
    fn size_on_disk_should_follow_allocated_block_count() {
        let path = temp_path("allocated-blocks");
        write(&path, b"disk-map").unwrap();

        let metadata = std::fs::metadata(&path).unwrap();
        let measured = size_on_disk_bytes(&metadata);

        assert_eq!(measured, metadata.blocks().saturating_mul(512));

        let _ = std::fs::remove_file(path);
    }
}
