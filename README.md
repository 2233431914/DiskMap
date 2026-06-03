# DiskMap

A native macOS disk usage analyzer with a squarified treemap. Local-only, no
network, no telemetry. Built for personal use — fast scans, keyboard-driven
workflow, destructive actions guarded by review and protected-path checks.

Inspired by SpaceSniffer, written in Rust with `eframe`/`egui`.

## Status

**MVP feature-complete.** All Phase 1–9 roadmap items landed:

- Parallel scanning with `jwalk`, incremental UI refresh
- Squarified treemap with hover, search, filter, depth control
- Right-click: Open, Reveal in Finder, Copy Path, Move to Trash (with
  confirmation and protected-path validation)
- Safe scan options: hidden files, symlink policy, stay-on-filesystem
- Exclude rules (`.git`, `node_modules`, custom patterns)
- Real-time filesystem watch (FSEvents/kqueue) with debounced subtree rescans
- Snapshot comparison, duplicate-name report, file age/type insights
- Focused report export (CSV / JSON) with reproduction metadata
- Optional experimental SQLite scan cache (off by default)
- Recent + pinned scan roots, persisted user-facing options

Packaging, signing/notarization, accessibility, and CLI are **not** done — see
[SPEC.md](SPEC.md) for the full roadmap.

## Build & Run

Requires Rust 1.85+ (edition 2021). macOS is the primary target; the code
compiles for other platforms but only macOS has the platform `Move to Trash`
integration.

```bash
cargo run --release
```

`target/release/disk-map` is a standalone binary. There is no installer, no
notarized `.app` bundle yet.

### Dev commands

```bash
cargo test --lib                      # 129 unit tests, <1s on M-series
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release                 # optimized binary
cargo bench                           # criterion perf suite (perfnp paths)
```

## Usage (60 seconds)

1. Type or paste a directory path in the toolbar and press Enter (or click
   **Scan**).
2. Treemap shows the focused subtree. Hover for path/size tooltip, click to
   select, double-click a directory to drill in.
3. `[/]` keys change depth, `Backspace` returns to the previous focus, `Esc`
   clears search.
4. `Roots` menu collects the last 10 successful scan roots and stores pinned
   favorites for one-click repeat analysis.
5. Right-click a node for **Open / Reveal / Copy Path / Move to Trash**. Trash
   shows a confirmation with path, size, and affected item count.

## Privacy

Everything is local. No network calls, no analytics, no remote cache. The
optional SQLite cache lives in the eframe storage directory; reports and
exports are written to the current working directory and named
`disk-map-export-*` with a timestamp.

## License

No license file is included. Personal-use project; the source is published
for reference and so the author's future self / AI collaborators can read it.
Add a `LICENSE` file before any external redistribution.

## See also

- [SPEC.md](SPEC.md) — full product spec and roadmap (Phases 1–18)
- [AGENTS.md](AGENTS.md) — engineering conventions for human and AI contributors
