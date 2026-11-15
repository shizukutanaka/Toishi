//! cluster stage (spec §2).
//!
//! Commits are grouped by the Jaccard similarity of their normalized
//! fingerprint sets. Clustering is greedy and deterministic: commits are
//! processed in ascending hash order, each commit joins the first cluster
//! whose representative set (the fingerprint set of the commit that
//! created the cluster) has Jaccard similarity >= [`JACCARD_THRESHOLD`],
//! and otherwise starts a new cluster.
//!
//! A cluster whose member count (its *support*) reaches the configured
//! threshold becomes a skill candidate. The default support is 3; on the
//! small, single-author repositories Toishi primarily targets, sparse
//! recurrence is the norm (within-project fix patterns recur at only
//! 8–21% in the MSR literature), so the threshold is configurable and a
//! zero-skill result is a first-class outcome, not an error.

use crate::CommitPattern;
use std::collections::BTreeSet;

/// Jaccard similarity at or above which two change patterns are
/// considered the same pattern.
pub const JACCARD_THRESHOLD: f64 = 0.5;

/// Default minimum support (number of commits) for a skill candidate.
pub const DEFAULT_SUPPORT: usize = 3;

/// A group of commits sharing one change pattern.
#[derive(Debug, Clone)]
pub struct Cluster {
    /// Indices into the commits array, in processing order.
    pub members: Vec<usize>,
    /// Fingerprint set of the first member; the cluster's identity.
    pub representative: BTreeSet<u64>,
    /// Union of all members' fingerprint sets.
    pub union: BTreeSet<u64>,
}

impl Cluster {
    /// Support: how many commits exhibit this pattern.
    pub fn support(&self) -> usize {
        self.members.len()
    }
}

/// Jaccard similarity |A ∩ B| / |A ∪ B|. Two empty sets are defined as
/// dissimilar (0.0): an empty pattern is not a pattern.
pub fn jaccard(a: &BTreeSet<u64>, b: &BTreeSet<u64>) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let intersection = a.intersection(b).count() as f64;
    let union = a.len() + b.len() - intersection as usize;
    intersection / union as f64
}

/// Cluster commit patterns. `patterns` must already be in deterministic
/// (ascending-hash) order; the result preserves that determinism.
pub fn cluster(patterns: &[CommitPattern]) -> Vec<Cluster> {
    let mut clusters: Vec<Cluster> = Vec::new();
    for pattern in patterns {
        if pattern.fingerprints.is_empty() {
            continue;
        }
        let mut joined = false;
        for cl in clusters.iter_mut() {
            if jaccard(&pattern.fingerprints, &cl.representative) >= JACCARD_THRESHOLD {
                cl.members.push(pattern.commit_index);
                cl.union.extend(pattern.fingerprints.iter().copied());
                joined = true;
                break;
            }
        }
        if !joined {
            clusters.push(Cluster {
                members: vec![pattern.commit_index],
                representative: pattern.fingerprints.clone(),
                union: pattern.fingerprints.clone(),
            });
        }
    }
    clusters
}

/// Select candidate clusters: support at or above `min_support`, in
/// deterministic order (support descending, then smallest member index).
pub fn candidates(clusters: &[Cluster], min_support: usize) -> Vec<Cluster> {
    let mut selected: Vec<Cluster> = clusters
        .iter()
        .filter(|c| c.support() >= min_support)
        .cloned()
        .collect();
    selected.sort_by(|a, b| {
        b.support()
            .cmp(&a.support())
            .then_with(|| a.members[0].cmp(&b.members[0]))
    });
    selected
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CommitPattern;
    use std::collections::BTreeSet;

    fn pattern(idx: usize, fps: &[u64]) -> CommitPattern {
        CommitPattern {
            commit_index: idx,
            fingerprints: fps.iter().copied().collect::<BTreeSet<_>>(),
            lines: Vec::new(),
        }
    }

    #[test]
    fn jaccard_values() {
        let a: BTreeSet<u64> = [1, 2, 3].into_iter().collect();
        let b: BTreeSet<u64> = [2, 3, 4].into_iter().collect();
        assert!((jaccard(&a, &b) - 0.5).abs() < 1e-9);
        assert_eq!(jaccard(&a, &a), 1.0);
        let empty = BTreeSet::new();
        assert_eq!(jaccard(&empty, &a), 0.0);
    }

    #[test]
    fn similar_patterns_cluster_together() {
        // 4 shared keys, 2 unique each: Jaccard = 4/8 = 0.5 -> same cluster.
        let patterns = vec![
            pattern(0, &[1, 2, 3, 4, 10, 11]),
            pattern(1, &[1, 2, 3, 4, 20, 21]),
            pattern(2, &[1, 2, 3, 4, 30, 31]),
        ];
        let clusters = cluster(&patterns);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].support(), 3);
    }

    #[test]
    fn dissimilar_patterns_stay_separate() {
        let patterns = vec![pattern(0, &[1, 2, 3]), pattern(1, &[7, 8, 9])];
        let clusters = cluster(&patterns);
        assert_eq!(clusters.len(), 2);
    }

    #[test]
    fn support_threshold_filters() {
        let patterns = vec![
            pattern(0, &[1, 2, 3, 4, 10, 11]),
            pattern(1, &[1, 2, 3, 4, 20, 21]),
            pattern(2, &[1, 2, 3, 4, 30, 31]),
            pattern(3, &[7, 8, 9]),
        ];
        let clusters = cluster(&patterns);
        assert_eq!(candidates(&clusters, 3).len(), 1);
        assert_eq!(candidates(&clusters, 2).len(), 1);
        assert_eq!(candidates(&clusters, 4).len(), 0);
    }

    #[test]
    fn empty_patterns_are_skipped() {
        let patterns = vec![pattern(0, &[]), pattern(1, &[1])];
        let clusters = cluster(&patterns);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].support(), 1);
    }
}
