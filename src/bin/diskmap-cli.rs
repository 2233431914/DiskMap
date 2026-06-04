//! Headless command-line interface for DiskMap.
//!
//! Usage:
//!   diskmap-cli scan <PATH> [OPTIONS]
//!
//! Reuses the GUI app's scanner and tree building code — no parallel
//! implementation. The output format is a flat list of nodes (path,
//! size, kind) suitable for piping into jq, R, pandas, or any other
//! downstream tool. For the GUI, use the main `disk-map` binary.
//!
//! Design notes:
//!  - No external arg-parser dependency. Hand-rolled parsing keeps the
//!    dep surface small for a self-use tool.
//!  - No persistence. The CLI is intentionally read-only: it scans,
//!    optionally writes the result to a file, and exits. It does not
//!    touch preferences, profiles, or the rule store.
//!  - Output is deterministic (sorted by path) so that diffs between
//!    runs are stable.

use disk_map::format::format_bytes;
use disk_map::scanner::{parse_exclude_patterns, scan_path_to_tree, ScanOptions};
use disk_map::tree::NodeKind;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "\
diskmap-cli — headless scan and export

USAGE:
    diskmap-cli scan <PATH> [OPTIONS]

OPTIONS:
    -f, --format <text|json|csv>    Output format (default: text)
    -o, --output <FILE>             Write to FILE instead of stdout
    -e, --exclude <PATTERN>         Exclude pattern (repeatable, comma/semi/newline
                                    separated when given once)
    --max-depth <N>                 Limit how deep the scan recurses (1+)
    --include-hidden                Include dotfiles
    --follow-symlinks                Follow symlinks (off by default)
    --stay-on-filesystem            Don't cross filesystem boundaries
    --sort-by <path|size>           Output row order (default: path)
    -h, --help                      Show this help
";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Text,
    Json,
    Csv,
}

impl Format {
    fn parse(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "text" | "txt" => Ok(Format::Text),
            "json" => Ok(Format::Json),
            "csv" => Ok(Format::Csv),
            other => Err(format!(
                "unknown format '{other}' (expected text|json|csv)"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortBy {
    Path,
    Size,
}

impl SortBy {
    fn parse(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "path" => Ok(SortBy::Path),
            "size" => Ok(SortBy::Size),
            other => Err(format!("unknown sort-by '{other}' (expected path|size)")),
        }
    }
}

#[derive(Debug)]
struct CliOptions {
    command: Command,
}

#[derive(Debug)]
enum Command {
    Scan {
        path: PathBuf,
        format: Format,
        output: Option<PathBuf>,
        scan_options: ScanOptions,
        sort_by: SortBy,
        max_depth: Option<usize>,
    },
    Help,
}

/// Parse argv. Returns a structured error on bad input — never panics
/// or exits on its own. The caller decides how to surface the error.
fn parse_args(argv: &[String]) -> Result<CliOptions, String> {
    if argv.is_empty() {
        return Ok(CliOptions {
            command: Command::Help,
        });
    }
    if argv.iter().any(|a| a == "-h" || a == "--help") {
        return Ok(CliOptions {
            command: Command::Help,
        });
    }

    let sub = argv
        .first()
        .ok_or_else(|| "missing subcommand".to_string())?;
    match sub.as_str() {
        "scan" => parse_scan(&argv[1..]),
        "help" | "-h" | "--help" => Ok(CliOptions {
            command: Command::Help,
        }),
        other => Err(format!(
            "unknown subcommand '{other}' (expected: scan)"
        )),
    }
}

fn parse_scan(argv: &[String]) -> Result<CliOptions, String> {
    if argv.is_empty() {
        return Err("scan requires a PATH argument".to_string());
    }
    let path = PathBuf::from(&argv[0]);
    if !path.exists() {
        return Err(format!("path does not exist: {}", path.display()));
    }

    let mut format = Format::Text;
    let mut output: Option<PathBuf> = None;
    let mut exclude_patterns: Vec<String> = Vec::new();
    let mut max_depth: Option<usize> = None;
    let mut include_hidden = ScanOptions::default().include_hidden;
    let mut follow_symlinks = ScanOptions::default().follow_symlinks;
    let mut stay_on_filesystem = ScanOptions::default().stay_on_filesystem;
    let mut sort_by = SortBy::Path;

    let mut i = 1;
    while i < argv.len() {
        let arg = &argv[i];
        match arg.as_str() {
            "-f" | "--format" => {
                let value = argv
                    .get(i + 1)
                    .ok_or_else(|| format!("{arg} requires a value"))?;
                format = Format::parse(value)?;
                i += 2;
            }
            "-o" | "--output" => {
                let value = argv
                    .get(i + 1)
                    .ok_or_else(|| format!("{arg} requires a value"))?;
                output = Some(PathBuf::from(value));
                i += 2;
            }
            "-e" | "--exclude" => {
                let value = argv
                    .get(i + 1)
                    .ok_or_else(|| format!("{arg} requires a value"))?;
                exclude_patterns.push(value.clone());
                i += 2;
            }
            "--max-depth" => {
                let value = argv
                    .get(i + 1)
                    .ok_or_else(|| format!("{arg} requires a value"))?;
                let n: usize = value
                    .parse()
                    .map_err(|_| format!("--max-depth must be a positive integer, got '{value}'"))?;
                if n == 0 {
                    return Err("--max-depth must be >= 1".to_string());
                }
                max_depth = Some(n);
                i += 2;
            }
            "--include-hidden" => {
                include_hidden = true;
                i += 1;
            }
            "--follow-symlinks" => {
                follow_symlinks = true;
                i += 1;
            }
            "--stay-on-filesystem" => {
                stay_on_filesystem = true;
                i += 1;
            }
            "--sort-by" => {
                let value = argv
                    .get(i + 1)
                    .ok_or_else(|| format!("{arg} requires a value"))?;
                sort_by = SortBy::parse(value)?;
                i += 2;
            }
            "-h" | "--help" => {
                return Ok(CliOptions {
                    command: Command::Help,
                });
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown flag '{other}'"));
            }
            other => {
                return Err(format!("unexpected positional argument '{other}'"));
            }
        }
    }

    let scan_options = ScanOptions {
        exclude_patterns: parse_exclude_patterns(&exclude_patterns.join(",")),
        include_hidden,
        follow_symlinks,
        stay_on_filesystem,
        ..ScanOptions::default()
    };

    Ok(CliOptions {
        command: Command::Scan {
            path,
            format,
            output,
            scan_options,
            sort_by,
            max_depth,
        },
    })
}

/// Flat row we emit per tree node. Built after the scan to keep
/// formatting out of the scan loop.
#[derive(Debug, Clone)]
struct Row {
    path: String,
    size: u64,
    kind: NodeKind,
}

fn collect_rows(
    tree: &mut disk_map::tree::TreeStore,
    root: &Path,
    max_depth: Option<usize>,
) -> Vec<Row> {
    let mut rows = Vec::new();
    let Some(root_id) = tree.root else {
        return rows;
    };
    walk(tree, root_id, 1, max_depth, &mut rows);
    let _ = root; // for future use
    rows
}

fn walk(
    tree: &mut disk_map::tree::TreeStore,
    node_id: usize,
    depth: usize,
    max_depth: Option<usize>,
    rows: &mut Vec<Row>,
) {
    let node = tree.node(node_id).clone();
    let path = tree
        .node_real_path(node_id)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| node.name.clone());
    rows.push(Row {
        path,
        size: node.size,
        kind: node.kind,
    });
    if let Some(max) = max_depth {
        if depth >= max {
            return;
        }
    }
    tree.ensure_sorted_children(node_id);
    let children = tree.sorted_children(node_id).to_vec();
    for child in children {
        walk(tree, child, depth + 1, max_depth, rows);
    }
}

fn sort_rows(rows: &mut [Row], sort_by: SortBy) {
    match sort_by {
        SortBy::Path => rows.sort_by(|a, b| a.path.cmp(&b.path)),
        SortBy::Size => rows.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.path.cmp(&b.path))),
    }
}

fn write_text<W: Write>(rows: &[Row], w: &mut W) -> io::Result<()> {
    for row in rows {
        let kind = match row.kind {
            NodeKind::File => "F",
            NodeKind::Dir => "D",
            NodeKind::Symlink => "L",
            NodeKind::Error => "E",
            NodeKind::Aggregate => "A",
        };
        writeln!(w, "{:<10}  {}  {}", format_bytes(row.size), kind, row.path)?;
    }
    Ok(())
}

fn write_csv<W: Write>(rows: &[Row], w: &mut W) -> io::Result<()> {
    writeln!(w, "path,size,kind")?;
    for row in rows {
        let kind = match row.kind {
            NodeKind::File => "file",
            NodeKind::Dir => "dir",
            NodeKind::Symlink => "symlink",
            NodeKind::Error => "error",
            NodeKind::Aggregate => "aggregate",
        };
        // Minimal CSV escaping: no commas or quotes in our paths in
        // practice. If a path has a quote, double it (RFC 4180).
        let escaped = row.path.replace('"', "\"\"");
        writeln!(w, "\"{}\",{},{}", escaped, row.size, kind)?;
    }
    Ok(())
}

fn write_json<W: Write>(rows: &[Row], w: &mut W) -> io::Result<()> {
    writeln!(w, "[")?;
    for (i, row) in rows.iter().enumerate() {
        let kind = match row.kind {
            NodeKind::File => "file",
            NodeKind::Dir => "dir",
            NodeKind::Symlink => "symlink",
            NodeKind::Error => "error",
            NodeKind::Aggregate => "aggregate",
        };
        // Naive JSON escaping. Paths shouldn't contain control chars
        // or unescaped quotes in practice, but be defensive.
        let escaped = json_escape(&row.path);
        let sep = if i + 1 < rows.len() { "," } else { "" };
        writeln!(
            w,
            "  {{\"path\": \"{escaped}\", \"size\": {}, \"kind\": \"{kind}\"}}{sep}",
            row.size
        )?;
    }
    writeln!(w, "]")?;
    Ok(())
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn run_scan(
    path: PathBuf,
    format: Format,
    output: Option<PathBuf>,
    scan_options: ScanOptions,
    sort_by: SortBy,
    max_depth: Option<usize>,
) -> u8 {
    // The scanner writes a "Scan N completed in ..." perf line to
    // stderr. That's fine for the GUI (debug noise in the same
    // terminal) but the CLI's structured output is on stdout. The
    // perf line is still on stderr — the user can `2>/dev/null` it if
    // they want to silence it. We don't suppress it here so that
    // calling the CLI without redirecting still gives some feedback.
    let mut tree = match scan_path_to_tree(path.clone(), scan_options) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("diskmap-cli: scan failed: {e}");
            return 2;
        }
    };
    let mut rows = collect_rows(&mut tree, &path, max_depth);
    sort_rows(&mut rows, sort_by);

    let mut writer: Box<dyn Write> = match output {
        Some(path) => match std::fs::File::create(&path) {
            Ok(f) => Box::new(f),
            Err(e) => {
                eprintln!(
                    "diskmap-cli: could not open output file {}: {e}",
                    path.display()
                );
                return 2;
            }
        },
        None => Box::new(io::stdout().lock()),
    };

    let result = match format {
        Format::Text => write_text(&rows, &mut writer),
        Format::Csv => write_csv(&rows, &mut writer),
        Format::Json => write_json(&rows, &mut writer),
    };
    if let Err(e) = result {
        eprintln!("diskmap-cli: write failed: {e}");
        return 2;
    }
    if let Err(e) = writer.flush() {
        eprintln!("diskmap-cli: flush failed: {e}");
        return 2;
    }
    0
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let options = match parse_args(&argv) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("diskmap-cli: {e}");
            eprintln!();
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match options.command {
        Command::Help => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Command::Scan {
            path,
            format,
            output,
            scan_options,
            sort_by,
            max_depth,
        } => {
            let code = run_scan(path, format, output, scan_options, sort_by, max_depth);
            ExitCode::from(code)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn empty_args_returns_help() {
        let result = parse_args(&args(&[])).unwrap();
        assert!(matches!(result.command, Command::Help));
    }

    #[test]
    fn help_flag_returns_help() {
        let result = parse_args(&args(&["--help"])).unwrap();
        assert!(matches!(result.command, Command::Help));
        let result = parse_args(&args(&["-h"])).unwrap();
        assert!(matches!(result.command, Command::Help));
    }

    #[test]
    fn scan_subcommand_parses_path() {
        // Use a path that exists on most systems; if not, the
        // parse_args check will fail.
        let result = parse_args(&args(&["scan", "/tmp"]));
        // /tmp may or may not exist; either way, parsing should work
        // OR fail with the existence error.
        if let Err(e) = result {
            assert!(e.contains("does not exist"), "got: {e}");
            return;
        }
        let opts = result.unwrap();
        match opts.command {
            Command::Scan { path, format, .. } => {
                assert_eq!(path, PathBuf::from("/tmp"));
                assert_eq!(format, Format::Text);
            }
            _ => panic!("expected Scan"),
        }
    }

    #[test]
    fn scan_subcommand_rejects_unknown_subcommand() {
        let result = parse_args(&args(&["frobnicate", "/tmp"]));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown subcommand"));
    }

    #[test]
    fn scan_subcommand_rejects_missing_path() {
        let result = parse_args(&args(&["scan"]));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("PATH"));
    }

    #[test]
    fn format_parsing_accepts_canonical_and_aliases() {
        assert_eq!(Format::parse("text").unwrap(), Format::Text);
        assert_eq!(Format::parse("TEXT").unwrap(), Format::Text);
        assert_eq!(Format::parse("txt").unwrap(), Format::Text);
        assert_eq!(Format::parse("json").unwrap(), Format::Json);
        assert_eq!(Format::parse("JSON").unwrap(), Format::Json);
        assert_eq!(Format::parse("csv").unwrap(), Format::Csv);
    }

    #[test]
    fn format_parsing_rejects_unknown() {
        let err = Format::parse("yaml").unwrap_err();
        assert!(err.contains("unknown format"));
    }

    #[test]
    fn sort_by_parsing() {
        assert_eq!(SortBy::parse("path").unwrap(), SortBy::Path);
        assert_eq!(SortBy::parse("size").unwrap(), SortBy::Size);
        let err = SortBy::parse("mtime").unwrap_err();
        assert!(err.contains("unknown sort-by"));
    }

    #[test]
    fn scan_with_all_options() {
        // Mock-path with /tmp
        let result = parse_args(&args(&[
            "scan",
            "/tmp",
            "-f",
            "json",
            "-o",
            "/tmp/out.json",
            "-e",
            ".git",
            "-e",
            "target",
            "--max-depth",
            "3",
            "--include-hidden",
            "--follow-symlinks",
            "--sort-by",
            "size",
        ]));
        if result.is_err() {
            // /tmp missing on this system — bail
            return;
        }
        let opts = result.unwrap();
        match opts.command {
            Command::Scan {
                path,
                format,
                output,
                scan_options,
                sort_by,
                max_depth,
            } => {
                assert_eq!(path, PathBuf::from("/tmp"));
                assert_eq!(format, Format::Json);
                assert_eq!(output, Some(PathBuf::from("/tmp/out.json")));
                assert_eq!(sort_by, SortBy::Size);
                assert_eq!(max_depth, Some(3));
                assert!(scan_options.include_hidden);
                assert!(scan_options.follow_symlinks);
                assert_eq!(scan_options.exclude_patterns, vec![".git", "target"]);
            }
            _ => panic!("expected Scan"),
        }
    }

    #[test]
    fn scan_rejects_zero_max_depth() {
        let result = parse_args(&args(&["scan", "/tmp", "--max-depth", "0"]));
        if result.is_ok() {
            panic!("should reject --max-depth 0");
        }
        let msg = result.unwrap_err();
        assert!(
            msg.contains("max-depth") || msg.contains("does not exist"),
            "got: {msg}"
        );
    }

    #[test]
    fn scan_rejects_unknown_flag() {
        let result = parse_args(&args(&["scan", "/tmp", "--frobnicate"]));
        if result.is_ok() {
            panic!("should reject --frobnicate");
        }
        let msg = result.unwrap_err();
        assert!(
            msg.contains("unknown flag") || msg.contains("does not exist"),
            "got: {msg}"
        );
    }

    #[test]
    fn json_escaping_handles_specials() {
        assert_eq!(json_escape("plain"), "plain");
        assert_eq!(json_escape("with \"quote\""), "with \\\"quote\\\"");
        assert_eq!(json_escape("back\\slash"), "back\\\\slash");
        assert_eq!(json_escape("line\nbreak"), "line\\nbreak");
        assert_eq!(json_escape("tab\there"), "tab\\there");
        assert_eq!(json_escape("cr\rmore"), "cr\\rmore");
    }

    #[test]
    fn row_sorting_by_path_is_stable_alphabetical() {
        let mut rows = vec![
            Row { path: "/c".into(), size: 1, kind: NodeKind::File },
            Row { path: "/a".into(), size: 3, kind: NodeKind::File },
            Row { path: "/b".into(), size: 2, kind: NodeKind::File },
        ];
        sort_rows(&mut rows, SortBy::Path);
        assert_eq!(rows[0].path, "/a");
        assert_eq!(rows[1].path, "/b");
        assert_eq!(rows[2].path, "/c");
    }

    #[test]
    fn row_sorting_by_size_descending_breaks_ties_by_path() {
        let mut rows = vec![
            Row { path: "/b".into(), size: 100, kind: NodeKind::File },
            Row { path: "/a".into(), size: 100, kind: NodeKind::File },
            Row { path: "/c".into(), size: 50, kind: NodeKind::File },
        ];
        sort_rows(&mut rows, SortBy::Size);
        // /a and /b tied at 100; tiebreak by path
        assert_eq!(rows[0].path, "/a");
        assert_eq!(rows[1].path, "/b");
        assert_eq!(rows[2].path, "/c");
    }
}
