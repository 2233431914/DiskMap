# DiskMap MVP Specification

## 1. Overview

**Project Name:** DiskMap
**Type:** Native desktop disk analysis tool
**Core Feature:** Scan directories and visualize space usage with Treemap
**Target:** macOS (primary), similar to SpaceSniffer

## 2. Technology Stack

- **Framework:** Rust + eframe/egui (v0.34)
- **Parallel scanning:** jwalk (v0.8)
- **Thread communication:** crossbeam-channel (v0.5)
- **In-memory tree aggregation:** Custom TreeStore
- **Treemap rendering:** egui::Painter (custom drawing)
- **Shell open:** open (v5)
- **Preference persistence:** eframe native storage/window persistence
- **Experimental cache:** rusqlite (implemented, disabled by default)

## 3. Features (Current Implemented Scope)

### 3.1 Core Features
- [x] Input/select scan directory path
- [x] Background scanning with jwalk parallel traversal
- [x] Real-time UI refresh during scan (snapshots at depth <= 1)
- [x] Scan exclude rules with persisted user patterns
- [x] Treemap visualization by area (Squarified Treemap algorithm)
- [x] Hover tooltip showing path and size
- [x] Left-click to select, double-click to drill into directory
- [x] Right-click context menu: Open / Reveal in Finder / Copy Path / Move to Trash
- [x] Search result navigation with Previous/Next and Enter/Shift+Enter
- [x] Search filter mode showing only matches and ancestor folders
- [x] Small-file aggregation as virtual "Other Files" nodes
- [x] Manual rescan for scan root and focused subtree
- [x] Default-on filesystem watch with debounced incremental subtree rescan
- [x] CSV/JSON export for scan root or focused subtree
- [x] Focused report JSON export with reproduction metadata for the current view
- [x] Manual read-only file age/type insight report for the focused subtree
- [x] Active size basis display
- [x] Optional SQLite scan cache setting, disabled by default
- [x] Extension-based color mode
- [x] Direct Move to Trash with protected-path validation and immediate view update
- [x] Cleanup review queue before platform Trash actions

### 3.2 Explicitly Out of Current Default Path
- SQLite storage enabled by default
- Duplicate file detection as a cleanup signal
- Permanent deletion that bypasses the operating system Trash
- Animations
- Cleanup automation that mutates scan/search state

### 3.3 Sidebar Features
- [x] Current directory path display
- [x] Current directory size display
- [x] Open in Finder button

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
Aggregator (TreeStore)
    ↓ accumulate sizes
    ↓ emit incremental batches/Finished
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
    lower_name: String,
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
- **Small files:** files at or below 16 KiB are aggregated per parent into a virtual `Other Files` node. Virtual aggregate nodes have no real filesystem path and cannot be opened, revealed, or copied as a path.

## 7. Platform Integration

### 7.1 macOS Specific
- `open -R <path>` for Reveal in Finder
- `open <path>` for Open

### 7.2 Clipboard
- Copy full path string through egui clipboard integration

### 7.3 Destructive Actions
- Move to Trash is available from the selected-node details panel and the right-click menu.
- Direct Trash validates protected paths and target existence before calling the platform adapter.
- Successful Trash removes the node from the in-memory view immediately; failed platform actions only report status.
- Review-queue Trash still supports a second confirmation click with path, size, and affected item count.

## 8. UI Layout

```
┌─────────────────────────────────────────────────────┐
│ Nav [Root] Path: [________] [Scan] Search Depth    │ <- TopPanel
├────────────┬──────────────────────────────────────┤
│ DiskMap  │                                      │
│ Current:  │                                      │
│ Size:     │         TREEMAP CANVAS                │
│ [Open]    │         (egui::Painter)               │
│ [Reveal]  │                                      │
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
  - [x] Follow or do not follow symlinks
- Safe scan options are persisted with other user-facing scan options. Defaults preserve the original scan behavior: hidden files included, symlinks not followed, and filesystem boundaries not restricted until enabled.
- [x] Add manual rescan for the current scan root and focused subtree without enabling real-time monitoring
- Manual rescan reuses the active scan options and starts a new scan for either the original scan root or the currently focused directory.

### Phase 4: Reporting and Size Model
- [x] Export the current scan tree or focused subtree as CSV/JSON with path, size, kind, and error fields
- Export actions write timestamped `disk-map-export-*` files to the current working directory and report the saved path in status.
- [x] Clearly display the active size basis, such as apparent size or allocated size on disk
- Current size basis is shown in details/progress UI. On Unix it is allocated size from filesystem blocks when available, with apparent byte length fallback; on other platforms it is apparent byte length from metadata.
- [x] Evaluate a user-facing size basis toggle if both size measurements are reliable on the target platform
- Decision: do not expose a size basis toggle yet. `TreeStore` currently stores one canonical size per node, and scanner/cache/export paths do not retain both apparent and allocated sizes. A future toggle must first add dual-size fields and migration tests so switching basis changes aggregation, treemap area, progress, and exports consistently.

### Phase 5: Real-time Monitoring
- [x] Add notify crate (FSEvents/kqueue)
- [x] Debounce 300-1000ms
- [x] Add default-on Watch control for debounced scan-root rescans after filesystem changes
- Watch is enabled by default and observes the current scan root. Users can disable it for the current session from the toolbar.
- [x] Incremental rescan of changed directories
- Debounced events are mapped to the deepest known directory containing the changed path. The app rescans that directory off the UI thread and replaces its in-memory subtree; unresolved changes fall back to the scan root.

### Phase 6: Treemap Upgrade
- [x] Preserve Squarified Treemap interface
- [x] Evaluate deeper zoom/search workflows
- Current workflow keeps the Squarified layout and adds keyboard-driven depth control (`[` / `]`) plus Enter-to-drill for selected directories. Search navigation remains scoped to the focused subtree and can move focus to matching directories or parent directories for file matches.

### Phase 7: Productization
- [x] Enable SQLite index for faster rescans behind a user setting
- SQLite remains disabled by default. The experimental `SQLite` toolbar setting switches scan cache mode to `Enabled` and is persisted with other scan options.
- [x] Search and filter
- Filter mode is an optional search display mode. It does not change search scope; it only removes non-matching branches from treemap layout while preserving the current focused subtree.
- [x] Extension-based coloring
- The optional `Ext` color mode keeps directory colors unchanged and assigns files stable colors based on lowercase extension.
- [x] Move to Trash functionality with confirmation and reliable platform adapter
- Move to Trash is available without a separate enable toggle, uses the platform Trash, and is unavailable for virtual aggregate nodes.

### Phase 8: Analysis Workflows
- [x] Recent scan roots and pinned favorites for repeat analysis
- The `Roots` menu keeps successful scan roots in a capped recent list and stores pinned favorites separately in local preferences. Selecting a root starts a new scan with the current scan options.
- [x] Snapshot comparison to show growth, shrinkage, and newly added large paths between scans
- Snapshot diff is read-only and compares the latest completed scan with the previous in-memory snapshot for the same root. It reports total delta plus top added, grown, shrunk, and removed paths by byte impact.
- [x] Optional duplicate-file candidate analysis as a read-only report before any cleanup workflow
- Duplicate analysis is manual and read-only. The current heuristic groups files by same normalized file name and same measured size inside the focused subtree; it does not hash file contents and does not enable cleanup actions.
- [x] File age and file type insights, including modified-time filters and category summaries
- Insight analysis is manual and read-only for the current focused subtree. File modified times are captured when available; category summaries are extension-based, and age buckets are best-effort with unknown mtime reported separately.
- [x] Export/share a focused report with enough metadata to reproduce the visible result
- Focused reports export JSON with generated time, scan root path, focused path, size basis, depth, search/filter state, color mode, scan options, exclude patterns, and the focused subtree entries.

### Phase 9: Cleanup Workflow Safety
- [x] Add a review queue for cleanup candidates before any destructive action
- Selected nodes can be moved directly to Trash from the details panel or right-click menu. The cleanup queue remains available internally for review-style flows and shows path, size, kind, and affected item count.
- [x] Add protected-path guardrails for system folders, mounted volumes, and user-configured deny lists
- Guardrails block filesystem root, the home root, common system locations, mounted volume roots, and user-configured protected roots. User paths are comma, semicolon, or newline separated and apply to the path itself plus descendants.
- [x] Require explicit confirmation with path, size, and affected item count before Move to Trash
- The queued Trash action first enters a confirmation state and reports the target path, selected byte size, and affected item count before the second click can call the platform Trash adapter. Direct Move to Trash keeps protected-path and existence validation before platform calls.
- [x] Keep cleanup actions separate from scanning and search so a failed platform action never mutates scan state
- Successful Move to Trash removes the node from the in-memory tree immediately; failed platform actions only report status and leave scan/search state unchanged.

### Phase 10: Accessibility and Packaging
- [ ] Keyboard shortcuts for primary navigation, search navigation, rescan, and focused export
- [ ] Accessible labels and focus order for toolbar, treemap selection, and context menu actions
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
- [x] Command palette for navigation, scan, export, filter, and view-mode actions (Implemented in: 62c1d7a; 17 builtin commands; Cmd+K / Ctrl+K to open; case-insensitive substring filter; Enter / Esc / Up / Down keyboard handling)
- [x] Keyboard-first triage flow for moving between search matches, report rows, and selected treemap nodes (Implemented in: existing handle_keyboard + new Esc/Cmd+K bindings; Enter=enter selected, Backspace=back, Alt+←/→=back/forward, [/]=depth, Esc=clear/close)
- [ ] Saved filter presets for extension, category, size threshold, modified age, hidden files, symlink policy, and exclude patterns
  - Partial: `FilterPreset` + `FilterStore` in `src/views.rs` support persisted named search-query/filter-mode presets in the sidebar. Typed presets for extension/category/size/age/hidden/symlink/exclude controls remain deferred.
- [ ] Multi-root comparison workspace for comparing several scan roots side by side
- [ ] Bookmark selected nodes inside a scan for later review
- [x] Saved views that remember focused node, depth, search/filter state, color mode, and selected report mode (Implemented in: 7ac572a; per-root `ViewState` with depth/search_query/search_filter_enabled/color_by_extension/last_report_mode/focused_id/selected_id; "Save current view" + "Apply saved view" buttons in a "VIEW" section of the details panel; 6 unit tests; serde round-trip)
- [ ] Deep-link style local references to reopen a saved root, snapshot, focused node, and view mode
- [ ] Configurable color palettes for directory depth, extension mode, and category mode
- [x] Headless CLI entry point for scan and export jobs using the same scanner, exclude rules, and report formats as the GUI (Implemented in: a6d9cb0; `diskmap-cli scan <path> [-f text|json|csv] [-e <pattern>] [--max-depth N] [--include-hidden] [--follow-symlinks] [--sort-by path|size] [-o FILE]`; hand-rolled arg parser, no new deps; 14 unit tests for parser + format + sort)

### Phase 15: Reliability and Distribution
- [ ] Crash-safe local state writes for preferences, history, snapshots, and cleanup audit logs
  - Partial: preferences plus compact user state (profiles, saved views, filter presets, and rulesets) use `SafeStorage` with write-to-temp + fsync + rename. Full history, snapshots, and cleanup audit logs are not persisted through this path yet.
- [x] Large-tree benchmark suite with fixed fixtures and regression thresholds (Implemented in: e47c6ac; baselines file at benches/baselines/large_tree.txt — 10k/100k numbers are TBD pending a full bench run)
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
- [ ] Report-row-to-treemap linking so selecting a duplicate, insight, comparison, or cleanup candidate row selects the corresponding node when it is still present
- [ ] Breadcrumb and minimap-style orientation aids for very deep focused subtrees, kept compact and optional
- [ ] Empty-space and tiny-file explanations so aggregate nodes and skipped tiny rectangles are understandable without adding visual clutter

### Phase 17: Automation and Scheduled Analysis
- [ ] Default-off scheduled scans for pinned roots, with local-only results and no cleanup automation
- [ ] Change summary notification after a scheduled scan when growth, new large files, or errors exceed user-configured thresholds
- [ ] Exportable scheduled report templates for focused roots, comparisons, category summaries, and cleanup dry runs
- [ ] Background work throttling so scheduled scans never compete aggressively with active interactive scans

### Phase 18: Extensibility and Rule Management
- [x] User-editable rule sets for categories, anomaly hints, cleanup candidates, and protected paths (Implemented in: e5f4799, ecde3e8; default ruleset + UI sidebar; adding new rules from the UI is deferred — for now, edit via JSON import)
- [x] Import/export rule bundles with validation and preview before applying changes (Implemented in: `src/rules.rs` + `src/app/rule_actions.rs`; JSON import validates version/fields/duplicate ids, sidebar preview shows added/removed/changed/unchanged counts, and apply/cancel is required before replacing the live ruleset)
- [ ] Per-root option profiles for exclude rules, safe scan options, watch/cache settings, and report defaults
  - Partial: per-root profiles are persisted through `SafeStorage` and applied before scan startup. Report defaults remain deferred.
- [x] Rule test fixtures that let users validate matching behavior against example paths before enabling a rule set (Implemented in: 2c12a36; integration tests in tests/rules_fixtures.rs)
