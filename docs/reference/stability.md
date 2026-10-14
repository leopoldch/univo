# Stability

From 1.0, Univo follows [Semantic Versioning](https://semver.org/). This page says what each 1.x release keeps compatible, and how to upgrade when a release changes scores or formats. The scope of 1.0 itself is set by [ADR 0024](../adr/0024-version-1-scope.md).

## Kept compatible across 1.x

- **Python API.**
  - **Names.** Every name in `univo.__all__` and in `univo.models`.
  - **Parameters.** Their parameters and defaults.
  - **Results.** The attributes of `Report`, `Match`, `Groups`, `Group` and `Decision`.
  - **Errors.** The exception classes and their hierarchy.
  - **Additions.** A minor release may add keyword parameters with defaults, attributes and exception subclasses.
  - **Removals and renamings.** These wait for 2.0, after a deprecation warning in a 1.x release.
- **Error codes.** The `code` of each error, such as `limit.pair_count`, and its kind. Messages and hints may be reworded.
- **Explanations.** `Match.reasons` holds `"<path>: <signal>"` with a signal of `missing`, `edit_distance`, `trigrams`, `exact`, `keys`, `semantic` or `acronym`, and `"<path>: normalized (<steps>)"` for exact matches. `Match.conflicts` lists the paths of contradicting fields. `search_profile` is `exhaustive`, `bounded` or `approximate`.
- **Completeness.** Without a reported limit (`candidate_truncated`, `output_truncated`) and outside the explicit approximate profile, search returns exactly what the exhaustive oracle returns.
- **Hard rules.** Partitions, vetoes, typed IDs and required fields.

## May change in a minor release

Each change is announced in the [changelog](../../CHANGELOG.md), with its evidence in an ADR.

- **Scores.** Raising `SCORING_VERSION` changes the scores of some pairs, as version 4 did for names with distinct words ([ADR 0023](../adr/0023-distinct-words.md)). Rankings stay explained; thresholds calibrated on an earlier version should be checked again.
- **The default model.** It is pinned by revision and checksums. A new default model is a minor release; snapshots record the model they were built with.
- **Messages, hints and warning texts.**

## Not covered

- **Internals.** The Rust crates, which are not published, and private modules such as `univo._univo` and `univo._api`.
- **Exact numbers.** Performance figures, which budgets test on a reference machine. `stats()` byte estimates. `config_fingerprint` values.

## Upgrading

1. **Read the changelog** for the versions in between.
2. **Rebuild snapshots when a version changed.** A snapshot records the scoring, search and normalizer versions, and the model, it was built with. A release that changes any of them refuses older snapshots with `IndexCompatibilityError`: there is no silent migration. Snapshots keep only normalized selected fields, so rebuild from the original records: create the `Finder` with the same schema, `add` the records, then `save`.
3. **Check calibrated thresholds** when `SCORING_VERSION` changed, on the same labelled data as before.

Snapshots written by a newer release are refused by older ones.
