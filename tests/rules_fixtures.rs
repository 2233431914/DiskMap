//! Integration tests for the rule engine against on-disk fixture
//! trees. These run as separate integration tests (not unit tests)
//! to keep them honest about the public API of the `rules` module.
//!
//! Fixtures are created in a per-test temp directory using only
//! `std::fs` (no `tempfile` dep). mtime is set via
//! `std::fs::File::set_modified` (stable since Rust 1.75).

use disk_map::rules::{
    default_ruleset, evaluate_rules, RuleContext, RuleSet, RULES_FORMAT_VERSION,
};
use disk_map::tree::{NodeKind, TreeStore};
use std::fs::{self, File};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn unique_temp_dir() -> std::path::PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    let p = std::env::temp_dir()
        .join(format!("disk-map-rules-fixture-{pid}-{nanos}-{n}"));
    fs::create_dir_all(&p).expect("temp dir should be creatable");
    p
}

/// Build a fixture tree:
///
///   root/
///   ├── big.bin               (1.5 GB, mtime now)
///   ├── small.txt             (1 KB)
///   ├── old_log.log           (500 MB, mtime 2 years ago)
///   ├── fresh_log.log         (500 MB, mtime 10 days ago)
///   ├── .hidden               (10 KB)
///   ├── recent_normal.bin     (50 MB, mtime 5 days ago)
///   └── sub/
///       ├── nested_big.bin    (1.2 GB, mtime 6 months ago)
///       └── nested_small.txt  (1 KB)
///
/// Returns (root_path, now_secs).
fn build_default_fixture() -> (std::path::PathBuf, u64) {
    let dir = unique_temp_dir();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    const DAY: u64 = 24 * 60 * 60;

    // big.bin: 1.5 GB, now
    write_file_with_size_and_mtime(
        &dir.join("big.bin"),
        1_500_000_000,
        SystemTime::UNIX_EPOCH + Duration::from_secs(now - 60),
    );

    // small.txt: 1 KB, now
    write_file_with_size_and_mtime(
        &dir.join("small.txt"),
        1024,
        SystemTime::UNIX_EPOCH + Duration::from_secs(now - 30),
    );

    // old_log.log: 500 MB, 2 years ago
    write_file_with_size_and_mtime(
        &dir.join("old_log.log"),
        500 * 1_048_576,
        SystemTime::UNIX_EPOCH + Duration::from_secs(now - 730 * DAY),
    );

    // fresh_log.log: 500 MB, 10 days ago
    write_file_with_size_and_mtime(
        &dir.join("fresh_log.log"),
        500 * 1_048_576,
        SystemTime::UNIX_EPOCH + Duration::from_secs(now - 10 * DAY),
    );

    // .hidden: 10 KB, now
    write_file_with_size_and_mtime(
        &dir.join(".hidden"),
        10_000,
        SystemTime::UNIX_EPOCH + Duration::from_secs(now - 60),
    );

    // recent_normal.bin: 50 MB, 5 days ago
    write_file_with_size_and_mtime(
        &dir.join("recent_normal.bin"),
        50 * 1_048_576,
        SystemTime::UNIX_EPOCH + Duration::from_secs(now - 5 * DAY),
    );

    // sub/nested_big.bin and nested_small.txt
    let sub = dir.join("sub");
    fs::create_dir(&sub).unwrap();
    write_file_with_size_and_mtime(
        &sub.join("nested_big.bin"),
        1_200_000_000,
        SystemTime::UNIX_EPOCH + Duration::from_secs(now - 180 * DAY),
    );
    write_file_with_size_and_mtime(
        &sub.join("nested_small.txt"),
        1024,
        SystemTime::UNIX_EPOCH + Duration::from_secs(now - 30),
    );

    (dir, now)
}

fn write_file_with_size_and_mtime(path: &std::path::Path, size: u64, mtime: SystemTime) {
    // Sparsely-written file: just seek to the end. Avoids actually
    // writing 1.5 GB to disk. stat() reports the seeked size.
    let f = File::create(path).expect("create fixture file");
    f.set_len(size).expect("set fixture file size");
    f.set_modified(mtime).expect("set fixture mtime");
    drop(f);
}

/// Scan a real on-disk directory into a TreeStore. Mirrors the
/// `scan_path_to_tree` API but stays in this test file to keep the
/// integration test self-contained (avoids cross-module imports that
/// the lib treats as public).
fn scan_dir_into_tree(root_path: &std::path::Path) -> (TreeStore, usize) {
    let mut tree = TreeStore::new();
    let root = tree.add_node(None, "root".into(), NodeKind::Dir, 0);
    tree.set_root_path(root_path.to_path_buf());
    let mut total: u64 = 0;
    let mut stack: Vec<(std::path::PathBuf, usize)> = vec![(root_path.to_path_buf(), root)];
    while let Some((dir_path, parent_id)) = stack.pop() {
        let entries = match fs::read_dir(&dir_path) {
            Ok(it) => it,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let full = entry.path();
            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            let kind = if metadata.is_symlink() {
                NodeKind::Symlink
            } else if metadata.is_dir() {
                NodeKind::Dir
            } else {
                NodeKind::File
            };
            let size = if kind == NodeKind::File {
                metadata.len()
            } else {
                0
            };
            let mtime = metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs());
            let child = tree.add_node_with_modified(
                Some(parent_id),
                name,
                kind,
                size,
                mtime,
            );
            // Track the node so we can verify by name.
            let _ = child;
            if kind == NodeKind::File {
                total += size;
            } else if kind == NodeKind::Dir {
                stack.push((full, child));
            }
        }
    }
    tree.apply_direct_size_delta(root, total);
    // Don't call repair_sorted_children (we don't have the list of
    // dirty node_ids); the engine calls ensure_sorted_children
    // internally during evaluation, so order is restored there.
    (tree, root)
}

#[test]
fn default_ruleset_finds_expected_hits() {
    let (fixture, now) = build_default_fixture();
    let (mut tree, root) = scan_dir_into_tree(&fixture);
    let rules = default_ruleset();
    let ctx = RuleContext { now_unix_secs: now };
    let hits = evaluate_rules(&rules, &mut tree, root, &ctx, 100);

    let hit_names: Vec<String> = hits
        .iter()
        .map(|h| tree.node(h.node_id).name.clone())
        .collect();

    // big.bin (1.5 GB) — matches large-file-1gb
    assert!(
        hit_names.iter().any(|n| n == "big.bin"),
        "expected big.bin in hits, got: {hit_names:?}"
    );
    // old_log.log (500 MB, 2 years old) — matches old-large-file
    assert!(
        hit_names.iter().any(|n| n == "old_log.log"),
        "expected old_log.log in hits, got: {hit_names:?}"
    );
    // nested_big.bin (1.2 GB) — matches large-file-1gb
    assert!(
        hit_names.iter().any(|n| n == "nested_big.bin"),
        "expected nested_big.bin in hits, got: {hit_names:?}"
    );
    // .hidden — matches hidden-files
    assert!(
        hit_names.iter().any(|n| n == ".hidden"),
        "expected .hidden in hits, got: {hit_names:?}"
    );

    // fresh_log.log (500 MB, 10 days old) — large enough but too
    // recent for old-large-file. Should NOT be in old-large-file hits
    // but might still appear in large-file-1gb — it's < 1 GB.
    let old_log_hits: Vec<_> = hits
        .iter()
        .filter(|h| tree.node(h.node_id).name == "fresh_log.log")
        .collect();
    assert!(
        old_log_hits.is_empty(),
        "fresh_log.log should not match, but got: {:?}",
        old_log_hits
    );

    // recent_normal.bin (50 MB, 5 days) — too small AND too recent
    let recent_hits: Vec<_> = hits
        .iter()
        .filter(|h| tree.node(h.node_id).name == "recent_normal.bin")
        .collect();
    assert!(
        recent_hits.is_empty(),
        "recent_normal.bin should not match, but got: {:?}",
        recent_hits
    );
}

#[test]
fn disabling_rule_removes_its_hits() {
    let (fixture, now) = build_default_fixture();
    let (mut tree, root) = scan_dir_into_tree(&fixture);
    let mut rules = default_ruleset();
    let ctx = RuleContext { now_unix_secs: now };

    // Disable the large-file rule. big.bin should drop out.
    rules.disable("large-file-1gb");
    let hits = evaluate_rules(&rules, &mut tree, root, &ctx, 100);
    let hit_names: Vec<String> = hits
        .iter()
        .map(|h| tree.node(h.node_id).name.clone())
        .collect();
    assert!(
        !hit_names.iter().any(|n| n == "big.bin"),
        "big.bin should not match with large-file-1gb disabled, got: {hit_names:?}"
    );
    // old_log.log still matches old-large-file
    assert!(
        hit_names.iter().any(|n| n == "old_log.log"),
        "expected old_log.log still in hits, got: {hit_names:?}"
    );
}

#[test]
fn limit_caps_results() {
    let (fixture, now) = build_default_fixture();
    let (mut tree, root) = scan_dir_into_tree(&fixture);
    let rules = default_ruleset();
    let ctx = RuleContext { now_unix_secs: now };
    // Our fixture has roughly 4 hits. A limit of 2 should return 2.
    let hits = evaluate_rules(&rules, &mut tree, root, &ctx, 2);
    assert!(hits.len() <= 2, "limit should be respected, got: {}", hits.len());
}

#[test]
fn empty_rule_set_produces_no_hits_on_real_tree() {
    let (fixture, now) = build_default_fixture();
    let (mut tree, root) = scan_dir_into_tree(&fixture);
    let rules = RuleSet::new();
    let ctx = RuleContext { now_unix_secs: now };
    let hits = evaluate_rules(&rules, &mut tree, root, &ctx, 100);
    assert!(hits.is_empty());
}

#[test]
fn rules_format_version_is_one() {
    // Forward-compat: we explicitly know v1 is the only supported
    // version. Bump and add migration when changing shape.
    assert_eq!(RULES_FORMAT_VERSION, 1);
}

#[test]
fn empty_tree_after_scan_is_handled() {
    // Sanity: a tree with no real files (e.g. root only) evaluates
    // cleanly and returns no hits.
    let mut tree = TreeStore::new();
    let root = tree.add_node(None, "root".into(), NodeKind::Dir, 0);
    tree.set_root_path("/".into());
    let rules = default_ruleset();
    let ctx = RuleContext::default();
    let hits = evaluate_rules(&rules, &mut tree, root, &ctx, 100);
    assert!(hits.is_empty());
}
