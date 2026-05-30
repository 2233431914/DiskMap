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

## 3. Features (MVP Scope)

### 3.1 Core Features
- [x] Input/select scan directory path
- [x] Background scanning with jwalk parallel traversal
- [x] Real-time UI refresh during scan (snapshots at depth <= 1)
- [x] Scan exclude rules with persisted user patterns
- [x] Treemap visualization by area (Squarified Treemap algorithm)
- [x] Hover tooltip showing path and size
- [x] Left-click to select, double-click to drill into directory
- [x] Right-click context menu: Reveal in Finder / Copy Path / Open
- [x] Search result navigation with Previous/Next and Enter/Shift+Enter
- [x] Small-file aggregation as virtual "Other Files" nodes

### 3.2 Excluded from MVP
- SQLite storage enabled in the default UI
- Duplicate file detection
- FSEvents real-time monitoring
- Animations
- File deletion / Move to Trash
- Export/reporting workflows
- Advanced scan safety toggles and manual rescan shortcuts

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
- Move to Trash is not exposed in the default MVP UI.
- Experimental trash support must report errors and must not silently trigger a rescan.

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

## 10. Future Phases

### Phase 2: Stabilization and Usability
- [x] Keep destructive actions disabled by default
- [x] Keep SQLite cache disabled by default
- [x] Maintain clippy-clean code with `cargo clippy --all-targets --all-features -- -D warnings`
- [x] Add scan error summary after completion: permission errors, skipped paths, symlinks, and error entries
- [x] Improve empty/error/cancelled states for missing paths, inaccessible roots, empty folders, and cancelled scans
- [x] Persist lightweight preferences: last scan path, window size, theme, and depth
- [x] Persist current user-facing scan options

### Phase 3: Scan Controls
- [x] Add scan exclude rules for common noisy folders and user patterns, such as `.git`, `node_modules`, build outputs, and cache directories
- Exclude input accepts comma, semicolon, or newline separated patterns. Plain names match path components; patterns containing `/` match the normalized path; `*` wildcard is supported.
- [ ] Add safe scan mode options:
  - [ ] Do not cross filesystem or mount boundaries
  - [ ] Include or exclude hidden files
  - [ ] Follow or do not follow symlinks
- [ ] Add manual rescan for the current scan root and focused subtree without enabling real-time monitoring

### Phase 4: Reporting and Size Model
- [ ] Export the current scan tree or focused subtree as CSV/JSON with path, size, kind, and error fields
- [ ] Clearly display the active size basis, such as apparent size or allocated size on disk
- [ ] Evaluate a user-facing size basis toggle if both size measurements are reliable on the target platform

### Phase 5: Real-time Monitoring
- [ ] Add notify crate (FSEvents/kqueue)
- [ ] Debounce 300-1000ms
- [ ] Incremental rescan of changed directories

### Phase 6: Treemap Upgrade
- [x] Preserve Squarified Treemap interface
- [ ] Evaluate deeper zoom/search workflows

### Phase 7: Productization
- [ ] Enable SQLite index for faster rescans behind a user setting
- [ ] Search and filter
- [ ] Extension-based coloring
- [ ] Move to Trash functionality with confirmation and reliable platform adapter

### Phase 8: Analysis Workflows
- [ ] Recent scan roots and pinned favorites for repeat analysis
- [ ] Snapshot comparison to show growth, shrinkage, and newly added large paths between scans
- [ ] Optional duplicate-file candidate analysis as a read-only report before any cleanup workflow
- [ ] File age and file type insights, including modified-time filters and category summaries
- [ ] Export/share a focused report with enough metadata to reproduce the visible result

### Phase 9: Cleanup Workflow Safety
- [ ] Add a review queue for cleanup candidates before any destructive action
- [ ] Add protected-path guardrails for system folders, mounted volumes, and user-configured deny lists
- [ ] Require explicit confirmation with path, size, and affected item count before Move to Trash
- [ ] Keep cleanup actions separate from scanning and search so a failed platform action never mutates scan state

### Phase 10: Accessibility and Packaging
- [ ] Keyboard shortcuts for primary navigation, search navigation, rescan, and focused export
- [ ] Accessible labels and focus order for toolbar, treemap selection, and context menu actions
- [ ] Performance regression benchmarks for large trees, search rebuilds, and layout generation
- [ ] macOS packaging, signing/notarization, and release checklist
