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

use crate::tree::{Node, NodeId, NodeKind, TreeStore};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RulePredicate {
    /// Match a file whose size is at least `min_size` bytes.
    LargeFile { min_size: u64 },
    /// Match a file whose modified-time is at least `min_age_days`
    /// old AND size is at least `min_size` bytes. Unknown mtime never
    /// matches.
    OldFile {
        min_age_days: u64,
        min_size: u64,
    },
    /// Match hidden files or directories (name starts with `.`).
    Hidden,
    /// Match symlink nodes.
    Symlink,
    /// Match any node whose real path starts with the given pattern.
    AlwaysProtected { path_pattern: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: RuleCategory,
    pub predicate: RulePredicate,
    pub enabled: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
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

/// Returns `Some(())` if `node` matches the rule, with the reason
/// written to `reason_out` and a heuristic `score` (0-100) for
/// ranking. Read-only: never mutates anything.
pub fn matches(rule: &Rule, node: &Node, ctx: &RuleContext) -> Option<(u32, String)> {
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
                    format!(
                        "modified {} days ago, size {}",
                        age_days, node.size
                    ),
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
            // We don't have the real path on the Node itself; we
            // accept a "best-effort" match using node.name for
            // top-level entries and rely on the caller (rule editor
            // UI or future scan-aware version) to refine.
            let name_match = node.name == path_pattern.trim_start_matches('/');
            if name_match {
                Some((100, format!("name matches protected pattern {path_pattern}")))
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
    let node = tree.node(node_id).clone();
    for rule in &rules.rules {
        if hits.len() >= limit {
            return;
        }
        if let Some((score, reason)) = matches(rule, &node, ctx) {
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
        description: "Files >100 MB that have not been modified in over a year. Common cleanup target.".into(),
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
        description: "Files or directories whose name starts with a dot. Review before exposing.".into(),
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
    set.add(Rule {
        id: "protected-system".into(),
        name: "System location".into(),
        description: "Top-level /System entry. Destructive actions against this are blocked.".into(),
        category: RuleCategory::ProtectedPath,
        predicate: RulePredicate::AlwaysProtected {
            path_pattern: "System".into(),
        },
        enabled: true,
    });
    set.add(Rule {
        id: "protected-library".into(),
        name: "Library location".into(),
        description: "Top-level /Library entry. Destructive actions against this are blocked.".into(),
        category: RuleCategory::ProtectedPath,
        predicate: RulePredicate::AlwaysProtected {
            path_pattern: "Library".into(),
        },
        enabled: true,
    });
    set.add(Rule {
        id: "protected-applications".into(),
        name: "Applications location".into(),
        description: "Top-level /Applications entry. Destructive actions against this are blocked.".into(),
        category: RuleCategory::ProtectedPath,
        predicate: RulePredicate::AlwaysProtected {
            path_pattern: "Applications".into(),
        },
        enabled: true,
    });
    set
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
        tree.add_node_with_modified(
            Some(parent),
            name.into(),
            NodeKind::File,
            size,
            mtime,
        )
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
        add_file(&mut tree, root, "fresh-big.bin", 200 * 1_048_576, Some(now - 10 * DAY));
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
    fn default_ruleset_has_seven_rules() {
        let rules = default_ruleset();
        assert_eq!(rules.rules.len(), 7);
        assert!(rules.enabled_count() >= 5, "most defaults should be enabled");
        assert!(rules.get("large-file-1gb").is_some());
        assert!(rules.get("protected-system").is_some());
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
}
