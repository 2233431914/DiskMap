# AGENTS.md

Engineering conventions for human and AI contributors working on DiskMap.
Project context: personal-use macOS tool, MVP feature-complete, single user,
single repo. Optimize for: **UX speed, stability, fast iteration**. Do **not**
optimize for: cross-platform distribution, external-contributor friendliness,
or compatibility with old eframe/egui versions.

## Project Layout

```
disk-map/
├── Cargo.toml
├── README.md          # user-facing
├── SPEC.md            # product spec + roadmap (Phases 1–18)
├── AGENTS.md          # this file
├── benches/perf.rs    # criterion perf suite
└── src/
    ├── main.rs        # 20 lines: eframe::run_native entry point
    ├── lib.rs         # module exports
    ├── app.rs         # DiskMapApp struct + UI composition (large, see below)
    ├── app/
    │   ├── navigation.rs     # focus / back-history / drill-down state
    │   ├── scan_session.rs   # active scan progress + perf counters
    │   └── search_nav.rs     # search cursor, match cycling, dirty state
    ├── scanner.rs     # jwalk parallel traversal, batched channel messages
    ├── tree.rs        # TreeStore / Node / NodeId — core data model
    ├── treemap.rs     # Squarified layout, search state, visual nodes
    ├── watcher.rs     # notify (FSEvents/kqueue) debounce
    ├── cleanup.rs     # CleanupQueue, protected-path guardrails
    ├── duplicates.rs  # read-only name+size duplicate candidates
    ├── insights.rs    # read-only age/type insight report
    ├── snapshot.rs    # capture + diff for snapshot comparison
    ├── export.rs      # CSV/JSON focused report export
    ├── db.rs          # experimental rusqlite scan cache
    ├── platform.rs    # macOS open/reveal/trash adapters
    └── format.rs      # byte size + duration formatting
```

`app.rs` is intentionally large (~5k lines). It owns the `DiskMapApp` struct
and the `update()` / `ui()` / `save()` loop. **Before adding new UI code to
`app.rs`, check whether it belongs in one of the existing `app/` submodules
or a new `app/panels/*` module.** Sub-modules under `app/` are the preferred
home for new state; panel-render code stays in `app.rs` only when it shares
short-lived scope with the rest of the per-frame UI.

## Engineering Rules

### Test before commit

```bash
cargo clippy --all-targets --all-features -- -D warnings
cargo test --lib
```

Both must be clean. The CI profile uses `-D warnings` so any new clippy lint
will fail the build.

### Tests live next to the code

Use `#[cfg(test)] mod tests {}` at the bottom of each source file. The
project's `TestStorage` helper in `src/app.rs:3671-3686` mocks
`eframe::Storage` for preference round-trip tests. Reuse it; do not introduce
a parallel mock.

### Threading model

- The scan thread is the only long-running background thread besides the
  notify watcher.
- UI ↔ scan communication is one-way: `crossbeam_channel::Sender<ScanMessage>`.
  The scan thread never touches egui types.
- Watcher events arrive on a second channel; `app.update()` drains both with
  `try_recv()`. Do not block the UI thread.

### Persistence

User-facing preferences live in `eframe::Storage` (one string per key). The
key constants are grouped at the top of `src/app.rs` as `STORAGE_*`. To add a
new preference:

1. Add a `STORAGE_*` constant.
2. Read it in `restore_preferences` with a `parse_storage_bool` /
   `parse_stored_paths` / `theme_preference_name` style helper.
3. Write it in `save_preferences` (unconditional — Storage has no diff).
4. Extend the two round-trip tests in the `#[cfg(test)] mod tests` block of
   `app.rs`.

### What **not** to do (or: how to read SPEC.md correctly)

The SPEC has many unchecked Phase 10–18 items. They are **backlog, not
TODO**. Do not start any of them without an explicit ask. Specifically:

- **Do not** add a size-basis toggle. `TreeStore` stores one canonical size
  per node; switching basis would silently change every size the user sees.
  Phase 4 documents the prerequisite (dual-size fields + migration) — that
  has not been done.
- **Do not** add permanent delete. Move to Trash is the only destructive
  action, and it goes through `src/platform.rs::move_to_trash`. The
  `#[cfg(test)]` variant of `move_to_trash` is `remove_file` /
  `remove_dir_all` so tests can run hermetically; **never call it from
  production code paths.**
- **Do not** add network code. The app is offline by design (see README
  Privacy). All paths and reports stay local.
- **Do not** introduce platform-specific branching outside `src/platform.rs`
  and the `#[cfg(unix)]` / `#[cfg(target_os = "macos")]` blocks in
  `src/scanner.rs`. Use `size_basis_label()` / `size_basis_detail()` for
  size-basis messages instead of hard-coding "allocated" or "apparent".
- **Do not** enable SQLite cache by default. The setting exists; the default
  is `CacheMode::Disabled`. Cache mode changes are part of user-facing
  preferences.

### Adding a new analysis module

Pattern (see `duplicates.rs` / `insights.rs` / `snapshot.rs`):

1. Pure data types + a top-level `pub fn analyze_<thing>(...)` that takes
   `&mut TreeStore` and a focused `NodeId`. Returns `Option<Report>` (None
   when the root is missing).
2. Read-only — never mutate `TreeStore` except to call
   `ensure_sorted_children` for stable iteration order.
3. Bound the result size with a `limit` argument (e.g. `INSIGHT_REPORT_LIMIT`
   in `insights.rs`).
4. Wire the report into `app.rs` as an `Option<Report>` field on
   `DiskMapApp`, populate it on a manual user action (toolbar button), and
   render it from a `show_<thing>_report_section` method.
5. Add unit tests for empty root, single-file root, multi-file root, and
   the limit cutoff.

### Adding a new preference key

See the *Persistence* section above. Round-trip tests are mandatory.

### When to update SPEC.md

Update SPEC.md **only** when:

- Marking a roadmap item `[x]` after landing the code.
- Recording a deliberate deferral (see Phase 4 size-basis decision for the
  template — write the *why*, not just "later").

Do not add new phases without a discussion first.

## Dev environment

- Rust 1.85+, edition 2021. `rustup` not required but the project does not
  pin a toolchain; homebrew rustc is fine.
- macOS 13+ for `notify` / FSEvents; the code compiles on Linux but is
  untested there.
- No `rust-toolchain` file. Add one only when reproducing a specific build
  becomes a problem.
- No CI config. Add `.github/workflows/ci.yml` if / when you stop running
  clippy + tests locally before every push.

## Benchmarking

```bash
cargo bench
```

`benches/perf.rs` covers scan batch aggregation, parent-lookup hot path,
incremental search, and treemap layout. There is **no fixed-fixture
regression suite** (Phase 15). Treat published numbers as directional; do
not claim a "regression" without a stable input tree.

## Crash safety

Crash-safe local writes (preferences, history, snapshots, cleanup audit)
are **Phase 15** and not done. The current implementation relies on
`eframe::Storage` doing atomic writes; do not extend it to write large
state (history, snapshots) until the Phase 15 work lands.
