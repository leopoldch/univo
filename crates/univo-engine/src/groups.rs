//! Optional groups over the records of an index, built from pairs and never
//! presented as entities.
//!
//! Similarity is not transitive: A close to B and B close to C does not make
//! A close to C. So two kinds of groups are kept apart:
//!
//! - exact groups: records equal on every selected field after
//!   normalization. Equality is transitive, so these are sound.
//! - conservative groups: a record or group joins another only when every
//!   pair between them reaches the minimum score without a veto, up to a
//!   size limit. Pairs are taken by descending score, then slot, so the
//!   result is deterministic; another order could group differently, and
//!   adding a member checks it against every member, `O(g²)` per group.
//!
//! Members of one exact group have the same normalized fields and
//! embeddings, so they score alike against any other record: each exact
//! group is one node of the pair graph, and a group of a thousand identical
//! records costs one search, not a million pairs.

use std::collections::HashMap;
use std::sync::Mutex;

use univo_core::{Error, Result, exact_fingerprint, has_exact_evidence, is_exact_match};

use crate::{SearchIndex, SearchOptions};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GroupOptions {
    /// Pairs scoring at least this are linked; `0 < min_score ≤ 1`.
    pub min_score: f32,
    /// Work limit of each candidate lane, as in search.
    pub max_entries: usize,
    /// Score every record of each partition instead of candidate lanes.
    pub exhaustive: bool,
    /// Largest conservative group, in records.
    pub max_group_size: usize,
    /// Most qualifying pairs kept in memory while grouping.
    pub max_pairs: usize,
    /// Scan budgets of the underlying all-pairs search, as in `find`: the
    /// admissible pairs when exhaustive, the semantic comparisons otherwise.
    pub max_exhaustive_pairs: usize,
    pub max_semantic_comparisons: usize,
}

/// A conservative group: every pair of its members reaches the minimum.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    /// Slots, in increasing order.
    pub members: Vec<usize>,
    /// The lowest score among its pairs: 1 within exact groups.
    pub min_score: f32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct GroupReport {
    /// Records equal on every selected field, two or more per group, each
    /// in increasing slot order, ordered by first slot.
    pub exact: Vec<Vec<usize>>,
    /// Conservative groups of two or more records, ordered by first slot.
    pub groups: Vec<Group>,
    /// Distinct pairs between exact groups or single records reaching the
    /// minimum score without a veto.
    pub qualifying_pairs: usize,
    /// Qualifying pairs whose records ended in different groups or in none:
    /// evidence that the conservative policy or the size limit split them.
    pub split_pairs: usize,
    /// Records scored by the searches.
    pub candidates: usize,
    /// A candidate lane reached its work limit: pairs may be missing.
    pub truncated: bool,
    /// Approximate retrieval was used: pairs may be missing.
    pub approximate: bool,
}

/// A conservative group under construction.
struct Building {
    /// Representatives of the exact groups it holds.
    nodes: Vec<usize>,
    /// Records it holds.
    size: usize,
    min_score: f32,
}

/// Records grouped by verified equality: each active slot's representative
/// (the first slot of its exact group, `usize::MAX` when inactive), each
/// representative's group size, and, for exact groups whose members score
/// at least the minimum together, that score.
struct Exact {
    groups: Vec<Vec<usize>>,
    node: Vec<usize>,
    sizes: HashMap<usize, usize>,
    inner: HashMap<usize, f32>,
}

/// Conservative groups as node lists, and the group of each grouped node.
struct Conservative {
    groups: Vec<Building>,
    group_of: HashMap<usize, usize>,
}

impl SearchIndex {
    /// Exact and conservative groups of the active records.
    ///
    /// # Errors
    ///
    /// Returns `parameter.threshold` for a minimum score outside `(0, 1]`,
    /// `limit.pair_count` or `limit.semantic_comparisons` above the scan
    /// budgets, before any scoring, `limit.group_pairs` when more than
    /// `max_pairs` qualifying pairs are found, and the search's errors.
    pub fn groups(&self, options: GroupOptions) -> Result<GroupReport> {
        let min_score = options.min_score;
        if !(min_score.is_finite() && min_score > 0.0 && min_score <= 1.0) {
            return Err(Error::invalid_threshold(min_score));
        }
        self.check_scan(
            options.exhaustive,
            options.max_exhaustive_pairs,
            options.max_semantic_comparisons,
        )?;
        let mut report = GroupReport::default();
        let exact = self.exact_groups(min_score)?;
        let edges = self.qualifying_pairs(&exact, options, &mut report)?;
        let conservative = conservative(&edges, &exact, options.max_group_size);

        let mut members: Vec<Vec<usize>> = vec![Vec::new(); conservative.groups.len()];
        for slot in self.store().active_slots() {
            if let Some(&group) = conservative.group_of.get(&exact.node[slot]) {
                members[group].push(slot);
            }
        }
        report.groups = conservative
            .groups
            .iter()
            .zip(members)
            .filter(|(group, _)| group.size > 1)
            .map(|(group, members)| Group {
                members,
                min_score: group.min_score,
            })
            .collect();
        report.groups.sort_unstable_by_key(|group| group.members[0]);
        report.qualifying_pairs = edges.len();
        report.split_pairs = edges
            .keys()
            .filter(|(a, b)| {
                let left = conservative.group_of.get(a);
                left.is_none() || left != conservative.group_of.get(b)
            })
            .count();
        report.exact = exact.groups;
        Ok(report)
    }

    /// Fingerprint buckets, split by verified equality with each group's
    /// first record, with the score of their members together.
    fn exact_groups(&self, min_score: f32) -> Result<Exact> {
        let schema = self.schema();
        let records = self.store().records();
        let mut buckets: HashMap<u64, Vec<Vec<usize>>> = HashMap::new();
        let mut singletons = Vec::new();
        for slot in self.store().active_slots() {
            // Never an exact match, whatever its bucket holds: scanning the
            // bucket for each blank record would be quadratic.
            if !has_exact_evidence(schema, &records[slot])? {
                singletons.push(vec![slot]);
                continue;
            }
            let groups = buckets
                .entry(exact_fingerprint(schema, &records[slot])?)
                .or_default();
            let mut joined = None;
            for (index, group) in groups.iter().enumerate() {
                if is_exact_match(schema, &records[group[0]], &records[slot])? {
                    joined = Some(index);
                    break;
                }
            }
            match joined {
                Some(index) => groups[index].push(slot),
                None => groups.push(vec![slot]),
            }
        }
        let mut exact = Exact {
            groups: Vec::new(),
            node: vec![usize::MAX; records.len()],
            sizes: HashMap::new(),
            inner: HashMap::new(),
        };
        for group in buckets.into_values().flatten().chain(singletons) {
            for &slot in &group {
                exact.node[slot] = group[0];
            }
            exact.sizes.insert(group[0], group.len());
            if group.len() > 1 {
                // Equal records all score alike, but not necessarily 1: two
                // identical unit vectors can have a cosine just below 1.
                if let Some(score) = self
                    .pair_score(group[0], group[1])?
                    .filter(|score| *score >= min_score)
                {
                    exact.inner.insert(group[0], score);
                }
                exact.groups.push(group);
            }
        }
        exact.groups.sort_unstable_by_key(|group| group[0]);
        Ok(exact)
    }

    /// The best score of each pair of nodes reaching the minimum, each
    /// found from its lower node.
    ///
    /// Nodes are searched in parallel inside a Rayon pool. Workers
    /// buffer at most [`EDGE_BUFFER`] pairs before folding them into the
    /// shared map, which only grows and stops at `max_pairs + 1` pairs: the
    /// budget fails exactly when more than `max_pairs` distinct pairs
    /// qualify, with the same count, whatever the threads.
    fn qualifying_pairs(
        &self,
        exact: &Exact,
        options: GroupOptions,
        report: &mut GroupReport,
    ) -> Result<HashMap<(usize, usize), f32>> {
        let search = SearchOptions {
            min_score: options.min_score,
            limit: 0,
            max_entries: options.max_entries,
        };
        let edges: Mutex<HashMap<(usize, usize), f32>> = Mutex::new(HashMap::new());
        let fold = |buffer: &mut Vec<((usize, usize), f32)>| -> Result<()> {
            let mut edges = edges.lock().map_err(|_| Error::index_poisoned())?;
            // Another worker may have exceeded the budget: insert nothing
            // more, so the reported count does not depend on the schedule.
            if edges.len() > options.max_pairs {
                return Err(Error::group_pair_limit(edges.len(), options.max_pairs));
            }
            for (pair, score) in buffer.drain(..) {
                let kept = edges.entry(pair).or_insert(score);
                *kept = kept.max(score);
                if edges.len() > options.max_pairs {
                    return Err(Error::group_pair_limit(edges.len(), options.max_pairs));
                }
            }
            Ok(())
        };
        let mut nodes: Vec<usize> = exact.sizes.keys().copied().collect();
        nodes.sort_unstable();
        let counts = crate::fold_queries(
            nodes,
            || (0usize, false, false),
            |(candidates, truncated, approximate), from| {
                // Pairs are folded into edges as they are found, so the
                // budget bounds memory: no search result keeps its
                // explanations.
                let mut buffer = Vec::new();
                let result =
                    self.visit_pairs(from, search, options.exhaustive, &mut |slot, score| {
                        let to = exact.node[slot];
                        if to != from && to != usize::MAX {
                            buffer.push(((from.min(to), from.max(to)), score));
                            if buffer.len() == EDGE_BUFFER {
                                fold(&mut buffer)?;
                            }
                        }
                        Ok(())
                    })?;
                fold(&mut buffer)?;
                Ok::<_, Error>((
                    candidates + result.candidates,
                    truncated | result.truncated,
                    approximate | result.approximate,
                ))
            },
            |a, b| (a.0 + b.0, a.1 | b.1, a.2 | b.2),
        )?;
        report.candidates += counts.0;
        report.truncated |= counts.1;
        report.approximate |= counts.2;
        edges.into_inner().map_err(|_| Error::index_poisoned())
    }
}

/// Qualifying pairs a worker holds before taking the shared map's lock.
const EDGE_BUFFER: usize = 1024;

/// Starts from the exact groups whose members score at least the minimum
/// together, then joins nodes and groups along the strongest pairs first,
/// only when every pair across the two sides qualifies and the size limit
/// holds. An exact group whose members do not qualify together joins
/// nothing.
fn conservative(
    edges: &HashMap<(usize, usize), f32>,
    exact: &Exact,
    max_group_size: usize,
) -> Conservative {
    let sizes = &exact.sizes;
    let usable = |node: usize| sizes[&node] == 1 || exact.inner.contains_key(&node);
    let mut order: Vec<((usize, usize), f32)> = edges
        .iter()
        .filter(|((a, b), _)| usable(*a) && usable(*b))
        .map(|(&pair, &score)| (pair, score))
        .collect();
    order.sort_unstable_by(|(a, a_score), (b, b_score)| {
        b_score.total_cmp(a_score).then_with(|| a.cmp(b))
    });
    let mut built = Conservative {
        groups: Vec::new(),
        group_of: HashMap::new(),
    };
    let mut seeds: Vec<(usize, f32)> = exact
        .inner
        .iter()
        .filter(|&(node, _)| sizes[node] <= max_group_size)
        .map(|(&node, &score)| (node, score))
        .collect();
    seeds.sort_unstable_by_key(|&(node, _)| node);
    for (node, score) in seeds {
        built.group_of.insert(node, built.groups.len());
        built.groups.push(Building {
            nodes: vec![node],
            size: sizes[&node],
            min_score: score,
        });
    }
    for ((a, b), _) in order {
        let (left, right) = (
            built.group_of.get(&a).copied(),
            built.group_of.get(&b).copied(),
        );
        if left.is_some() && left == right {
            continue;
        }
        let side = |group: Option<usize>, single: usize| match group {
            Some(group) => {
                let group = &built.groups[group];
                (group.nodes.clone(), group.size, group.min_score)
            }
            None => (vec![single], sizes[&single], 1.0),
        };
        let (left_nodes, left_size, left_min) = side(left, a);
        let (right_nodes, right_size, right_min) = side(right, b);
        if left_size + right_size > max_group_size {
            continue;
        }
        let mut joined_min = left_min.min(right_min);
        let complete = left_nodes.iter().all(|&x| {
            right_nodes.iter().all(|&y| {
                edges
                    .get(&(x.min(y), x.max(y)))
                    .map(|&score| joined_min = joined_min.min(score))
                    .is_some()
            })
        });
        if !complete {
            continue;
        }
        let target = match (left, right) {
            (None, None) => {
                built.groups.push(Building {
                    nodes: Vec::new(),
                    size: 0,
                    min_score: 1.0,
                });
                built.groups.len() - 1
            }
            (Some(group), None) | (None, Some(group)) => group,
            (Some(first), Some(second)) => {
                let (kept, emptied) = (first.min(second), first.max(second));
                built.groups[emptied].nodes.clear();
                built.groups[emptied].size = 0;
                kept
            }
        };
        for node in left_nodes.into_iter().chain(right_nodes) {
            if built.group_of.insert(node, target) != Some(target) {
                built.groups[target].nodes.push(node);
            }
        }
        built.groups[target].size = left_size + right_size;
        built.groups[target].min_score = joined_min;
    }
    built
}
