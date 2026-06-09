//! Rule engine for read-only analysis.
//!
//! The rule engine is a *foundation* for Phase 18. It defines:
//!  - The `Rule` data model: id, name, description, category, predicate,
//!    enabled flag.
//!  - The `RulePredicate` enum: per-node predicates that decide whether
//!    a given tree node matches the rule.
//!  - A `RuleSet` collection with add/remove/get/enable/disable.
//!  - A `default_ruleset()` covering the high-value cases the existing
//!    `duplicates.rs` / `insights.rs` / `cleanup.rs` modules detect, so
//!    the user sees a sensible out-of-the-box ruleset on first launch.
//!  - `evaluate_rules(rules, tree, root, limit) -> Vec<RuleHit>` that
//!    walks the tree, applies each enabled rule's predicate, and
//!    returns at most `limit` hits. **Read-only** — never mutates the
//!    tree except for `ensure_sorted_children` (deterministic
//!    iteration order, same convention as `duplicates.rs` /
//!    `insights.rs`).
//!
//! The full duplicate-finder predicate (file groups with the same name
//! and size) is intentionally out of scope here — it requires
//! post-walk aggregation, not per-node matching. It can be added as a
//! new `RulePredicate` variant later without breaking the data model.

use crate::platform;
use crate::tree::{Node, NodeId, NodeKind, TreeStore};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

/// Cap on how many hits `evaluate_rules` will collect and return.
/// Matches the convention in `insights::INSIGHT_REPORT_LIMIT` so the
/// sidebar section doesn't grow unbounded for very large trees.
pub const INSIGHT_REPORT_LIMIT_FROM_RULES: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RuleCategory {
    /// A read-only observation about a node (e.g. "this file is
    /// unusually large for a hidden file").
    AnomalyHint,
    /// A node the user may want to delete / move to trash. The engine
    /// itself never deletes anything.
    CleanupCandidate,
    /// A path that is protected from destructive actions regardless
    /// of any other rule.
    ProtectedPath,
}

impl RuleCategory {
    pub fn label(self) -> &'static str {
        match self {
            Self::AnomalyHint => "anomaly hint",
            Self::CleanupCandidate => "cleanup candidate",
            Self::ProtectedPath => "protected path",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RulePredicate {
    /// Match a file whose size is at least `min_size` bytes.
    LargeFile { min_size: u64 },
    /// Match a file whose modified-time is at least `min_age_days`
    /// old AND size is at least `min_size` bytes. Unknown mtime never
    /// matches.
    OldFile { min_age_days: u64, min_size: u64 },
    /// Match hidden files or directories (name starts with `.`).
    Hidden,
    /// Match symlink nodes.
    Symlink,
    /// Match any node whose real path starts with the given pattern.
    AlwaysProtected { path_pattern: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: RuleCategory,
    pub predicate: RulePredicate,
    pub enabled: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleSet {
    pub rules: Vec<Rule>,
}

impl RuleSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, rule: Rule) {
        if !self.rules.iter().any(|r| r.id == rule.id) {
            self.rules.push(rule);
        }
    }

    pub fn remove(&mut self, id: &str) -> Option<Rule> {
        let index = self.rules.iter().position(|r| r.id == id)?;
        Some(self.rules.remove(index))
    }

    pub fn get(&self, id: &str) -> Option<&Rule> {
        self.rules.iter().find(|r| r.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Rule> {
        self.rules.iter_mut().find(|r| r.id == id)
    }

    pub fn enable(&mut self, id: &str) -> bool {
        if let Some(rule) = self.get_mut(id) {
            rule.enabled = true;
            true
        } else {
            false
        }
    }

    pub fn disable(&mut self, id: &str) -> bool {
        if let Some(rule) = self.get_mut(id) {
            rule.enabled = false;
            true
        } else {
            false
        }
    }

    pub fn enabled_count(&self) -> usize {
        self.rules.iter().filter(|r| r.enabled).count()
    }
}

/// Per-evaluation context. Created by `evaluate_rules`; the current
/// implementation only needs the current wall-clock time, but adding
/// fields here (e.g. "user-configured project root") is a non-breaking
/// change.
#[derive(Debug, Clone, Copy, Default)]
pub struct RuleContext {
    pub now_unix_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleHit {
    pub rule_id: String,
    pub node_id: NodeId,
    pub score: u32,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleImportPreview {
    pub source_path: PathBuf,
    pub ruleset: RuleSet,
    pub incoming_rule_count: usize,
    pub incoming_enabled_count: usize,
    pub added_count: usize,
    pub removed_count: usize,
    pub changed_count: usize,
    pub unchanged_count: usize,
}

/// Returns `Some(())` if `node` matches the rule, with the reason
/// written to `reason_out` and a heuristic `score` (0-100) for
/// ranking. Read-only: never mutates anything.
pub fn matches(rule: &Rule, node: &Node, ctx: &RuleContext) -> Option<(u32, String)> {
    matches_with_path(rule, node, None, ctx)
}

fn matches_with_path(
    rule: &Rule,
    node: &Node,
    real_path: Option<&Path>,
    ctx: &RuleContext,
) -> Option<(u32, String)> {
    if !rule.enabled {
        return None;
    }
    match &rule.predicate {
        RulePredicate::LargeFile { min_size } => {
            if node.kind == NodeKind::File && node.size >= *min_size {
                Some((
                    score_for_size(node.size, *min_size),
                    format!("file size {} >= threshold {}", node.size, min_size),
                ))
            } else {
                None
            }
        }
        RulePredicate::OldFile {
            min_age_days,
            min_size,
        } => {
            if node.kind != NodeKind::File {
                return None;
            }
            if node.size < *min_size {
                return None;
            }
            let mtime = node.modified_secs?;
            if ctx.now_unix_secs < mtime {
                // Clock skew — mtime is in the future. Treat as
                // "very recent" so we never match a future-dated
                // file as old.
                return None;
            }
            let age_secs = ctx.now_unix_secs - mtime;
            const SECS_PER_DAY: u64 = 24 * 60 * 60;
            let age_days = age_secs / SECS_PER_DAY;
            if age_days >= *min_age_days {
                Some((
                    score_for_age(age_days, *min_age_days),
                    format!("modified {} days ago, size {}", age_days, node.size),
                ))
            } else {
                None
            }
        }
        RulePredicate::Hidden => {
            let hidden = node.name.starts_with('.');
            if hidden {
                Some((50, format!("hidden name: {}", node.name)))
            } else {
                None
            }
        }
        RulePredicate::Symlink => {
            if node.kind == NodeKind::Symlink {
                Some((40, format!("symlink: {}", node.name)))
            } else {
                None
            }
        }
        RulePredicate::AlwaysProtected { path_pattern } => {
            let path = real_path?;
            if platform::protected_path_reason(path).is_some()
                && path_matches_pattern(path, path_pattern)
            {
                Some((
                    100,
                    format!("protected path {} matches {path_pattern}", path.display()),
                ))
            } else {
                None
            }
        }
    }
}

/// Walk the tree from `root_id` and collect up to `limit` hits.
///
/// Read-only with one exception: calls `ensure_sorted_children` for
/// stable iteration order, matching the convention in `duplicates.rs`
/// and `insights.rs`.
pub fn evaluate_rules(
    rules: &RuleSet,
    tree: &mut TreeStore,
    root_id: NodeId,
    ctx: &RuleContext,
    limit: usize,
) -> Vec<RuleHit> {
    let mut hits = Vec::new();
    if root_id >= tree.len() {
        return hits;
    }
    walk(rules, tree, root_id, ctx, &mut hits, limit);
    // Stable order: by score desc, then by rule_id, then by node_id.
    hits.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.rule_id.cmp(&b.rule_id))
            .then_with(|| a.node_id.cmp(&b.node_id))
    });
    hits
}

fn walk(
    rules: &RuleSet,
    tree: &mut TreeStore,
    node_id: NodeId,
    ctx: &RuleContext,
    hits: &mut Vec<RuleHit>,
    limit: usize,
) {
    if hits.len() >= limit {
        return;
    }
    let real_path = tree.node_real_path(node_id);
    let node = tree.node(node_id).clone();
    for rule in &rules.rules {
        if hits.len() >= limit {
            return;
        }
        if let Some((score, reason)) = matches_with_path(rule, &node, real_path.as_deref(), ctx) {
            hits.push(RuleHit {
                rule_id: rule.id.clone(),
                node_id,
                score,
                reason,
            });
        }
    }
    tree.ensure_sorted_children(node_id);
    let children = tree.sorted_children(node_id).to_vec();
    for child in children {
        walk(rules, tree, child, ctx, hits, limit);
    }
}

/// A reasonable starting ruleset: a couple of cleanup candidates plus
/// the obvious system-location protected paths. The user can disable
/// any of them from the UI.
pub fn default_ruleset() -> RuleSet {
    let mut set = RuleSet::new();
    set.add(Rule {
        id: "large-file-1gb".into(),
        name: "Files ≥ 1 GB".into(),
        description: "Flag any file at least 1 GB. Likely targets for review or removal.".into(),
        category: RuleCategory::CleanupCandidate,
        predicate: RulePredicate::LargeFile {
            min_size: 1_073_741_824,
        },
        enabled: true,
    });
    set.add(Rule {
        id: "old-large-file".into(),
        name: "Old + large files".into(),
        description:
            "Files >100 MB that have not been modified in over a year. Common cleanup target."
                .into(),
        category: RuleCategory::CleanupCandidate,
        predicate: RulePredicate::OldFile {
            min_age_days: 365,
            min_size: 100 * 1_048_576,
        },
        enabled: true,
    });
    set.add(Rule {
        id: "hidden-files".into(),
        name: "Hidden entries".into(),
        description: "Files or directories whose name starts with a dot. Review before exposing."
            .into(),
        category: RuleCategory::AnomalyHint,
        predicate: RulePredicate::Hidden,
        enabled: true,
    });
    set.add(Rule {
        id: "symlinks".into(),
        name: "Symlinks".into(),
        description: "Symbolic links. May need special handling depending on scan options.".into(),
        category: RuleCategory::AnomalyHint,
        predicate: RulePredicate::Symlink,
        enabled: false,
    });
    for protected in platform::default_protected_path_rules() {
        set.add(Rule {
            id: protected.id.into(),
            name: protected.name.into(),
            description: protected.description.into(),
            category: RuleCategory::ProtectedPath,
            predicate: RulePredicate::AlwaysProtected {
                path_pattern: protected.path_pattern.into(),
            },
            enabled: true,
        });
    }
    set
}

/// Wire format version. Bump when the JSON shape changes.
pub const RULES_FORMAT_VERSION: u32 = 1;

/// Wrapper struct for JSON serialization. Adds a `version` field at
/// the top level so future readers can detect incompatible files.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExportedRuleSet {
    pub version: u32,
    pub rules: Vec<Rule>,
}

impl From<&RuleSet> for ExportedRuleSet {
    fn from(set: &RuleSet) -> Self {
        Self {
            version: RULES_FORMAT_VERSION,
            rules: set.rules.clone(),
        }
    }
}

impl From<ExportedRuleSet> for RuleSet {
    fn from(exp: ExportedRuleSet) -> Self {
        Self { rules: exp.rules }
    }
}

/// Serialize a `RuleSet` to pretty JSON. Always writes the current
/// `RULES_FORMAT_VERSION` so future readers can sanity-check.
pub fn export_ruleset_json(set: &RuleSet) -> String {
    let exp = ExportedRuleSet::from(set);
    serde_json::to_string_pretty(&exp)
        .unwrap_or_else(|e| format!("{{\"error\": \"serialize failed: {e}\"}}"))
}

/// Parse and validate JSON into a `RuleSet`. Returns a human-readable
/// error on any failure (invalid JSON, wrong version, missing fields,
/// empty id, etc.). Path-safety: doesn't actually load files; the
/// caller reads the file and passes the contents in.
pub fn import_ruleset_json(text: &str) -> Result<RuleSet, String> {
    let exp: ExportedRuleSet =
        serde_json::from_str(text).map_err(|e| format!("invalid JSON: {e}"))?;
    if exp.version != RULES_FORMAT_VERSION {
        return Err(format!(
            "unsupported version: expected {}, got {}",
            RULES_FORMAT_VERSION, exp.version
        ));
    }
    for (i, rule) in exp.rules.iter().enumerate() {
        if rule.id.trim().is_empty() {
            return Err(format!("rule at index {i} has empty id"));
        }
        if rule.name.trim().is_empty() {
            return Err(format!("rule '{}' has empty name", rule.id));
        }
        if exp.rules[..i].iter().any(|existing| existing.id == rule.id) {
            return Err(format!("duplicate rule id '{}'", rule.id));
        }
    }
    Ok(RuleSet { rules: exp.rules })
}

pub fn preview_ruleset_import(
    current: &RuleSet,
    incoming: RuleSet,
    source_path: PathBuf,
) -> RuleImportPreview {
    let current_by_id = current
        .rules
        .iter()
        .map(|rule| (rule.id.as_str(), rule))
        .collect::<BTreeMap<_, _>>();
    let incoming_by_id = incoming
        .rules
        .iter()
        .map(|rule| (rule.id.as_str(), rule))
        .collect::<BTreeMap<_, _>>();

    let mut added_count = 0;
    let mut changed_count = 0;
    let mut unchanged_count = 0;
    for (id, incoming_rule) in &incoming_by_id {
        match current_by_id.get(id) {
            Some(current_rule) if *current_rule == *incoming_rule => unchanged_count += 1,
            Some(_) => changed_count += 1,
            None => added_count += 1,
        }
    }

    let removed_count = current_by_id
        .keys()
        .filter(|id| !incoming_by_id.contains_key(**id))
        .count();

    RuleImportPreview {
        source_path,
        incoming_rule_count: incoming.rules.len(),
        incoming_enabled_count: incoming.enabled_count(),
        added_count,
        removed_count,
        changed_count,
        unchanged_count,
        ruleset: incoming,
    }
}

/// Write the ruleset to `<dest_dir>/disk-map-rules-<ts>.json` and
/// return the resulting path. Creates `dest_dir` if it doesn't exist.
pub fn export_ruleset_to_dir(set: &RuleSet, dest_dir: &Path) -> std::io::Result<PathBuf> {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};
    fs::create_dir_all(dest_dir)?;
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = dest_dir.join(format!("disk-map-rules-{ts}.json"));
    let json = export_ruleset_json(set);
    fs::write(&path, json)?;
    Ok(path)
}

/// Read JSON from `path` and parse it as a `RuleSet`. The path must
/// already be a file; we don't recurse, follow symlinks, or do any
/// other filesystem exploration. Returns a human-readable error on
/// any I/O or parse failure.
pub fn import_ruleset_from_path(path: &Path) -> Result<RuleSet, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    import_ruleset_json(&text)
}

fn score_for_size(actual: u64, threshold: u64) -> u32 {
    if threshold == 0 {
        return 50;
    }
    // log-scaled: 1x threshold = 30, 10x = 60, 100x = 90
    let ratio = (actual / threshold.max(1)) as f64;
    let score = 30.0_f64 + (ratio.log10().max(0.0) * 30.0);
    score.round().clamp(0.0, 100.0) as u32
}

fn score_for_age(actual_days: u64, threshold_days: u64) -> u32 {
    if threshold_days == 0 {
        return 50;
    }
    let ratio = (actual_days / threshold_days.max(1)) as f64;
    let score = 30.0_f64 + (ratio.log10().max(0.0) * 30.0);
    score.round().clamp(0.0, 100.0) as u32
}

fn path_matches_pattern(path: &Path, path_pattern: &str) -> bool {
    let pattern_parts = path_pattern
        .trim_matches('/')
        .split('/')
        .filter(|part| !part.is_empty());
    let mut components = path.components();
    if !matches!(components.next(), Some(Component::RootDir)) {
        return false;
    }

    let mut saw_pattern = false;
    for pattern_part in pattern_parts {
        saw_pattern = true;
        let Some(Component::Normal(component)) = components.next() else {
            return false;
        };
        if component.to_string_lossy() != pattern_part {
            return false;
        }
    }

    saw_pattern
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tree() -> (TreeStore, NodeId) {
        let mut tree = TreeStore::new();
        let root = tree.add_node(None, "root".into(), NodeKind::Dir, 0);
        tree.set_root_path("/".into());
        (tree, root)
    }

    fn add_file(
        tree: &mut TreeStore,
        parent: NodeId,
        name: &str,
        size: u64,
        mtime: Option<u64>,
    ) -> NodeId {
        tree.add_node_with_modified(Some(parent), name.into(), NodeKind::File, size, mtime)
    }

    #[test]
    fn empty_ruleset_produces_no_hits() {
        let (mut tree, root) = make_tree();
        let rules = RuleSet::new();
        let ctx = RuleContext::default();
        let hits = evaluate_rules(&rules, &mut tree, root, &ctx, 100);
        assert!(hits.is_empty());
    }

    #[test]
    fn disabled_rule_produces_no_hits() {
        let (mut tree, root) = make_tree();
        add_file(&mut tree, root, "big.bin", 5_000_000_000, None);
        let mut rules = RuleSet::new();
        rules.add(Rule {
            id: "r1".into(),
            name: "Big files".into(),
            description: "".into(),
            category: RuleCategory::CleanupCandidate,
            predicate: RulePredicate::LargeFile {
                min_size: 1_000_000_000,
            },
            enabled: false,
        });
        let ctx = RuleContext::default();
        let hits = evaluate_rules(&rules, &mut tree, root, &ctx, 100);
        assert!(hits.is_empty(), "disabled rule should not match");
    }

    #[test]
    fn large_file_rule_matches() {
        let (mut tree, root) = make_tree();
        add_file(&mut tree, root, "small.txt", 100, None);
        let big = add_file(&mut tree, root, "huge.bin", 2_000_000_000, None);
        let mut rules = RuleSet::new();
        rules.add(Rule {
            id: "r1".into(),
            name: "Big".into(),
            description: "".into(),
            category: RuleCategory::CleanupCandidate,
            predicate: RulePredicate::LargeFile {
                min_size: 1_000_000_000,
            },
            enabled: true,
        });
        let ctx = RuleContext::default();
        let hits = evaluate_rules(&rules, &mut tree, root, &ctx, 100);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].node_id, big);
    }

    #[test]
    fn old_file_rule_requires_both_size_and_age() {
        let (mut tree, root) = make_tree();
        let now: u64 = 1_700_000_000;
        const DAY: u64 = 24 * 60 * 60;
        // Old enough but too small
        add_file(&mut tree, root, "old-small.txt", 100, Some(now - 400 * DAY));
        // Big enough but too recent
        add_file(
            &mut tree,
            root,
            "fresh-big.bin",
            200 * 1_048_576,
            Some(now - 10 * DAY),
        );
        // Both
        let both = add_file(
            &mut tree,
            root,
            "old-big.bin",
            200 * 1_048_576,
            Some(now - 400 * DAY),
        );
        let mut rules = RuleSet::new();
        rules.add(Rule {
            id: "r1".into(),
            name: "Old+big".into(),
            description: "".into(),
            category: RuleCategory::CleanupCandidate,
            predicate: RulePredicate::OldFile {
                min_age_days: 365,
                min_size: 100 * 1_048_576,
            },
            enabled: true,
        });
        let ctx = RuleContext { now_unix_secs: now };
        let hits = evaluate_rules(&rules, &mut tree, root, &ctx, 100);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].node_id, both);
    }

    #[test]
    fn old_file_rule_rejects_future_dated_mtime() {
        let (mut tree, root) = make_tree();
        let now: u64 = 1_700_000_000;
        // mtime is in the future — treat as recent, never match
        add_file(&mut tree, root, "future.bin", 1_000_000, Some(now + 1000));
        let mut rules = RuleSet::new();
        rules.add(Rule {
            id: "r1".into(),
            name: "Old".into(),
            description: "".into(),
            category: RuleCategory::CleanupCandidate,
            predicate: RulePredicate::OldFile {
                min_age_days: 30,
                min_size: 1000,
            },
            enabled: true,
        });
        let ctx = RuleContext { now_unix_secs: now };
        let hits = evaluate_rules(&rules, &mut tree, root, &ctx, 100);
        assert!(hits.is_empty());
    }

    #[test]
    fn limit_caps_results() {
        let (mut tree, root) = make_tree();
        for i in 0..10 {
            add_file(
                &mut tree,
                root,
                &format!("file-{i}.bin"),
                (i + 1) * 1_000_000_000,
                None,
            );
        }
        let mut rules = RuleSet::new();
        rules.add(Rule {
            id: "r1".into(),
            name: "Big".into(),
            description: "".into(),
            category: RuleCategory::CleanupCandidate,
            predicate: RulePredicate::LargeFile {
                min_size: 500_000_000,
            },
            enabled: true,
        });
        let ctx = RuleContext::default();
        let hits = evaluate_rules(&rules, &mut tree, root, &ctx, 3);
        assert_eq!(hits.len(), 3, "limit should be respected");
    }

    #[test]
    fn hidden_and_symlink_predicates() {
        let (mut tree, root) = make_tree();
        let dot = add_file(&mut tree, root, ".hidden", 10, None);
        let link = tree.add_node(Some(root), "link".into(), NodeKind::Symlink, 0);
        let mut rules = RuleSet::new();
        rules.add(Rule {
            id: "h".into(),
            name: "h".into(),
            description: "".into(),
            category: RuleCategory::AnomalyHint,
            predicate: RulePredicate::Hidden,
            enabled: true,
        });
        rules.add(Rule {
            id: "s".into(),
            name: "s".into(),
            description: "".into(),
            category: RuleCategory::AnomalyHint,
            predicate: RulePredicate::Symlink,
            enabled: true,
        });
        let ctx = RuleContext::default();
        let hits = evaluate_rules(&rules, &mut tree, root, &ctx, 100);
        let ids: Vec<NodeId> = hits.iter().map(|h| h.node_id).collect();
        assert!(ids.contains(&dot));
        assert!(ids.contains(&link));
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn protected_rules_use_real_paths_instead_of_node_names() {
        let mut rules = RuleSet::new();
        rules.add(Rule {
            id: "protected-usr".into(),
            name: "Protected usr".into(),
            description: "".into(),
            category: RuleCategory::ProtectedPath,
            predicate: RulePredicate::AlwaysProtected {
                path_pattern: "usr".into(),
            },
            enabled: true,
        });
        let ctx = RuleContext::default();

        let mut project_tree = TreeStore::new();
        let project_root = project_tree.add_node(None, "project".into(), NodeKind::Dir, 0);
        project_tree.set_root_path("/home/user/project".into());
        project_tree.add_node(Some(project_root), "usr".into(), NodeKind::Dir, 0);
        assert!(
            evaluate_rules(&rules, &mut project_tree, project_root, &ctx, 100).is_empty(),
            "a project directory named usr should not be reported as a protected system path"
        );

        let mut system_tree = TreeStore::new();
        let system_root = system_tree.add_node(None, "usr".into(), NodeKind::Dir, 0);
        system_tree.set_root_path("/usr".into());
        let hits = evaluate_rules(&rules, &mut system_tree, system_root, &ctx, 100);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].node_id, system_root);
    }

    #[test]
    fn default_ruleset_has_seven_rules() {
        let rules = default_ruleset();
        assert_eq!(
            rules.rules.len(),
            4 + crate::platform::default_protected_path_rules().len()
        );
        assert!(
            rules.enabled_count() >= 5,
            "most defaults should be enabled"
        );
        assert!(rules.get("large-file-1gb").is_some());
        assert!(crate::platform::default_protected_path_rules()
            .iter()
            .all(|rule| rules.get(rule.id).is_some()));
    }

    #[test]
    fn rule_set_enable_disable_round_trip() {
        let mut rules = RuleSet::new();
        rules.add(Rule {
            id: "r1".into(),
            name: "r1".into(),
            description: "".into(),
            category: RuleCategory::AnomalyHint,
            predicate: RulePredicate::Hidden,
            enabled: true,
        });
        assert!(rules.disable("r1"));
        assert!(!rules.get("r1").unwrap().enabled);
        assert!(rules.enable("r1"));
        assert!(rules.get("r1").unwrap().enabled);
        assert!(!rules.disable("does-not-exist"));
    }

    #[test]
    fn duplicate_ids_are_deduplicated_on_add() {
        let mut rules = RuleSet::new();
        let mk = || Rule {
            id: "x".into(),
            name: "x".into(),
            description: "".into(),
            category: RuleCategory::AnomalyHint,
            predicate: RulePredicate::Hidden,
            enabled: true,
        };
        rules.add(mk());
        rules.add(mk());
        assert_eq!(rules.rules.len(), 1);
    }

    #[test]
    fn export_then_import_round_trips() {
        let set = default_ruleset();
        let json = export_ruleset_json(&set);
        let restored = import_ruleset_json(&json).expect("import should succeed");
        assert_eq!(restored, set);
    }

    #[test]
    fn empty_ruleset_round_trips() {
        let set = RuleSet::new();
        let json = export_ruleset_json(&set);
        let restored = import_ruleset_json(&json).unwrap();
        assert_eq!(restored.rules.len(), 0);
    }

    #[test]
    fn import_rejects_invalid_json() {
        let err = import_ruleset_json("not json at all").unwrap_err();
        assert!(err.contains("invalid JSON"), "got: {err}");
    }

    #[test]
    fn import_rejects_wrong_version() {
        let bad = r#"{"version": 999, "rules": []}"#;
        let err = import_ruleset_json(bad).unwrap_err();
        assert!(err.contains("version"), "got: {err}");
    }

    #[test]
    fn import_rejects_empty_id() {
        let bad = r#"{"version": 1, "rules": [{"id": "", "name": "x", "description": "", "category": "AnomalyHint", "predicate": "Hidden", "enabled": true}]}"#;
        let err = import_ruleset_json(bad).unwrap_err();
        assert!(err.contains("empty id"), "got: {err}");
    }

    #[test]
    fn import_rejects_unknown_category_variant() {
        let bad = r#"{"version": 1, "rules": [{"id": "x", "name": "x", "description": "", "category": "NoSuchCategory", "predicate": "Hidden", "enabled": true}]}"#;
        let err = import_ruleset_json(bad).unwrap_err();
        assert!(
            err.contains("invalid JSON") || err.contains("category"),
            "got: {err}"
        );
    }

    #[test]
    fn import_rejects_duplicate_ids() {
        let bad = r#"{"version": 1, "rules": [
            {"id": "x", "name": "x", "description": "", "category": "AnomalyHint", "predicate": "Hidden", "enabled": true},
            {"id": "x", "name": "x2", "description": "", "category": "AnomalyHint", "predicate": "Symlink", "enabled": true}
        ]}"#;
        let err = import_ruleset_json(bad).unwrap_err();
        assert!(err.contains("duplicate rule id"), "got: {err}");
    }

    #[test]
    fn preview_ruleset_import_summarizes_delta_without_applying() {
        let mut current = RuleSet::new();
        current.add(Rule {
            id: "same".into(),
            name: "Same".into(),
            description: "".into(),
            category: RuleCategory::AnomalyHint,
            predicate: RulePredicate::Hidden,
            enabled: true,
        });
        current.add(Rule {
            id: "changed".into(),
            name: "Changed".into(),
            description: "before".into(),
            category: RuleCategory::AnomalyHint,
            predicate: RulePredicate::Hidden,
            enabled: true,
        });
        current.add(Rule {
            id: "removed".into(),
            name: "Removed".into(),
            description: "".into(),
            category: RuleCategory::AnomalyHint,
            predicate: RulePredicate::Hidden,
            enabled: true,
        });

        let mut incoming = RuleSet::new();
        incoming.add(current.get("same").unwrap().clone());
        incoming.add(Rule {
            id: "changed".into(),
            name: "Changed".into(),
            description: "after".into(),
            category: RuleCategory::AnomalyHint,
            predicate: RulePredicate::Hidden,
            enabled: false,
        });
        incoming.add(Rule {
            id: "added".into(),
            name: "Added".into(),
            description: "".into(),
            category: RuleCategory::AnomalyHint,
            predicate: RulePredicate::Symlink,
            enabled: true,
        });

        let preview = preview_ruleset_import(&current, incoming, PathBuf::from("/rules.json"));

        assert_eq!(preview.incoming_rule_count, 3);
        assert_eq!(preview.incoming_enabled_count, 2);
        assert_eq!(preview.added_count, 1);
        assert_eq!(preview.removed_count, 1);
        assert_eq!(preview.changed_count, 1);
        assert_eq!(preview.unchanged_count, 1);
        assert_eq!(current.rules.len(), 3);
    }
}
