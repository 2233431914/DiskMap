use crate::db::ScanDb;
use crate::tree::{node_id_from_index, node_index, NodeId, NodeKind, NodeRecord, TreeStore};

use crossbeam_channel::{unbounded, Sender};
use jwalk::{ClientState, WalkDirGeneric};
use rustc_hash::FxHashMap;
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

#[derive(Debug, Clone)]
pub struct ScanOptions {
    pub batch_flush_interval: Duration,
    pub max_pending_nodes: usize,
    pub max_pending_size_deltas: usize,
    pub cache_mode: CacheMode,
    pub cache_path: Option<PathBuf>,
    pub exclude_patterns: Vec<String>,
    pub include_hidden: bool,
    /// Legacy compatibility field. Traversal is disabled for safety; callers
    /// requesting `true` receive the same non-following scan semantics.
    pub follow_symlinks: bool,
    pub stay_on_filesystem: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            batch_flush_interval: Duration::from_millis(33),
            max_pending_nodes: 2_048,
            max_pending_size_deltas: 4_096,
            cache_mode: CacheMode::Disabled,
            cache_path: None,
            exclude_patterns: Vec::new(),
            include_hidden: true,
            follow_symlinks: false,
            stay_on_filesystem: false,
        }
    }
}

pub fn parse_exclude_patterns(input: &str) -> Vec<String> {
    let mut patterns = Vec::new();
    for pattern in input
        .split([',', ';', '\n'])
        .map(str::trim)
        .filter(|pattern| !pattern.is_empty())
    {
        if !patterns.iter().any(|existing| existing == pattern) {
            patterns.push(pattern.to_string());
        }
    }
    patterns
}

pub fn size_basis_label() -> &'static str {
    #[cfg(unix)]
    {
        "Allocated size"
    }

    #[cfg(not(unix))]
    {
        "Apparent size"
    }
}

pub fn size_basis_detail() -> &'static str {
    #[cfg(unix)]
    {
        "Uses filesystem allocated blocks so sparse and virtual files do not count apparent bytes."
    }

    #[cfg(not(unix))]
    {
        "Uses apparent byte length reported by file metadata."
    }
}

#[derive(Debug, Clone, Default)]
pub struct PerfStats {
    pub messages_sent: u64,
    pub batches_sent: u64,
    pub entries_seen: u64,
    pub nodes_discovered: u64,
    pub files_scanned: u64,
    pub dirs_scanned: u64,
    pub size_delta_merges: u64,
    pub ancestor_size_delta_total_ms: f64,
    pub parent_stack_hits: u64,
    pub parent_lookup_fallbacks: u64,
    pub progress_snapshots_sent: u64,
    pub prefetched_files: u64,
    pub metadata_fallback_files: u64,
    pub metadata_total_ms: f64,
    pub mtime_total_ms: f64,
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
    pub total_files: Option<u64>,
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
    cached_mtime: Option<u64>,
    modified_secs: Option<u64>,
    measured_size: u64,
}

#[derive(Debug, Default)]
struct PrefetchPerfCounters {
    metadata_ns: AtomicU64,
    mtime_ns: AtomicU64,
    size_ns: AtomicU64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PrefetchBudget {
    max_files: usize,
    max_time: Duration,
}

const PREFETCH_SMALL_DIR_FILE_THRESHOLD: usize = 512;
const PREFETCH_MEDIUM_DIR_FILE_THRESHOLD: usize = 4_096;
const PREFETCH_LARGE_DIR_FILE_THRESHOLD: usize = 16_384;
const PREFETCH_HUGE_DIR_FILE_THRESHOLD: usize = 65_536;
const PREFETCH_MEDIUM_DIR_FILE_CAP: usize = 4_096;
const PREFETCH_LARGE_DIR_FILE_CAP: usize = 16_384;
const PREFETCH_HUGE_DIR_FILE_CAP: usize = 49_152;
const PREFETCH_GIANT_DIR_FILE_CAP: usize = 65_536;
const PREFETCH_SMALL_DIR_TIME_BUDGET: Duration = Duration::from_millis(12);
const PREFETCH_MEDIUM_DIR_TIME_BUDGET: Duration = Duration::from_millis(45);
const PREFETCH_LARGE_DIR_TIME_BUDGET: Duration = Duration::from_millis(110);
const PREFETCH_HUGE_DIR_TIME_BUDGET: Duration = Duration::from_millis(260);
const PREFETCH_GIANT_DIR_TIME_BUDGET: Duration = Duration::from_millis(380);

fn prefetch_budget_for_dir(file_count: usize) -> PrefetchBudget {
    if file_count == 0 {
        return PrefetchBudget {
            max_files: 0,
            max_time: Duration::ZERO,
        };
    }

    if file_count <= PREFETCH_SMALL_DIR_FILE_THRESHOLD {
        PrefetchBudget {
            max_files: file_count,
            max_time: PREFETCH_SMALL_DIR_TIME_BUDGET,
        }
    } else if file_count <= PREFETCH_MEDIUM_DIR_FILE_THRESHOLD {
        PrefetchBudget {
            max_files: file_count.min(PREFETCH_MEDIUM_DIR_FILE_CAP),
            max_time: PREFETCH_MEDIUM_DIR_TIME_BUDGET,
        }
    } else if file_count <= PREFETCH_LARGE_DIR_FILE_THRESHOLD {
        PrefetchBudget {
            max_files: file_count.min(PREFETCH_LARGE_DIR_FILE_CAP),
            max_time: PREFETCH_LARGE_DIR_TIME_BUDGET,
        }
    } else if file_count <= PREFETCH_HUGE_DIR_FILE_THRESHOLD {
        PrefetchBudget {
            max_files: file_count.min(PREFETCH_HUGE_DIR_FILE_CAP),
            max_time: PREFETCH_HUGE_DIR_TIME_BUDGET,
        }
    } else {
        PrefetchBudget {
            max_files: file_count.min(PREFETCH_GIANT_DIR_FILE_CAP),
            max_time: PREFETCH_GIANT_DIR_TIME_BUDGET,
        }
    }
}

impl ScanHandle {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

#[derive(Debug, Default)]
struct BatchAccumulator {
    discovered_nodes: Vec<DiscoveredNode>,
    size_deltas: FxHashMap<NodeId, u64>,
    scanned_nodes: Vec<NodeId>,
    progress: Option<ProgressSnapshot>,
}

#[derive(Debug, Clone)]
struct ScanCounters {
    files_scanned: u64,
    total_files: Option<u64>,
    dirs_scanned: u64,
    bytes_seen: u64,
    current_path: PathBuf,
}

impl ScanCounters {
    fn new(current_path: PathBuf, total_files: Option<u64>) -> Self {
        Self {
            files_scanned: 0,
            total_files,
            dirs_scanned: 0,
            bytes_seen: 0,
            current_path,
        }
    }

    fn progress_snapshot(&self) -> ProgressSnapshot {
        ProgressSnapshot {
            files_scanned: self.files_scanned,
            total_files: self.total_files,
            dirs_scanned: self.dirs_scanned,
            bytes_seen: self.bytes_seen,
            current_path: self.current_path.clone(),
        }
    }
}

#[derive(Debug, Clone)]
struct ScanIndex {
    parents: Vec<Option<NodeId>>,
    dir_node_ids: Vec<NodeId>,
    total_bytes: u64,
}

impl ScanIndex {
    fn new(root_id: NodeId) -> Self {
        Self {
            parents: vec![None],
            dir_node_ids: vec![root_id],
            total_bytes: 0,
        }
    }

    fn alloc_node(&mut self, parent_id: NodeId, kind: NodeKind) -> NodeId {
        let node_id = node_id_from_index(self.parents.len());
        self.parents.push(Some(parent_id));
        if matches!(kind, NodeKind::Dir) {
            self.dir_node_ids.push(node_id);
        }
        node_id
    }

    fn add_file_size(
        &mut self,
        parent_id: NodeId,
        size: u64,
        batch: &mut BatchAccumulator,
        perf_stats: &mut PerfStats,
    ) {
        self.total_bytes = self.total_bytes.saturating_add(size);

        let ancestor_delta_start = Instant::now();
        let mut current = Some(parent_id);
        while let Some(ancestor_id) = current {
            let entry = batch.size_deltas.entry(ancestor_id).or_insert(0);
            if *entry > 0 {
                perf_stats.size_delta_merges += 1;
            }
            *entry = entry.saturating_add(size);
            current = self.parent_of(ancestor_id);
        }
        perf_stats.ancestor_size_delta_total_ms +=
            ancestor_delta_start.elapsed().as_secs_f64() * 1000.0;
    }

    fn parent_of(&self, node_id: NodeId) -> Option<NodeId> {
        self.parents.get(node_index(node_id)).copied().flatten()
    }

    fn dir_node_ids(&self) -> &[NodeId] {
        &self.dir_node_ids
    }

    fn total_bytes(&self) -> u64 {
        self.total_bytes
    }
}

#[derive(Debug, Clone)]
struct ExcludeMatcher {
    patterns: Vec<String>,
}

impl ExcludeMatcher {
    fn new(patterns: Vec<String>) -> Self {
        Self {
            patterns: patterns
                .into_iter()
                .map(|pattern| pattern.replace('\\', "/").to_ascii_lowercase())
                .collect(),
        }
    }

    fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    fn matches_path(&self, path: &Path) -> bool {
        if self.patterns.is_empty() {
            return false;
        }

        let normalized_path = path
            .to_string_lossy()
            .replace('\\', "/")
            .to_ascii_lowercase();
        let components: Vec<String> = path
            .components()
            .filter_map(|component| component.as_os_str().to_str())
            .map(|component| component.to_ascii_lowercase())
            .collect();

        self.patterns.iter().any(|pattern| {
            if pattern.contains('/') || pattern.contains('\\') {
                if pattern.contains('*') {
                    wildcard_match(pattern, &normalized_path)
                } else {
                    normalized_path.contains(pattern)
                }
            } else {
                components
                    .iter()
                    .any(|component| wildcard_match(pattern, component))
            }
        })
    }
}

fn wildcard_match(pattern: &str, value: &str) -> bool {
    if !pattern.contains('*') {
        return value == pattern;
    }

    let parts: Vec<&str> = pattern.split('*').filter(|part| !part.is_empty()).collect();
    if parts.is_empty() {
        return true;
    }

    let mut remainder = value;
    for (index, part) in parts.iter().enumerate() {
        let Some(found_at) = remainder.find(part) else {
            return false;
        };
        if index == 0 && !pattern.starts_with('*') && found_at != 0 {
            return false;
        }
        remainder = &remainder[found_at + part.len()..];
    }

    pattern.ends_with('*') || value.ends_with(parts.last().copied().unwrap_or_default())
}

#[cfg(unix)]
fn root_device_id(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|metadata| metadata.dev())
}

#[cfg(not(unix))]
fn root_device_id(_path: &Path) -> Option<u64> {
    None
}

#[cfg(unix)]
fn metadata_is_on_device(metadata: &Metadata, device_id: u64) -> bool {
    metadata.dev() == device_id
}

#[cfg(not(unix))]
fn metadata_is_on_device(_metadata: &Metadata, _device_id: u64) -> bool {
    true
}

impl BatchAccumulator {
    fn new(options: &ScanOptions) -> Self {
        Self {
            discovered_nodes: Vec::with_capacity(options.max_pending_nodes.min(256)),
            size_deltas: FxHashMap::with_capacity_and_hasher(
                options.max_pending_size_deltas.min(512),
                Default::default(),
            ),
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

    fn should_flush(&self, options: &ScanOptions, last_flush: Instant) -> bool {
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

fn retain_scan_candidates(
    children: &mut Vec<jwalk::Result<jwalk::DirEntry<ScanWalkState>>>,
    exclude_matcher: &ExcludeMatcher,
    same_filesystem_device: Option<u64>,
) {
    if exclude_matcher.is_empty() && same_filesystem_device.is_none() {
        return;
    }

    children.retain(|child| {
        let Ok(entry) = child.as_ref() else {
            return true;
        };
        if !exclude_matcher.is_empty() && exclude_matcher.matches_path(&entry.path()) {
            return false;
        }
        if let Some(device_id) = same_filesystem_device {
            return entry
                .metadata()
                .map(|metadata| metadata_is_on_device(&metadata, device_id))
                .unwrap_or(true);
        }
        true
    });
}

pub fn scan_path_to_tree(path: PathBuf, options: ScanOptions) -> anyhow::Result<TreeStore> {
    let (tx, rx) = unbounded();
    run_scan(
        path.clone(),
        0,
        options,
        tx,
        Arc::new(AtomicBool::new(false)),
    );

    let mut tree = TreeStore::new();
    while let Ok(message) = rx.recv() {
        match message {
            ScanMessage::Started {
                path, root_node, ..
            } => {
                tree.clear();
                tree.push_node(None, root_node);
                tree.set_root_path(path);
            }
            ScanMessage::Batch { batch, .. } => {
                for discovered in batch.discovered_nodes {
                    tree.insert_node(
                        discovered.node_id,
                        Some(discovered.parent_id),
                        discovered.node,
                    );
                }
                for (node_id, delta) in batch.size_deltas {
                    if node_index(node_id) < tree.len() {
                        tree.apply_direct_size_delta(node_id, delta);
                    }
                }
                for node_id in batch.scanned_nodes {
                    if node_index(node_id) < tree.len() {
                        tree.mark_scanned(node_id);
                    }
                }
            }
            ScanMessage::Finished { .. } => return Ok(tree),
            ScanMessage::Cancelled { .. } => anyhow::bail!("scan cancelled"),
            ScanMessage::Error { message, .. } => anyhow::bail!(message),
        }
    }

    anyhow::bail!("scan ended without a result for {}", path.display())
}

fn run_scan(
    path: PathBuf,
    scan_id: u64,
    options: ScanOptions,
    tx: Sender<ScanMessage>,
    cancel: Arc<AtomicBool>,
) {
    let requested_path = path;
    let had_trailing_separator = requested_path
        .as_os_str()
        .as_encoded_bytes()
        .last()
        .is_some_and(|byte| *byte == std::path::MAIN_SEPARATOR as u8);
    let path: PathBuf = requested_path.components().collect();
    let mut perf_stats = PerfStats::default();

    let root_metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) => {
            perf_stats.messages_sent += 1;
            let _ = tx.send(ScanMessage::Error {
                scan_id,
                message: format!("Cannot inspect {}: {error}", path.display()),
                perf_stats,
            });
            return;
        }
    };

    if !path.exists() && !root_metadata.file_type().is_symlink() {
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
    let root_file_type = root_metadata.file_type();
    if had_trailing_separator && root_file_type.is_symlink() {
        perf_stats.messages_sent += 1;
        let _ = tx.send(ScanMessage::Error {
            scan_id,
            message: format!(
                "Refusing to scan symbolic link root with a trailing separator: {}",
                requested_path.display()
            ),
            perf_stats,
        });
        return;
    }

    let root_node = if root_file_type.is_dir() {
        TreeStore::root_record(root_name)
    } else if root_file_type.is_file() {
        NodeRecord {
            name: root_name,
            kind: NodeKind::File,
            size: size_on_disk_bytes(&root_metadata),
            modified_secs: modified_secs_for_metadata(&root_metadata),
            scanned: true,
            error: None,
        }
    } else if root_file_type.is_symlink() {
        NodeRecord {
            name: root_name,
            kind: NodeKind::Symlink,
            size: 0,
            modified_secs: None,
            scanned: true,
            error: None,
        }
    } else {
        perf_stats.messages_sent += 1;
        let _ = tx.send(ScanMessage::Error {
            scan_id,
            message: format!("Unsupported file type: {}", path.display()),
            perf_stats,
        });
        return;
    };
    let root_size = root_node.size;
    perf_stats.messages_sent += 1;
    let _ = tx.send(ScanMessage::Started {
        scan_id,
        path: path.clone(),
        root_node,
    });

    if !root_file_type.is_dir() {
        perf_stats.entries_seen = 1;
        perf_stats.nodes_discovered = 1;
        perf_stats.files_scanned = u64::from(root_file_type.is_file());
        perf_stats.scan_elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        perf_stats.messages_sent += 1;
        let _ = tx.send(ScanMessage::Finished {
            scan_id,
            total_bytes: root_size,
            perf_stats,
        });
        return;
    }

    let root_id = node_id_from_index(0);
    let mut scan_index = ScanIndex::new(root_id);

    let mut parent_lookup = ParentLookup::new(path.clone(), root_id);

    let mut db = match (options.cache_mode, options.cache_path.as_deref()) {
        (CacheMode::Enabled, Some(path)) => ScanDb::new(path).ok(),
        _ => None,
    };
    let mut counters = ScanCounters::new(path.clone(), None);
    let mut batch = BatchAccumulator::new(&options);
    let mut last_flush = Instant::now();

    let prefetch_perf = Arc::new(PrefetchPerfCounters::default());
    let walker_prefetch_perf = Arc::clone(&prefetch_perf);
    let cache_enabled = db.is_some();
    let exclude_matcher = ExcludeMatcher::new(options.exclude_patterns.clone());
    let same_filesystem_device = if options.stay_on_filesystem {
        root_device_id(&path)
    } else {
        None
    };
    let include_hidden = options.include_hidden;
    // Keep links visible without traversal until link-cycle and path semantics
    // are explicitly defined.
    let follow_symlinks = false;
    let walker = WalkDirGeneric::<ScanWalkState>::new(&path)
        .skip_hidden(!include_hidden)
        .follow_links(follow_symlinks)
        .process_read_dir(move |_depth, _path, _state, children| {
            retain_scan_candidates(children, &exclude_matcher, same_filesystem_device);

            let file_count = children
                .iter()
                .filter_map(|child| child.as_ref().ok())
                .filter(|entry| entry.file_type().is_file())
                .count();
            let budget = prefetch_budget_for_dir(file_count);
            if budget.max_files == 0 {
                return;
            }

            let dir_prefetch_start = Instant::now();
            let mut prefetched_files = 0usize;
            for child in children.iter_mut() {
                if prefetched_files >= budget.max_files
                    || dir_prefetch_start.elapsed() >= budget.max_time
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

                let mtime_start = Instant::now();
                let modified_secs = modified_secs_for_metadata(&metadata);
                walker_prefetch_perf
                    .mtime_ns
                    .fetch_add(mtime_start.elapsed().as_nanos() as u64, Ordering::Relaxed);
                let cached_mtime = cached_mtime_for_modified_secs(modified_secs, cache_enabled);

                let size_start = Instant::now();
                let measured_size = size_on_disk_bytes(&metadata);
                walker_prefetch_perf
                    .size_ns
                    .fetch_add(size_start.elapsed().as_nanos() as u64, Ordering::Relaxed);

                dir_entry.client_state.prefetched_file = Some(Ok(PrefetchedFileInfo {
                    metadata,
                    cached_mtime,
                    modified_secs,
                    measured_size,
                }));
            }
        })
        .into_iter();

    for entry_result in walker {
        if cancel.load(Ordering::Relaxed) {
            perf_stats.scan_elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
            flush_batch(
                &tx,
                scan_id,
                &mut batch,
                &mut perf_stats,
                &mut last_flush,
                &counters,
            );
            perf_stats.messages_sent += 1;
            let _ = tx.send(ScanMessage::Cancelled {
                scan_id,
                perf_stats,
            });
            return;
        }

        let entry = match entry_result {
            Ok(entry) => entry,
            Err(error) => {
                perf_stats.entries_seen += 1;
                let error_path = error.path().unwrap_or(&path).to_path_buf();
                let parent_path = error_path.parent().unwrap_or(&path);
                let parent_id = parent_lookup.parent_id_for(
                    error.depth(),
                    parent_path,
                    &error_path,
                    &mut perf_stats,
                );
                let name = error_path
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| "Scan error".to_string());
                let node = NodeRecord {
                    name,
                    kind: NodeKind::Error,
                    size: 0,
                    modified_secs: None,
                    scanned: true,
                    error: Some(error.to_string()),
                };
                let node_id = scan_index.alloc_node(parent_id, node.kind);
                perf_stats.nodes_discovered += 1;
                batch.discovered_nodes.push(DiscoveredNode {
                    node_id,
                    parent_id,
                    node,
                });
                batch.scanned_nodes.push(node_id);
                counters.current_path = error_path;
                if batch.should_flush(&options, last_flush) {
                    flush_batch(
                        &tx,
                        scan_id,
                        &mut batch,
                        &mut perf_stats,
                        &mut last_flush,
                        &counters,
                    );
                }
                continue;
            }
        };

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
        let mut node_error = entry.read_children_error.as_ref().map(ToString::to_string);
        let mut modified_secs = None;
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
                    perf_stats.prefetched_files += 1;
                    modified_secs = prefetched.modified_secs;
                    let size_start = Instant::now();
                    let size = measured_size_for_file(
                        &entry_path,
                        &prefetched.metadata,
                        prefetched.cached_mtime,
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
                    perf_stats.metadata_fallback_files += 1;
                    let metadata_start = Instant::now();
                    let metadata = match entry.metadata() {
                        Ok(metadata) => metadata,
                        Err(err) => {
                            perf_stats.metadata_total_ms +=
                                metadata_start.elapsed().as_secs_f64() * 1000.0;
                            node_error = Some(err.to_string());
                            counters.files_scanned += 1;
                            let node = NodeRecord {
                                name,
                                kind: NodeKind::Error,
                                size: 0,
                                modified_secs: None,
                                scanned: true,
                                error: node_error.clone(),
                            };
                            let node_id = scan_index.alloc_node(parent_id, node.kind);
                            perf_stats.nodes_discovered += 1;
                            batch.discovered_nodes.push(DiscoveredNode {
                                node_id,
                                parent_id,
                                node,
                            });
                            batch.scanned_nodes.push(node_id);
                            counters.current_path = entry_path;
                            if batch.should_flush(&options, last_flush) {
                                flush_batch(
                                    &tx,
                                    scan_id,
                                    &mut batch,
                                    &mut perf_stats,
                                    &mut last_flush,
                                    &counters,
                                );
                            }
                            continue;
                        }
                    };
                    perf_stats.metadata_total_ms += metadata_start.elapsed().as_secs_f64() * 1000.0;
                    let mtime_start = Instant::now();
                    modified_secs = modified_secs_for_metadata(&metadata);
                    perf_stats.mtime_total_ms += mtime_start.elapsed().as_secs_f64() * 1000.0;
                    let cached_mtime = cached_mtime_for_modified_secs(modified_secs, db.is_some());
                    let measured_size = size_on_disk_bytes(&metadata);
                    let size_start = Instant::now();
                    let size = measured_size_for_file(
                        &entry_path,
                        &metadata,
                        cached_mtime,
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

        if kind == NodeKind::Dir {
            let is_terminal_error = node_error.is_some();
            let node = NodeRecord {
                name,
                kind: if node_error.is_some() {
                    NodeKind::Error
                } else {
                    kind
                },
                size,
                modified_secs: None,
                scanned: is_terminal_error,
                error: node_error,
            };
            let is_directory_node = node.kind == NodeKind::Dir;
            let node_id = scan_index.alloc_node(parent_id, node.kind);
            perf_stats.nodes_discovered += 1;
            batch.discovered_nodes.push(DiscoveredNode {
                node_id,
                parent_id,
                node,
            });
            if is_directory_node {
                parent_lookup.record_directory(
                    entry_depth,
                    entry.read_children_path.as_deref(),
                    &entry_path,
                    node_id,
                );
            } else {
                batch.scanned_nodes.push(node_id);
            }
            counters.dirs_scanned += 1;
        } else if kind == NodeKind::File {
            counters.files_scanned += 1;
            counters.bytes_seen += size;
            scan_index.add_file_size(parent_id, size, &mut batch, &mut perf_stats);

            let node = NodeRecord {
                name,
                kind: if node_error.is_some() {
                    NodeKind::Error
                } else {
                    kind
                },
                size,
                modified_secs,
                scanned: true,
                error: node_error,
            };
            let node_id = scan_index.alloc_node(parent_id, node.kind);
            perf_stats.nodes_discovered += 1;
            batch.discovered_nodes.push(DiscoveredNode {
                node_id,
                parent_id,
                node,
            });
            batch.scanned_nodes.push(node_id);
        } else {
            let node = NodeRecord {
                name,
                kind: if node_error.is_some() {
                    NodeKind::Error
                } else {
                    kind
                },
                size,
                modified_secs,
                scanned: true,
                error: node_error,
            };
            let node_id = scan_index.alloc_node(parent_id, node.kind);
            perf_stats.nodes_discovered += 1;
            batch.discovered_nodes.push(DiscoveredNode {
                node_id,
                parent_id,
                node,
            });
            batch.scanned_nodes.push(node_id);
        }

        counters.current_path = entry_path;

        if batch.should_flush(&options, last_flush) {
            flush_batch(
                &tx,
                scan_id,
                &mut batch,
                &mut perf_stats,
                &mut last_flush,
                &counters,
            );
        }
    }

    for &node_id in scan_index.dir_node_ids() {
        batch.scanned_nodes.push(node_id);
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
        &counters,
    );

    let elapsed = start.elapsed();
    perf_stats.files_scanned = counters.files_scanned;
    perf_stats.dirs_scanned = counters.dirs_scanned;
    perf_stats.metadata_total_ms +=
        prefetch_perf.metadata_ns.load(Ordering::Relaxed) as f64 / 1_000_000.0;
    perf_stats.mtime_total_ms +=
        prefetch_perf.mtime_ns.load(Ordering::Relaxed) as f64 / 1_000_000.0;
    perf_stats.size_measure_total_ms +=
        prefetch_perf.size_ns.load(Ordering::Relaxed) as f64 / 1_000_000.0;
    perf_stats.scan_elapsed_ms = elapsed.as_secs_f64() * 1000.0;
    if std::env::var_os("DISKMAP_SCAN_TRACE").is_some() {
        eprintln!("Scan {scan_id} completed in {:?}", elapsed);
    }

    let total_bytes = scan_index.total_bytes();
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
    directory_ids: FxHashMap<PathBuf, NodeId>,
}

impl ParentLookup {
    fn new(root_path: PathBuf, root_id: NodeId) -> Self {
        let mut directory_ids = FxHashMap::with_capacity_and_hasher(1024, Default::default());
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
            .or_else(|| {
                entry_path
                    .parent()
                    .and_then(|parent| self.directory_ids.get(parent).copied())
            })
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
    counters: &ScanCounters,
) {
    if batch.is_empty() {
        return;
    }

    let flush_start = Instant::now();
    batch.progress = Some(counters.progress_snapshot());
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
    cached_mtime: Option<u64>,
    measured_size: u64,
    db: Option<&mut ScanDb>,
    perf_stats: &mut PerfStats,
) -> u64 {
    if let Some(db) = db {
        let Some(mtime) = cached_mtime else {
            perf_stats.db_cache_misses += 1;
            let _ = db.insert(path, measured_size, 0);
            return measured_size;
        };
        if should_bypass_cached_size(measured_size) {
            perf_stats.db_cache_misses += 1;
            let _ = db.insert(path, measured_size, mtime);
            return measured_size;
        }
        if let Some(cached_size) = db.get_cached(path, mtime) {
            perf_stats.db_cache_hits += 1;
            return cached_size;
        }
        perf_stats.db_cache_misses += 1;
        let _ = db.insert(path, measured_size, mtime);
    }
    measured_size
}

#[cfg(unix)]
fn should_bypass_cached_size(measured_size: u64) -> bool {
    measured_size == 0
}

#[cfg(not(unix))]
fn should_bypass_cached_size(_measured_size: u64) -> bool {
    false
}

fn modified_secs_for_metadata(metadata: &Metadata) -> Option<u64> {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
}

fn cached_mtime_for_modified_secs(modified_secs: Option<u64>, cache_enabled: bool) -> Option<u64> {
    cache_enabled.then_some(modified_secs.unwrap_or(0))
}

fn size_on_disk_bytes(metadata: &Metadata) -> u64 {
    #[cfg(unix)]
    {
        // st_blocks is reported in 512-byte units and reflects allocated disk usage,
        // including 0 for sparse and virtual files such as /proc/kcore.
        metadata.blocks().saturating_mul(512)
    }

    #[cfg(not(unix))]
    {
        metadata.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::write;
    #[cfg(unix)]
    use std::fs::File;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("disk-map-{label}-{nanos}"))
    }

    #[test]
    fn started_to_finished_messages_describe_batched_tree() {
        let root = TreeStore::root_record("root".into());
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
                            kind: NodeKind::File,
                            size: 42,
                            modified_secs: None,
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

        assert!(matches!(
            messages.first(),
            Some(ScanMessage::Started { .. })
        ));
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
        let root = tree.add_node(None, "root".into(), NodeKind::Dir, 0);
        tree.set_root_path(PathBuf::from("/root"));
        let child = tree.add_node(Some(root), "child".into(), NodeKind::Dir, 0);
        tree.add_node(Some(child), "file".into(), NodeKind::File, 42);

        tree.apply_size_delta(child, 42);

        assert_eq!(tree.node(root).size, 42);
        assert_eq!(tree.node(child).size, 42);
    }

    #[test]
    fn batch_accumulator_merges_size_deltas_by_node() {
        let options = ScanOptions::default();
        let mut batch = BatchAccumulator::new(&options);

        *batch.size_deltas.entry(1).or_insert(0) += 10;
        *batch.size_deltas.entry(1).or_insert(0) += 32;
        *batch.size_deltas.entry(2).or_insert(0) += 5;

        let outgoing = batch.into_batch();
        let deltas: FxHashMap<NodeId, u64> = outgoing.size_deltas.into_iter().collect();

        assert_eq!(deltas.get(&1), Some(&42));
        assert_eq!(deltas.get(&2), Some(&5));
    }

    #[test]
    fn default_scan_options_keep_current_safe_scan_behavior() {
        let options = ScanOptions::default();

        assert!(options.include_hidden);
        assert!(!options.follow_symlinks);
        assert!(!options.stay_on_filesystem);
    }

    #[test]
    fn size_basis_label_describes_current_measurement_mode() {
        #[cfg(unix)]
        assert_eq!(size_basis_label(), "Allocated size");

        #[cfg(not(unix))]
        assert_eq!(size_basis_label(), "Apparent size");

        assert!(!size_basis_detail().is_empty());
    }

    #[test]
    fn scan_path_to_tree_returns_complete_tree_for_directory() {
        let dir = temp_path("sync-tree");
        std::fs::create_dir_all(&dir).unwrap();
        write(dir.join("file.txt"), b"disk-map").unwrap();

        let tree = scan_path_to_tree(dir.clone(), ScanOptions::default()).unwrap();

        assert!(tree.root.is_some());
        assert_eq!(
            tree.node(tree.root.unwrap()).name,
            dir.file_name().unwrap().to_string_lossy()
        );
        assert!(!tree.is_empty());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn scan_path_to_tree_returns_file_as_root() {
        let file_path = temp_path("single-file-root");
        write(&file_path, b"disk-map").unwrap();
        let expected_size = size_on_disk_bytes(&std::fs::metadata(&file_path).unwrap());

        let mut tree = scan_path_to_tree(file_path.clone(), ScanOptions::default()).unwrap();
        let root = tree.root.expect("file scan should create a root");

        assert_eq!(tree.node(root).kind, NodeKind::File);
        assert_eq!(tree.node(root).size, expected_size);
        assert_eq!(tree.node_real_path(root), Some(file_path.clone()));

        let _ = std::fs::remove_file(file_path);
    }

    #[test]
    fn scan_path_to_tree_keeps_small_files_as_real_nodes() {
        let dir = temp_path("small-files-lossless");
        std::fs::create_dir_all(&dir).unwrap();
        write(dir.join("alpha.txt"), b"a").unwrap();
        write(dir.join("beta.txt"), b"b").unwrap();

        let tree = scan_path_to_tree(dir.clone(), ScanOptions::default()).unwrap();

        assert!(tree.nodes.iter().any(|node| node.name == "alpha.txt"));
        assert!(tree.nodes.iter().any(|node| node.name == "beta.txt"));
        assert!(!tree
            .nodes
            .iter()
            .any(|node| matches!(node.kind, NodeKind::Aggregate)));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn scan_does_not_follow_symlinks_even_when_legacy_option_is_true() {
        use std::os::unix::fs::symlink;

        let root = temp_path("symlink-root");
        let target = temp_path("symlink-target");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        write(target.join("payload.bin"), b"payload").unwrap();
        symlink(&target, root.join("link")).unwrap();
        let options = ScanOptions {
            follow_symlinks: true,
            ..ScanOptions::default()
        };

        let tree = scan_path_to_tree(root.clone(), options).unwrap();

        assert!(tree
            .nodes
            .iter()
            .any(|node| node.name == "link" && node.kind == NodeKind::Symlink));
        assert!(!tree.nodes.iter().any(|node| node.name == "payload.bin"));

        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(target);
    }

    #[cfg(unix)]
    #[test]
    fn scan_does_not_follow_symlink_root_with_trailing_slash() {
        use std::os::unix::fs::symlink;

        let root = temp_path("trailing-slash-symlink-root");
        let target = temp_path("trailing-slash-symlink-target");
        std::fs::create_dir_all(&target).unwrap();
        write(target.join("payload.bin"), b"payload").unwrap();
        symlink(&target, &root).unwrap();
        let path_with_slash = PathBuf::from(format!("{}/", root.display()));

        let error = scan_path_to_tree(path_with_slash, ScanOptions::default())
            .expect_err("trailing-slash symlink roots should be rejected");

        assert!(error
            .to_string()
            .contains("symbolic link root with a trailing separator"));

        let _ = std::fs::remove_file(root);
        let _ = std::fs::remove_dir_all(target);
    }

    #[cfg(unix)]
    #[test]
    fn scan_rejects_unsupported_special_file_root() {
        let error = scan_path_to_tree(PathBuf::from("/dev/null"), ScanOptions::default())
            .expect_err("device roots should not be reported as symlinks");

        assert!(error.to_string().contains("Unsupported file type"));
    }

    #[cfg(unix)]
    #[test]
    fn scan_reports_directory_read_errors_when_permissions_allow_testing_them() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_path("permission-error");
        let blocked = root.join("blocked");
        std::fs::create_dir_all(&blocked).unwrap();
        write(blocked.join("hidden.txt"), b"hidden").unwrap();
        let original_mode = std::fs::metadata(&blocked).unwrap().permissions().mode();
        std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o000)).unwrap();

        if std::fs::read_dir(&blocked).is_ok() {
            std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(original_mode))
                .unwrap();
            let _ = std::fs::remove_dir_all(root);
            return;
        }

        let tree = scan_path_to_tree(root.clone(), ScanOptions::default()).unwrap();
        std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(original_mode)).unwrap();

        assert!(tree.nodes.iter().any(|node| {
            node.name == "blocked"
                && node.kind == NodeKind::Error
                && node.error.is_some()
                && node.scanned
        }));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn scan_progress_does_not_precount_total_files() {
        let dir = temp_path("no-precount-total-files");
        let nested = dir.join("nested");
        let ignored = dir.join("ignored");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir_all(&ignored).unwrap();
        write(dir.join("root.txt"), b"root").unwrap();
        write(nested.join("child.txt"), b"child").unwrap();
        write(dir.join(".hidden.txt"), b"hidden").unwrap();
        write(dir.join("skip.tmp"), b"skip").unwrap();
        write(ignored.join("ignored.txt"), b"ignored").unwrap();
        let (tx, rx) = unbounded();
        let options = ScanOptions {
            include_hidden: false,
            exclude_patterns: vec!["skip.tmp".into(), "ignored".into()],
            ..ScanOptions::default()
        };

        run_scan(
            dir.clone(),
            1,
            options,
            tx,
            Arc::new(AtomicBool::new(false)),
        );

        let mut latest_progress = None;
        let mut finished_stats = None;
        while let Ok(message) = rx.try_recv() {
            match message {
                ScanMessage::Batch { batch, .. } => {
                    latest_progress = batch.progress.or(latest_progress);
                }
                ScanMessage::Finished { perf_stats, .. } => {
                    finished_stats = Some(perf_stats);
                }
                ScanMessage::Started { .. } => {}
                ScanMessage::Cancelled { .. } | ScanMessage::Error { .. } => {
                    panic!("test scan should finish successfully");
                }
            }
        }

        let progress = latest_progress.expect("scan should emit progress");
        assert_eq!(progress.total_files, None);
        assert_eq!(progress.files_scanned, 2);
        assert_eq!(finished_stats.expect("scan should finish").files_scanned, 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn scan_path_to_tree_records_file_modified_time_when_available() {
        let dir = temp_path("sync-mtime");
        std::fs::create_dir_all(&dir).unwrap();
        write(dir.join("large.bin"), vec![1_u8; 16 * 1024 + 1]).unwrap();

        let tree = scan_path_to_tree(dir.clone(), ScanOptions::default()).unwrap();
        let file = tree
            .nodes
            .iter()
            .find(|node| node.name == "large.bin")
            .expect("large file node");

        assert!(file.modified_secs.is_some());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn scan_path_to_tree_uses_explicit_sqlite_cache_path() {
        let dir = temp_path("sync-cache-root");
        std::fs::create_dir_all(&dir).unwrap();
        write(dir.join("large.bin"), vec![1_u8; 16 * 1024 + 1]).unwrap();
        let cache_root = temp_path("sync-cache-dir");
        let cache_path = cache_root.join("nested").join("disk-map-cache.db");

        let options = ScanOptions {
            cache_mode: CacheMode::Enabled,
            cache_path: Some(cache_path.clone()),
            ..ScanOptions::default()
        };

        let tree = scan_path_to_tree(dir.clone(), options).unwrap();

        assert!(tree.root.is_some());
        assert!(cache_path.exists());

        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(cache_root);
    }

    #[test]
    fn parse_exclude_patterns_trims_splits_and_deduplicates() {
        let patterns = parse_exclude_patterns(".git, node_modules; target\n.git");

        assert_eq!(patterns, vec![".git", "node_modules", "target"]);
    }

    #[test]
    fn exclude_matcher_matches_components_paths_and_wildcards() {
        let matcher = ExcludeMatcher::new(parse_exclude_patterns(".git,Library/Caches,*.tmp"));

        assert!(matcher.matches_path(Path::new("/repo/.git/config")));
        assert!(matcher.matches_path(Path::new("/Users/me/Library/Caches/app/file")));
        assert!(matcher.matches_path(Path::new("/tmp/build.tmp")));
        assert!(!matcher.matches_path(Path::new("/repo/src/targeted/file")));
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

    #[cfg(unix)]
    #[test]
    fn sparse_files_do_not_count_apparent_size_as_disk_usage() {
        let path = temp_path("sparse-blocks");
        let file = File::create(&path).unwrap();
        file.set_len(128 * 1024 * 1024).unwrap();

        let metadata = std::fs::metadata(&path).unwrap();
        let measured = size_on_disk_bytes(&metadata);

        assert_eq!(measured, metadata.blocks().saturating_mul(512));
        if metadata.blocks() == 0 {
            assert_eq!(metadata.len(), 128 * 1024 * 1024);
            assert_eq!(measured, 0);
        }

        let _ = std::fs::remove_file(path);
    }

    #[cfg(unix)]
    #[test]
    fn zero_allocated_size_bypasses_stale_sqlite_cache_entry() {
        let file_path = temp_path("zero-allocated-cache-file");
        write(&file_path, b"metadata source").unwrap();
        let metadata = std::fs::metadata(&file_path).unwrap();
        let cache_path = temp_path("zero-allocated-cache-db");
        let mut db = ScanDb::new(&cache_path).unwrap();
        db.insert(&file_path, 140_737_471_598_592, 7).unwrap();
        db.flush().unwrap();
        let mut stats = PerfStats::default();

        let measured =
            measured_size_for_file(&file_path, &metadata, Some(7), 0, Some(&mut db), &mut stats);

        assert_eq!(measured, 0);
        assert_eq!(db.get_cached(&file_path, 7), Some(0));
        assert_eq!(stats.db_cache_hits, 0);
        assert_eq!(stats.db_cache_misses, 1);

        let _ = std::fs::remove_file(file_path);
        let _ = std::fs::remove_file(cache_path);
    }

    #[cfg(unix)]
    #[test]
    fn root_device_id_matches_metadata_device() {
        let path = temp_path("root-device");
        write(&path, b"disk-map").unwrap();

        let metadata = std::fs::metadata(&path).unwrap();

        assert_eq!(root_device_id(&path), Some(metadata.dev()));
        assert!(metadata_is_on_device(&metadata, metadata.dev()));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn prefetch_budget_prefetches_all_small_directories() {
        let budget = prefetch_budget_for_dir(128);

        assert_eq!(
            budget,
            PrefetchBudget {
                max_files: 128,
                max_time: PREFETCH_SMALL_DIR_TIME_BUDGET,
            }
        );
    }

    #[test]
    fn prefetch_budget_scales_up_for_large_directories() {
        let budget = prefetch_budget_for_dir(10_000);

        assert_eq!(
            budget,
            PrefetchBudget {
                max_files: 10_000,
                max_time: PREFETCH_LARGE_DIR_TIME_BUDGET,
            }
        );
    }

    #[test]
    fn prefetch_budget_expands_for_huge_directories() {
        let budget = prefetch_budget_for_dir(40_000);

        assert_eq!(
            budget,
            PrefetchBudget {
                max_files: 40_000,
                max_time: PREFETCH_HUGE_DIR_TIME_BUDGET,
            }
        );
    }

    #[test]
    fn prefetch_budget_caps_giant_directories() {
        let budget = prefetch_budget_for_dir(100_000);

        assert_eq!(
            budget,
            PrefetchBudget {
                max_files: PREFETCH_GIANT_DIR_FILE_CAP,
                max_time: PREFETCH_GIANT_DIR_TIME_BUDGET,
            }
        );
    }
}
