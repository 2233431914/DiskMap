# DiskMap

A native disk usage analyzer with a squarified treemap for macOS and Linux.
Local-only, no network, no telemetry. Built for personal use — fast scans,
keyboard-driven workflow, destructive actions guarded by review and
protected-path checks.

Inspired by SpaceSniffer, written in Rust with `eframe`/`egui`.

## Status

**MVP feature-complete.** All Phase 1–9 roadmap items landed:

- Parallel scanning with `jwalk`, incremental UI refresh
- Squarified treemap with hover, search, filter, depth control
- Right-click: Open, Reveal in Finder / Open Containing Folder, Copy Path,
  Move to Trash (with confirmation and protected-path validation)
- Safe scan options: hidden files, symlink policy, stay-on-filesystem
- Exclude rules (`.git`, `node_modules`, custom patterns)
- Real-time filesystem watch with debounced subtree rescans
- Snapshot comparison, duplicate-name report, file age/type insights
- Focused report export (CSV / JSON) with reproduction metadata
- Optional experimental SQLite scan cache (off by default)
- Recent + pinned scan roots, persisted user-facing options

The headless CLI and local macOS `.app` packaging path are available. Linux
runs as a native desktop binary; distro packaging is not yet part of the
roadmap. Accessibility hardening and a full signed/notarized public release
checklist remain roadmap work — see [SPEC.md](SPEC.md) for details.

## Build & Run

Requires Rust 1.85+ (edition 2021). macOS and Linux are supported runtime
targets. Linux desktops also need the usual native GUI libraries used by
`eframe`/`winit`, a desktop opener for `Open` / `Open Containing Folder`,
and freedesktop Trash support for `Move to Trash`. Trash may fail in
headless sessions or on filesystems that do not expose a compatible Trash
location.

```bash
cargo run --release
```

`target/release/disk-map` is a standalone binary.

On Ubuntu/Debian, the native dependencies tested for local Linux builds are:

```bash
sudo apt install build-essential pkg-config libx11-dev libxi-dev libxcursor-dev libxrandr-dev libxinerama-dev libgl1-mesa-dev libegl1-mesa-dev libwayland-dev libxkbcommon-dev libasound2-dev
```

For very large watched trees on Linux, the inotify watch limit can be the
runtime bottleneck:

```bash
cat /proc/sys/fs/inotify/max_user_watches
sudo sysctl fs.inotify.max_user_watches=524288
```

### macOS App Bundle

```bash
scripts/package-macos.sh
```

This builds `target/dist/DiskMap.app` and
`target/dist/DiskMap-<version>-macos-<arch>.zip`. The default signature is
ad-hoc for local testing. Developer ID signing, notarization, and a simple DMG
are documented in [packaging/macos/README.md](packaging/macos/README.md).

### Dev commands

```bash
cargo test --lib                      # library unit tests
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release                 # optimized GUI binary (target/release/disk-map)
scripts/package-macos.sh              # build target/dist/DiskMap.app + zip
cargo build --release --bin diskmap-cli  # optimized CLI binary
cargo bench --bench perf              # micro-benchmarks (synthetic 1k nodes)
cargo bench --bench large_tree        # large-tree suite with 1k/10k/100k fixtures
```

### Headless CLI

For scripting and piping into other tools, there's a separate
`diskmap-cli` binary that reuses the same scanner:

```bash
diskmap-cli scan /path/to/dir                    # text to stdout
diskmap-cli scan /path/to/dir -f json            # JSON to stdout
diskmap-cli scan /path/to/dir -f csv -o out.csv  # CSV to file
diskmap-cli scan /path/to/dir -e .git -e target  # exclude patterns
diskmap-cli scan /path/to/dir --max-depth 3      # cap depth
diskmap-cli scan /path/to/dir --include-hidden   # dotfiles
diskmap-cli scan /path/to/dir --follow-symlinks  # symlinks
diskmap-cli scan /path/to/dir --sort-by size     # largest first
```

`2>/dev/null` to silence the scanner's perf log. The CLI has no
preferences, no profiles, no destructive actions — read-only.

## Usage (60 seconds)

1. Type or paste a directory path in the toolbar and press Enter (or click
   **Scan**).
2. Treemap shows the focused subtree. Hover for path/size tooltip, click to
   select, double-click a directory to drill in.
3. `[/]` keys change depth, `Backspace` returns to the previous focus, `Esc`
   clears search.
4. `Roots` menu collects the last 10 successful scan roots and stores pinned
   favorites for one-click repeat analysis.
5. Right-click a node for **Open / Reveal in Finder** on macOS or
   **Open Containing Folder** on Linux, plus **Copy Path / Move to Trash**.
   Trash shows a confirmation with path, size, and affected item count.

## Keyboard shortcuts

| Key            | Action                                  |
|----------------|-----------------------------------------|
| `Enter`        | Enter selected directory                |
| `Backspace`    | Navigate back                           |
| `Alt+←/→`     | Navigate back / forward                  |
| `[` / `]`      | Decrease / increase treemap depth       |
| `Esc`          | Clear selection / search / close palette |
| `Cmd+K` / `Ctrl+K` | Open command palette                |

## Privacy

Everything is local. No network calls, no analytics, no remote cache. The
optional SQLite cache lives in DiskMap's app data directory alongside
crash-safe local preferences/state; reports and exports are written to the
current working directory and named `disk-map-export-*` with a timestamp.
On Linux, the app data directory is `$XDG_DATA_HOME/disk-map` when
`XDG_DATA_HOME` is an absolute path, otherwise `~/.local/share/disk-map`.

## License

No license file is included. Personal-use project; the source is published
for reference and so the author's future self / AI collaborators can read it.
Add a `LICENSE` file before any external redistribution.

## See also

- [SPEC.md](SPEC.md) — full product spec and roadmap (Phases 1–18)
- [AGENTS.md](AGENTS.md) — engineering conventions for human and AI contributors
