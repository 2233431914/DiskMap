# DiskMap MVP Specification

## 1. Overview

**Project Name:** DiskMap
**Type:** Native desktop disk analysis tool
**Core Feature:** Scan directories and visualize space usage with Treemap
**Target:** macOS and Linux desktop, similar to SpaceSniffer

## 2. Technology Stack

- **Framework:** Rust + eframe/egui (v0.34)
- **Parallel scanning:** jwalk (v0.8)
- **Thread communication:** crossbeam-channel (v0.5)
- **In-memory tree:** Custom TreeStore with lossless file nodes; treemap aggregation is display-only
- **Treemap rendering:** egui::Painter (custom drawing)
- **Shell open:** open (v5)
- **Local persistence:** crash-safe app data JSON for compact app state/preferences; eframe native window persistence
- **Experimental cache:** rusqlite (implemented, disabled by default)

## 3. Features (Current Implemented Scope)

### 3.1 Core Features
- [x] Input/select scan directory path
- [x] Background scanning with jwalk parallel traversal
- [x] Real-time UI refresh during scan (batched scan messages)
- [x] Scan exclude rules with persisted user patterns
- [x] Treemap visualization by area (Squarified Treemap algorithm)
- [x] Hover tooltip showing path and size
- [x] Left-click to select, double-click to drill into directory
- [x] Right-click context menu: Open / Reveal in Finder or Open Containing Folder / Copy Path / Move to Trash
- [x] Search result navigation with Previous/Next and Enter/Shift+Enter
- [x] Search filter mode showing only matches and ancestor folders
- [x] Display-only small-file aggregation in treemap (the underlying TreeStore keeps every file)
- [x] Manual filesystem rescan for the scan root
- Deferred: focused-subtree rescans remain out of scope; the toolbar rescan always rebuilds the current scan root.
- [x] Default-on filesystem watch with debounced full scan-root rescan
- [x] GUI CSV/JSON export and focused report panels
- [x] GUI file age/type insight report
- [x] Active size basis display
- [ ] GUI SQLite scan cache setting (deferred; cache implementation remains disabled)
- [x] Extension-based color mode
- [x] Direct Move to Trash with normalized protected-path validation, explicit confirmation, and immediate view update
- [ ] Cleanup review queue UI (deferred; no production queue entry point)

### 3.2 Explicitly Out of Current Default Path
- SQLite storage enabled by default
- Duplicate file detection as a cleanup signal
- Permanent deletion that bypasses the operating system Trash
- Animations
- Cleanup automation that mutates scan/search state

### 3.3 Sidebar Features
- [x] Current directory path display
- [x] Current directory size display
- [x] Open and platform file-manager action buttons

### 3.4 Navigation
- [x] Breadcrumb path display
- [x] Root button to return to scan root
- [x] Status bar showing scan progress

### 3.5 Preferences
- [x] Persist last scan path, window size, theme, depth, and current user-facing scan options
- [x] Restore persisted preferences on startup
- [x] Persist recent scan roots and pinned favorites for repeat analysis

## 4. Architecture

### 4.1 Data Flow
```
UI Thread
    ↓ StartScan(path)
Scan Thread (jwalk)
    ↓ ScanMessage::Batch/Error via channel
TreeStore on UI thread
    ↓ append lossless nodes and update ancestor sizes
    ↓ mark layout/search state dirty
egui Painter
    ↓ draw Treemap
```

### 4.2 Threading Model
- Scanning runs on separate thread spawned by `scanner::start_scan`
- Communication via crossbeam-channel (unbounded sender/receiver)
- UI thread receives messages in `app.update()` via `rx.try_recv()`
- No direct UI manipulation from scan thread

### 4.3 File Structure
```
disk-map/
├── Cargo.toml
└── src/
    ├── main.rs
    ├── app.rs
    ├── app/
    │   ├── navigation.rs
    │   ├── scan_session.rs
    │   └── search_nav.rs
    ├── scanner.rs
    ├── tree.rs
    ├── treemap.rs
    ├── cleanup.rs
    ├── export.rs
    ├── insights.rs
    ├── platform.rs
    └── format.rs
```

`app.rs` owns UI composition, painting, and cross-state coordination. App state with deeper lifecycle rules is kept in focused submodules: navigation history/focus, search navigation cursor/dirty state, and scan session progress/perf state.

## 5. Data Structures

### 5.1 NodeKind
```rust
enum NodeKind { File, Dir, Symlink, Error, Aggregate }
```

`Aggregate` is retained for library compatibility and report fixtures; the
production scanner emits real file nodes and treemap aggregation is visual-only.

### 5.2 Node
```rust
struct Node {
    parent: Option<NodeId>,
    name: String,
    kind: NodeKind,
    size: u64,
    modified_secs: Option<u64>,
    children: Vec<NodeId>,
    scanned: bool,
    error: Option<String>,
}
```

### 5.3 TreeStore
```rust
struct TreeStore {
    nodes: Vec<Node>,
    root: Option<NodeId>,
    root_path: PathBuf,
}
```

## 6. Treemap Layout

- **Algorithm:** Squarified Treemap
- **Default display depth:** 1 level, adjustable up to 10
- **Node sorting:** By size descending (largest first)
- **Visual feedback:**
  - Hover/selection highlights node
  - Search matches and ancestors are visually distinguished
  - Search navigation cycles through matches in the current focused subtree using tree display order
  - Directory palette cycles by depth
  - Minimum rect threshold: 2px (skip render)
- **Small files:** the scanner keeps every file and path. When a directory has at least eight files at or below 16 KiB, treemap layout groups them into a display-only `Other Files` block; search disables this grouping. The block has no filesystem path and cannot be opened, revealed, copied, or moved to Trash.

## 7. Platform Integration

### 7.1 Platform Shell Integration
- macOS: `open -R <path>` for Reveal in Finder, `open <path>` for Open
- Linux and other desktops: `open` crate integration for Open; the file-manager action opens the directory itself or the selected file's containing directory

### 7.2 Clipboard
- Copy full path string through egui clipboard integration

### 7.3 Destructive Actions
- Move to Trash is available from the selected-node details panel and the right-click menu.
- Direct Trash normalizes paths, validates protected paths and target existence, then requires a second explicit confirmation before calling the platform adapter.
- Successful Trash removes the node from the in-memory view immediately; failed platform actions only report status.
- Any production Trash entry point uses the same second confirmation with normalized path, size, and affected item count.

## 8. UI Layout

```
┌─────────────────────────────────────────────────────┐
│ Nav [Root] Path: [________] [Scan] Search Depth    │ <- TopPanel
├────────────┬──────────────────────────────────────┤
│ DiskMap  │                                      │
│ Current:  │                                      │
│ Size:     │         TREEMAP CANVAS                │
│ [Open]    │         (egui::Painter)               │
│ [Folder]  │                                      │
│           │                                      │
└────────────┴──────────────────────────────────────┘
```

## 9. Performance Targets

- Scan 100K files in < 30 seconds
- UI responsive during scan (60fps capable)
- Memory efficient (no duplicate tree structures)

## 10. Roadmap

Unchecked items below are accepted product backlog, not current behavior. Analysis features should stay read-only by default. Destructive workflows must keep protected-path checks and clear status reporting before platform actions.

### Phase 2: Stabilization and Usability
- [x] Keep permanent deletion unavailable; Move to Trash uses protected-path validation
- [x] Keep SQLite cache disabled by default
- [x] Maintain clippy-clean code with `cargo clippy --all-targets --all-features -- -D warnings`
- [x] Add scan error summary after completion: permission errors, skipped paths, symlinks, and error entries
- [x] Improve empty/error/cancelled states for missing paths, inaccessible roots, empty folders, and cancelled scans
- [x] Persist lightweight preferences: last scan path, window size, theme, and depth
- [x] Persist current user-facing scan options

### Phase 3: Scan Controls
- [x] Add scan exclude rules for common noisy folders and user patterns, such as `.git`, `node_modules`, build outputs, and cache directories
- Exclude input accepts comma, semicolon, or newline separated patterns. Plain names match path components; patterns containing `/` match the normalized path; `*` wildcard is supported.
- [x] Add safe scan mode options:
  - [x] Do not cross filesystem or mount boundaries where platform device IDs are available
  - [x] Include or exclude hidden files
  - [x] Show symlinks without following them
- Symlink traversal is intentionally disabled and legacy preference/CLI values are migrated or rejected. Hidden files are included by default and filesystem boundaries are not restricted until enabled.
- [x] Manual rescan for the current scan root without enabling real-time monitoring
- Deferred: focused-subtree rescans remain out of scope; the toolbar rescan always rebuilds the current scan root.

### Phase 4: Reporting and Size Model
- [x] Headless CLI export of scan rows as text/CSV/JSON with path, size, kind, and error fields
- `diskmap-cli` writes text/CSV/JSON to stdout by default or to the exact path supplied with `-o`.
- [x] Clearly display the active size basis, such as apparent size or allocated size on disk
- Current size basis is shown in details/progress UI. On Unix it is allocated size from filesystem blocks when available, with apparent byte length fallback; on other platforms it is apparent byte length from metadata.
- [x] Evaluate a user-facing size basis toggle if both size measurements are reliable on the target platform
- Decision: do not expose a size basis toggle yet. `TreeStore` currently stores one canonical size per node, and scanner/cache/export paths do not retain both apparent and allocated sizes. A future toggle must first add dual-size fields and migration tests so switching basis changes aggregation, treemap area, progress, and exports consistently.

### Phase 5: Real-time Monitoring
- [x] Add notify crate (platform backend: FSEvents/kqueue/inotify as available)
- [x] Debounce 300-1000ms
- [x] Add default-on Watch control for debounced scan-root rescans after filesystem changes
- Watch is enabled by default and observes the current scan root. Users can disable it for the current session from the toolbar.
- [x] Full scan-root rescan after debounced changes
- The watcher remains attached to the current root while scans run. Events are coalesced and trigger a generation-guarded full rescan; changes observed during a scan schedule one follow-up rescan.

### Phase 6: Treemap Upgrade
- [x] Preserve Squarified Treemap interface
- [x] Evaluate deeper zoom/search workflows
- Current workflow keeps the Squarified layout and adds keyboard-driven depth control (`[` / `]`) plus Enter-to-drill for selected directories. Search navigation remains scoped to the focused subtree and can move focus to matching directories or parent directories for file matches.

### Phase 7: Productization
- [ ] Enable SQLite index for faster rescans behind a user setting
- Deferred: the cache implementation remains available for experiments but production UI forces `CacheMode::Disabled`.
- [x] Search and filter
- Filter mode is an optional search display mode. It does not change search scope; it only removes non-matching branches from treemap layout while preserving the current focused subtree.
- [x] Extension-based coloring
- The optional `Ext` color mode keeps directory colors unchanged and assigns files stable colors based on lowercase extension.
- [x] Move to Trash functionality with confirmation and reliable platform adapter
- Move to Trash is available without a separate enable toggle, uses the platform Trash, and is unavailable for virtual aggregate nodes.

### Phase 8: Analysis Workflows
- [x] Recent scan roots and pinned favorites for repeat analysis
- The `Roots` menu keeps successful scan roots in a capped recent list and stores pinned favorites separately in local preferences. Selecting a root starts a new scan with the current scan options.
- [x] Snapshot comparison in the GUI
- [x] Duplicate candidates and file age/type reports in the GUI
- Reports are generated or updated from completed scans in the details panel; report paths can focus the matching treemap node.
- [x] Focused report export from the GUI
- [x] Snapshot diff export from the GUI
- GUI export writes the current focused subtree or snapshot diff as CSV or JSON; the headless CLI remains available for scripted exports.

### Phase 9: Cleanup Workflow Safety
- [ ] Add a review queue for cleanup candidates before any destructive action
- Deferred: selected nodes use the single-item confirmation flow; no queue UI is exposed.
- [x] Add protected-path guardrails for system folders, mounted volumes, and user-configured deny lists
- Guardrails block filesystem root, the home root, common system locations, mounted volume roots, and user-configured protected roots. User paths are comma, semicolon, or newline separated and apply to the path itself plus descendants.
- [x] Require explicit confirmation with path, size, and affected item count before Move to Trash
- Every production Trash action first enters a confirmation state and reports the normalized target path, selected byte size, and affected item count before the second click can call the platform Trash adapter.
- [x] Keep cleanup actions separate from scanning and search so a failed platform action never mutates scan state
- Successful Move to Trash removes the node from the in-memory tree immediately; failed platform actions only report status and leave scan/search state unchanged.

### Phase 10: Accessibility and Packaging
- [x] Keyboard shortcuts for primary navigation, search navigation, rescan, and focused export
- [x] Accessible labels and focus order for toolbar, treemap selection, and context menu actions
- [ ] Performance regression benchmarks for large trees, search rebuilds, and layout generation
- [ ] macOS packaging, signing/notarization, and release checklist
  - Partial: `scripts/package-macos.sh` builds `target/dist/DiskMap.app` plus zip, supports ad-hoc or Developer ID signing, optional `notarytool` submission/stapling, and optional simple DMG. Public release still needs a real Developer ID notarization run and upgrade/rollback checklist.

### Phase 11: Practical Analysis Additions
- [ ] Size anomaly hints: highlight unexpectedly large caches, build artifacts, and log folders using configurable read-only rules
- [ ] Type/category breakdown: summarize file categories such as media, archives, code, documents, dependencies, caches, and system artifacts
- [ ] Age cleanup view: show old large files and stale directories by modified time without selecting them for destructive action automatically
- [ ] Quick filters for size threshold, modified age, file category, extension, hidden files, symlinks, error nodes, and virtual aggregate nodes
- [ ] Selected-node metadata summary with modified time, child counts, file category, extension, real-path availability, and percent of current focus size
- [ ] Permission and scan-error insight summary with retry, reveal, or exclude suggestions that never mutate scan options automatically
- [ ] Explainable read-only recommendation scoring for likely cleanup candidates, showing the exact rule and evidence behind each suggestion
- [ ] Scan session notes: let users attach short local notes to saved roots, snapshots, or reports
- [ ] Ignore suggestions: propose exclude patterns for repeated noisy folders, but require user confirmation before adding rules
- [ ] Open containing terminal for real paths where the platform supports it

### Phase 12: Comparison and History
- [ ] Persist lightweight scan metadata history for recent roots, including timestamp, size basis, option summary, and root path
- [ ] Snapshot diff view with added, removed, grown, and shrunk paths grouped by impact
- [ ] Compare any two saved snapshots for the same root, not only the latest completed pair
- [ ] Compare two independently scanned roots as a read-only side-by-side analysis when their paths differ
- [ ] Group comparison results by folder, file category, extension, and modified-age bucket
- [ ] Trend chart for recent scans of the same root
- [ ] Saved report library for generated duplicate, insight, export, and comparison summaries
- [ ] Export comparison reports as CSV/JSON with enough metadata to reproduce the comparison
- [ ] Optional baseline pinning so one snapshot can be reused as the comparison target

### Phase 13: Cleanup Assistant
- [ ] Read-only candidate rules for common cleanup targets, such as dependency folders, build outputs, old archives, large logs, and duplicate-name clusters
- [ ] Review queue with include/exclude decisions, total selected size, item count, and affected roots
- [ ] Protected path policy that blocks system locations, mounted volumes, home root, and user-configured deny lists
- [ ] Queue-level dry-run validation that checks path existence, real-path availability, protected-path status, and size drift before any platform operation
- [ ] Confirmation dialog that requires visible path, selected byte size, affected item count, and operation type before platform trash
- [ ] Cleanup audit log recording requested action, result, failures, timestamp, and target paths
- [ ] Dry-run export for cleanup plans before any platform action is enabled
- [ ] Post-action verification that reports moved, missing, skipped, and failed paths without silently changing scan/search state
- [ ] Undo guidance that explains platform Trash recovery options when supported, without promising guaranteed restoration

### Phase 14: Power User Workflow
- [ ] Command palette for navigation, scan, export, filter, and view-mode actions
- Deferred: no command registry or command-palette UI is shipped.
- [x] Keyboard-first navigation for the shipped scan/search/treemap flow
- [ ] Saved filter presets for extension, category, size threshold, modified age, hidden files, symlink policy, and exclude patterns
  - Partial: `FilterPreset` + `FilterStore` in `src/views.rs` support persisted named search-query/filter-mode presets in the sidebar. Typed presets for extension/category/size/age/hidden/symlink/exclude controls remain deferred.
- [ ] Multi-root comparison workspace for comparing several scan roots side by side
- [ ] Bookmark selected nodes inside a scan for later review
- [ ] Saved views that remember focused node, depth, search/filter state, color mode, and selected report mode
- Deferred: persistence types remain for compatibility, but no production UI exposes saved-view actions.
- [ ] Deep-link style local references to reopen a saved root, snapshot, focused node, and view mode
- [ ] Configurable color palettes for directory depth, extension mode, and category mode
- [x] Headless CLI entry point for scan and export jobs using the production scanner (the `--follow-symlinks` flag is rejected because traversal is disabled)

### Phase 15: Reliability and Distribution
- [ ] Crash-safe local state writes for preferences, history, snapshots, and cleanup audit logs
  - Partial: preferences plus compact user state (profiles, saved views, filter presets, and rulesets) use `SafeStorage` with write-to-temp + fsync + rename. Full history, snapshots, and cleanup audit logs are not persisted through this path yet.
- [x] Large-tree benchmark suite with fixed fixtures and directional baselines (fixtures now contain the requested 1k/10k/100k entry counts; layout baselines remain intentionally TBD)
- [ ] UI smoke tests for scan, navigation, search, export, watch, cache, and trash confirmation flows
  - Partial: app-driver smoke coverage exercises scan, navigation, search, JSON export action, watch startup, SQLite cache path, and trash confirmation. Rendered UI smoke coverage remains deferred.
- [x] Diagnostics bundle export with app version, platform, scan options, perf counters, recent errors, and redacted local paths where requested (Implemented in: 0091b27)
- [ ] macOS app bundle release profile with signing and notarization documentation
  - Partial: app bundle script and signing/notarization documentation live in `scripts/package-macos.sh` and `packaging/macos/README.md`; release-profile automation remains incomplete until a real Developer ID/notarization credential path is exercised.
- [ ] Release checklist for upgrades, preference migration, cache compatibility, and rollback testing
- [ ] Import/export settings bundle for migrating preferences between machines
- [ ] Privacy statement documenting that scans, histories, caches, and reports are local-only unless the user exports files manually

### Phase 16: Visualization and Review Ergonomics
- [ ] Category and age color modes that reuse the same category and modified-time model as the insight reports
- [ ] Size histogram for the focused subtree, with buckets that can feed the quick filter controls
- [x] Report-row-to-treemap linking for duplicate, insight, and comparison rows when the node is still present
- [ ] Breadcrumb and minimap-style orientation aids for very deep focused subtrees, kept compact and optional
- [ ] Empty-space and tiny-file explanations so aggregate nodes and skipped tiny rectangles are understandable without adding visual clutter

### Phase 17: Automation and Scheduled Analysis
- [ ] Default-off scheduled scans for pinned roots, with local-only results and no cleanup automation
- [ ] Change summary notification after a scheduled scan when growth, new large files, or errors exceed user-configured thresholds
- [ ] Exportable scheduled report templates for focused roots, comparisons, category summaries, and cleanup dry runs
- [ ] Background work throttling so scheduled scans never compete aggressively with active interactive scans

### Phase 18: Extensibility and Rule Management
- [ ] User-editable rule sets in the GUI
- Deferred: the rule engine and JSON validation remain library/test code; no production sidebar action is shipped.
- [ ] Import/export rule bundles from the GUI
- Deferred until a real production entry point is restored.
- [ ] Per-root option profiles for exclude rules, safe scan options, watch/cache settings, and report defaults
  - Partial: per-root profiles are persisted through `SafeStorage` and applied before scan startup. Report defaults remain deferred.
- [x] Rule test fixtures that let users validate matching behavior against example paths before enabling a rule set (Implemented in: 2c12a36; integration tests in tests/rules_fixtures.rs)
