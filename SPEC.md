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
- **Clipboard:** arboard (v3)
- **Shell open:** open (v5)

## 3. Features (MVP Scope)

### 3.1 Core Features
- [x] Input/select scan directory path
- [x] Background scanning with jwalk parallel traversal
- [x] Real-time UI refresh during scan (snapshots at depth <= 1)
- [x] Treemap visualization by area (Slice-and-Dice algorithm)
- [x] Hover tooltip showing path and size
- [x] Left-click to drill into directory
- [x] Right-click context menu: Reveal in Finder / Copy Path / Open

### 3.2 Excluded from MVP
- SQLite storage
- Duplicate file detection
- FSEvents real-time monitoring
- Allocated size (real disk blocks)
- Animations
- File deletion

### 3.3 Sidebar Features
- [x] Current directory path display
- [x] Current directory size display
- [x] Open in Finder button
- [x] List of largest children (sorted, clickable to drill)

### 3.4 Navigation
- [x] Breadcrumb path display
- [x] Root button to return to scan root
- [x] Status bar showing scan progress

## 4. Architecture

### 4.1 Data Flow
```
UI Thread
    ↓ StartScan(path)
Scan Thread (jwalk)
    ↓ ScanEvent::File/Dir/Error via channel
Aggregator (TreeStore)
    ↓ accumulate sizes
    ↓ emit Snapshot/Finished
egui Painter
    ↓ draw Treemap
```

### 4.2 Threading Model
- Scanning runs on separate thread spawned by `scanner::start_scan`
- Communication via crossbeam-channel (bounded sender, unbounded receiver)
- UI thread receives messages in `app.update()` via `rx.try_recv()`
- No direct UI manipulation from scan thread

### 4.3 File Structure
```
disk-map/
├── Cargo.toml
└── src/
    ├── main.rs
    ├── app.rs
    ├── scanner.rs
    ├── tree.rs
    ├── treemap.rs
    ├── platform.rs
    └── format.rs
```

## 5. Data Structures

### 5.1 NodeKind
```rust
enum NodeKind { File, Dir, Symlink, Error }
```

### 5.2 Node
```rust
struct Node {
    id: NodeId,
    parent: Option<NodeId>,
    name: String,
    path: PathBuf,
    kind: NodeKind,
    size: u64,
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
}
```

## 6. Treemap Layout

- **Algorithm:** Slice-and-Dice (simple, fast)
- **Max display depth:** 5 levels
- **Node sorting:** By size descending (largest first)
- **Visual feedback:**
  - Hover highlights node (gamma 1.35)
  - 6-color palette cycling by depth
  - Minimum rect threshold: 2px (skip render)

## 7. Platform Integration

### 7.1 macOS Specific
- `open -R <path>` for Reveal in Finder
- `open <path>` for Open

### 7.2 Clipboard
- Copy full path string to clipboard via arboard

## 8. UI Layout

```
┌─────────────────────────────────────────────────────┐
│ Path: [_______________] [Scan] [Root]  Status      │ <- TopPanel
├────────────┬──────────────────────────────────────┤
│ DiskMap  │                                      │
│ Current:  │                                      │
│ Size:     │         TREEMAP CANVAS                │
│ [Open]    │         (egui::Painter)               │
│ [Reveal]  │                                      │
│           │                                      │
│ Largest:  │                                      │
│ - child1  │                                      │
│ - child2  │                                      │
│ ...       │                                      │
└────────────┴──────────────────────────────────────┘
```

## 9. Performance Targets

- Scan 100K files in < 30 seconds
- UI responsive during scan (60fps capable)
- Memory efficient (no duplicate tree structures)

## 10. Future Phases

### Phase 2: Performance
- Replace recursive scan with jwalk
- Batch events (every 100ms progress, every 500ms snapshot)
- Area threshold: merge small nodes

### Phase 3: Real-time Monitoring
- Add notify crate (FSEvents/kqueue)
- Debounce 300-1000ms
- Incremental rescan of changed directories

### Phase 4: Treemap Upgrade
- Replace Slice-and-Dice with Squarified Treemap
- Maintain same interface

### Phase 5: Productization
- SQLite index for faster rescans
- Search and filter
- Extension-based coloring
- Move to Trash functionality