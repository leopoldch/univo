# 0017. Exact and conservative groups

- Status: Accepted
- Date: 2026-10-12
- Roadmap: U-018

## Context

[specs/03-matching.md §17](../../specs/03-matching.md) keeps the pair as the basic result and allows optional groups, once pairs are stable:
- exact equalities, grouped by canonical key;
- candidate groups as an explicit graph, without "single entity" status;
- conservative grouping, where a record is added only if every pair it needs satisfies the policy, with a stable order, a size limit, and documented order dependence and `O(g²)` cost.

It forbids union-find over every similar pair as proof of identity. The acceptance criterion of U-018 is that no transitive closure is presented as identity.

## Decision

- **API.** `SearchIndex::groups` in the engine, and `Finder.group(records, ...)` in Python, group a temporary dataset like `find`. They return exact groups, conservative groups, and counters. The explicit candidate graph is what `find` already reports.
- **Exact groups.** Active records are bucketed by exact fingerprint, then split by verified equality (`is_exact_match`) against each group's first record. A fingerprint collision can never merge unequal records. Equality after normalization is transitive, so these groups are sound. Records without exact evidence (`has_exact_evidence`), such as blank strings, are singletons at once: they share one fingerprint but never match, and scanning their bucket would be quadratic.
- **Each exact group is one node.** Its members have identical normalized fields and embeddings, so they score alike against any record. Pairs are searched only from one representative per exact group. A group of a thousand identical records costs one search instead of about 500,000 pairs.
- **Conservative groups.** They start from the exact groups whose members reach the minimum together. That score is computed with the search's scoring, not assumed to be 1: identical unit vectors can have a cosine of 0.99999994 in `f32`. An exact group below the minimum joins nothing. Qualifying pairs between nodes (score at least `min_score`, no veto, from the same search as `find`) are taken by descending score, then slot. Two nodes or groups are joined only if every pair across them qualifies and the size stays within `max_group_size`.
- **Visibility.** Qualifying pairs whose records end in different groups are counted (`split_pairs`) and reported as a warning: they are the non-transitivity the policy refused to close.
- **Limits.** `max_pairs` bounds the qualifying pairs held in memory (`limit.group_pairs`, a new code). Pairs are folded into the graph as each search scores them, without keeping their explanations, so the budget bounds memory throughout. The scan budgets of `find` apply before any scoring.
- **No merge.** Groups never change records; `plan_merge` stays after 1.0.

## Alternatives

- **Connected components (union-find) of qualifying pairs:** linear and order-independent, but it closes chains: A–B and B–C put A and C together whatever A–C scores. This is exactly what the specification forbids. Rejected, and a mutation test shows the tests catch it.
- **Maximal cliques:** order-independent, but exponential in the worst case, and a record can belong to several cliques, so groups overlap. Rejected for a default; the pairs from `find` remain available for such analyses.
- **Average or centroid linkage:** joins a record whose average similarity to a group is high, even if one pair is far below the threshold. That breaks the guarantee that every pair inside a group was scored and qualifies. Rejected.
- **Grouping the stored index instead of a temporary dataset:** useful later, but it would need the generation and the lock of U-017 for a long read. The temporary form matches `find` and keeps the first version simple.

## Evidence

- Random datasets (300 records, three seeds) at thresholds 0.6, 0.8 and 0.95, checked with `compare` itself:
  - every pair inside every conservative group qualifies and is not vetoed;
  - groups are disjoint and within the size limit;
  - exact groups are verified and maximal;
  - results are deterministic, and the same in candidate and exhaustive search.
- Mutation checks: single-link chaining and an ignored size limit each make the tests fail.

## Consequences

- Groups depend on the order of pairs. The order is deterministic, documented and based on the strongest evidence first, but it is a policy choice: another order could group differently.
- A bounded or approximate search can only miss pairs. Groups are then smaller, never wrongly joined, and the report says so.
- Memory is the qualifying pairs between nodes, bounded by `max_pairs`. The time is the searches of `find`, minus one per duplicate record, plus `O(g²)` checks per group.

## Revisit when

- An annotated validation set (U-012 follow-up) shows conservative groups too fragmented to be useful: consider a calibrated group policy, still checking every pair.
- Users need groups of a stored index: run the same algorithm under the shared lock of U-017, with the generation in the report.
