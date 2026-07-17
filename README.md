# DiskMap

[中文](#zh) | [English](#en)

<a id="zh"></a>

## 中文

DiskMap 是一款面向 macOS 和 Linux 桌面的原生、本地优先磁盘空间分析工具。它会扫描一个目录树，在内存中保留无损文件节点，并用可交互的 squarified treemap 把大文件和大目录直接显示出来。

这个项目 inspired by SpaceSniffer：核心体验是快速生成一张磁盘占用可视地图，并支持下钻、悬停查看详情、搜索，以及通过系统文件管理器打开或定位文件。DiskMap 不是 SpaceSniffer 的移植版，而是使用 Rust、`eframe` 和 `egui` 重新实现的一套 macOS/Linux 个人桌面工作流。

DiskMap 也是一个 vibe coding 项目。产品方向、UX 迭代和实现过程主要通过人类 + AI 的快速协作完成，同时用 Rust 单元测试、clippy 和手动桌面测试来保持日常使用所需的稳定性。

项目默认离线运行：没有网络请求，没有遥测，没有远程缓存。

### 功能概览

- 后台并行扫描本地目录
- 用矩形面积表达文件/文件夹大小
- 支持点击选择、双击进入目录、前进/后退、面包屑、深度控制、搜索和搜索过滤
- 通过系统桌面环境打开文件，或在 Finder / 文件管理器中定位文件
- 持久化轻量偏好设置，例如最近扫描根目录、收藏根目录、扫描选项、主题和 treemap 深度
- 清理相关动作走系统 Trash，不提供绕过 Trash 的永久删除

### 当前状态

**MVP feature-complete，UI 已收敛到 SpaceSniffer 风格的核心体验。** 当前适配平台是 macOS 和 Linux 桌面。Windows 目前不是目标平台，也没有经过测试。主 GUI 聚焦于扫描根目录选择、treemap 浏览、搜索和 open/reveal 操作：

- 基于 `jwalk` 的并行扫描和批量 UI 刷新
- Squarified treemap，支持 hover、search、filter 和 depth control
- 右键菜单：Open、Reveal in Finder / Open Containing Folder、Copy Path
- Settings 弹窗管理扫描根目录和扫描条件
- 安全扫描选项：隐藏文件、stay-on-filesystem；符号链接只显示、不跟随
- 排除规则：`.git`、`node_modules` 和自定义模式
- 基于 notify 的实时文件系统 watch，去抖后进行全量根目录重扫
- 最近扫描根目录和 pinned roots，并持久化用户可见选项

Headless CLI 和本地 macOS `.app` 打包路径已经可用。代码库中仍保留只读分析/导出模块和带保护逻辑的 cleanup 模块，但分析/导出模块不再暴露在主 GUI 中。

### 构建与运行

需要 Rust 1.85+，edition 2021。运行目标是 macOS 和 Linux。Linux 桌面还需要 `eframe`/`winit` 常见的原生 GUI 依赖，以及用于 `Open` / `Open Containing Folder` 的桌面 opener。

```bash
cargo run --release
```

`target/release/DiskMap` 是独立 GUI binary。

macOS bundle 脚本和打包说明位于 `scripts/` 与 `packaging/macos/`；当前仓库不包含独立的跨平台图标资源或 Linux launcher。

Ubuntu/Debian 本地 Linux 构建测试过的依赖：

```bash
sudo apt install build-essential pkg-config libx11-dev libxi-dev libxcursor-dev libxrandr-dev libxinerama-dev libgl1-mesa-dev libegl1-mesa-dev libwayland-dev libxkbcommon-dev libasound2-dev
```

在 Linux 上 watch 很大的目录树时，inotify watch limit 可能成为运行时瓶颈：

```bash
cat /proc/sys/fs/inotify/max_user_watches
sudo sysctl fs.inotify.max_user_watches=524288
```

#### macOS App Bundle

```bash
scripts/package-macos.sh
```

该脚本会构建 `target/dist/DiskMap.app` 和 `target/dist/DiskMap-<version>-macos-<arch>.zip`。默认签名是用于本地测试的 ad-hoc signature。Developer ID signing、notarization 和简单 DMG 流程见 [packaging/macos/README.md](packaging/macos/README.md)。

#### 开发命令

```bash
cargo test --lib                      # library unit tests
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release                 # optimized GUI binary (target/release/DiskMap)
scripts/package-macos.sh              # build target/dist/DiskMap.app + zip
cargo build --release --bin diskmap-cli  # optimized CLI binary
cargo bench --bench perf              # micro-benchmarks (synthetic 1k nodes)
cargo bench --bench large_tree        # large-tree suite with 1k/10k/100k fixtures
```

#### Headless CLI

如果需要脚本化或接入其他工具，可以使用单独的 `diskmap-cli` binary。它复用同一套 scanner：

```bash
diskmap-cli scan /path/to/dir                    # text to stdout
diskmap-cli scan /path/to/dir -f json            # JSON to stdout
diskmap-cli scan /path/to/dir -f csv -o out.csv  # CSV to file
diskmap-cli scan /path/to/dir -e .git -e target  # exclude patterns
diskmap-cli scan /path/to/dir --max-depth 3      # cap depth
diskmap-cli scan /path/to/dir --include-hidden   # dotfiles
diskmap-cli scan /path/to/dir --sort-by size     # largest first
```

符号链接会被列出但不会跟随；旧的 `--follow-symlinks` 参数会明确报错。设置 `DISKMAP_SCAN_TRACE=1` 可输出 scanner timing。CLI 没有 preferences、profiles 或 destructive actions，是只读工具。

### 60 秒上手

1. 点击 settings gear，根据需要编辑 scan root，然后点击 **Start Scan**。默认 scan root 是 home directory。
2. Treemap 显示当前 focused subtree。悬停查看 path/size tooltip，点击选择，双击目录进入。
3. `[` / `]` 调整深度，`Backspace` 返回上一个 focus，`Esc` 清除 selection/search 或关闭 Settings。
4. `Roots` 菜单保存最近 10 个成功扫描的根目录，并支持 pinned favorites。
5. 右键节点，在 macOS 上选择 **Open / Reveal in Finder**，在 Linux 上选择 **Open Containing Folder**，也可以 **Copy Path**。

### 快捷键

| Key         | Action                              |
|-------------|-------------------------------------|
| `Enter`     | 进入选中的目录                      |
| `Backspace` | 返回                                |
| `Alt+←/→`   | 后退 / 前进                         |
| `[` / `]`   | 降低 / 提高 treemap 深度            |
| `Esc`       | 清除 selection/search 或关闭 Settings |

### 隐私

所有行为都在本地完成。没有网络请求，没有 analytics，没有 remote cache。Crash-safe 的本地 preferences/state 存储在 DiskMap 的 app data directory。Linux 上，如果 `XDG_DATA_HOME` 是绝对路径，则 app data directory 是 `$XDG_DATA_HOME/disk-map`，否则是 `~/.local/share/disk-map`。

### 开源协议

DiskMap 使用 [`GPL-3.0-or-later`](LICENSE) 授权。

这意味着你可以使用、研究、修改和分发本项目；如果你分发基于 DiskMap 源码修改或衍生出来的版本，需要继续使用 GPL 兼容的开源条款，并提供对应源码。

`or-later` 表示使用者可以选择 GPL v3，或自由软件基金会未来发布的 GPL 后续版本。GPL 是真正的 FOSS / open source 许可证；它允许商业使用和收费分发，但要求分发修改版时继续开放源码。

<a id="en"></a>

## English

DiskMap is a native, local-first disk usage analyzer for macOS and Linux. It scans a directory tree, keeps every file node in memory, and renders an interactive squarified treemap so large files and folders become visible immediately.

The project is inspired by SpaceSniffer: the core workflow is a fast visual map of disk usage with drill-down navigation, hover details, search, and direct open/reveal actions. DiskMap is not a SpaceSniffer port; it is a Rust `eframe`/`egui` reinterpretation built for a personal macOS/Linux desktop workflow.

DiskMap is also a vibe coding project. Product direction, UX iteration, and implementation have been developed through fast human + AI coding sessions, with Rust tests, clippy, and manual desktop testing used to keep the result stable enough for personal daily use.

Local-only by design: no network calls, no telemetry, no remote cache.

### What It Does

- Scans local directories in the background with parallel traversal
- Draws a squarified treemap where rectangle area maps to file/folder size
- Supports click selection, double-click drill-down, back/forward navigation, breadcrumbs, depth control, search, and search filtering
- Opens files or reveals containing folders through the native desktop shell
- Persists lightweight preferences such as recent roots, pinned roots, scan options, theme, and treemap depth
- Keeps destructive behavior guarded: cleanup actions go through platform Trash rather than permanent deletion

### Status

**MVP feature-complete, UI simplified toward the SpaceSniffer core.** Current runtime targets are macOS and Linux desktop. Windows is not currently targeted or tested. The main app focuses on scan root selection, treemap browsing, search, and open/reveal actions:

- Parallel scanning with `jwalk`, batched UI refresh
- Squarified treemap with hover, search, filter, depth control
- Right-click: Open, Reveal in Finder / Open Containing Folder, Copy Path
- Settings popup for scan root and scan conditions
- Safe scan options: hidden files and stay-on-filesystem; symlinks are shown but not followed
- Exclude rules (`.git`, `node_modules`, custom patterns)
- Real-time filesystem watch with debounced full-root rescans
- Recent + pinned scan roots, persisted user-facing options

The headless CLI and local macOS `.app` packaging path are available. The codebase still contains read-only analysis/export modules and guarded cleanup logic, but the analysis/export modules are not exposed in the main GUI.

### Build & Run

Requires Rust 1.85+ (edition 2021). macOS and Linux are supported runtime targets. Linux desktops also need the usual native GUI libraries used by `eframe`/`winit` and a desktop opener for `Open` / `Open Containing Folder`.

```bash
cargo run --release
```

`target/release/DiskMap` is a standalone GUI binary.

The macOS bundle script and packaging notes live in `scripts/` and `packaging/macos/`. This repository does not currently ship separate cross-platform icon assets or a Linux launcher.

On Ubuntu/Debian, the native dependencies tested for local Linux builds are:

```bash
sudo apt install build-essential pkg-config libx11-dev libxi-dev libxcursor-dev libxrandr-dev libxinerama-dev libgl1-mesa-dev libegl1-mesa-dev libwayland-dev libxkbcommon-dev libasound2-dev
```

For very large watched trees on Linux, the inotify watch limit can be the runtime bottleneck:

```bash
cat /proc/sys/fs/inotify/max_user_watches
sudo sysctl fs.inotify.max_user_watches=524288
```

#### macOS App Bundle

```bash
scripts/package-macos.sh
```

This builds `target/dist/DiskMap.app` and `target/dist/DiskMap-<version>-macos-<arch>.zip`. The default signature is ad-hoc for local testing. Developer ID signing, notarization, and a simple DMG are documented in [packaging/macos/README.md](packaging/macos/README.md).

#### Dev Commands

```bash
cargo test --lib                      # library unit tests
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release                 # optimized GUI binary (target/release/DiskMap)
scripts/package-macos.sh              # build target/dist/DiskMap.app + zip
cargo build --release --bin diskmap-cli  # optimized CLI binary
cargo bench --bench perf              # micro-benchmarks (synthetic 1k nodes)
cargo bench --bench large_tree        # large-tree suite with 1k/10k/100k fixtures
```

#### Headless CLI

For scripting and piping into other tools, there is a separate `diskmap-cli` binary that reuses the same scanner:

```bash
diskmap-cli scan /path/to/dir                    # text to stdout
diskmap-cli scan /path/to/dir -f json            # JSON to stdout
diskmap-cli scan /path/to/dir -f csv -o out.csv  # CSV to file
diskmap-cli scan /path/to/dir -e .git -e target  # exclude patterns
diskmap-cli scan /path/to/dir --max-depth 3      # cap depth
diskmap-cli scan /path/to/dir --include-hidden   # dotfiles
diskmap-cli scan /path/to/dir --sort-by size     # largest first
```

Symlinks are listed but not followed; the legacy `--follow-symlinks` flag is rejected. Set `DISKMAP_SCAN_TRACE=1` to print scanner timing. The CLI has no preferences, profiles, or destructive actions; it is read-only.

### Usage in 60 Seconds

1. Click the settings gear, edit the scan root if needed, and click **Start Scan**. The default scan root is your home directory.
2. Treemap shows the focused subtree. Hover for path/size tooltip, click to select, double-click a directory to drill in.
3. `[` / `]` change depth, `Backspace` returns to the previous focus, and `Esc` clears selection/search or closes Settings.
4. The `Roots` menu collects the last 10 successful scan roots and stores pinned favorites for repeat analysis.
5. Right-click a node for **Open / Reveal in Finder** on macOS or **Open Containing Folder** on Linux, plus **Copy Path**.

### Keyboard Shortcuts

| Key         | Action                                  |
|-------------|-----------------------------------------|
| `Enter`     | Enter selected directory                |
| `Backspace` | Navigate back                           |
| `Alt+←/→`   | Navigate back / forward                 |
| `[` / `]`   | Decrease / increase treemap depth       |
| `Esc`       | Clear selection / search / close Settings |

### Privacy

Everything is local. No network calls, no analytics, no remote cache. Crash-safe local preferences/state live in DiskMap's app data directory. On Linux, the app data directory is `$XDG_DATA_HOME/disk-map` when `XDG_DATA_HOME` is an absolute path, otherwise `~/.local/share/disk-map`.

### License

DiskMap is licensed under [`GPL-3.0-or-later`](LICENSE).

You may use, study, modify, and distribute this project. If you distribute a modified or derivative version based on DiskMap's source code, you must keep it under GPL-compatible open source terms and provide the corresponding source code.

`or-later` means recipients may choose GPL v3 or a later GPL version published by the Free Software Foundation. GPL is a real FOSS / open source license; it allows commercial use and paid distribution, but requires distributed modified versions to remain open source.

## See Also

- [SPEC.md](SPEC.md) - full product spec and roadmap (Phases 1-18)
- [AGENTS.md](AGENTS.md) - engineering conventions for human and AI contributors
