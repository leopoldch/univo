# 0004. Error model shared by every binding

- Status: Accepted
- Date: 2026-10-07
- Roadmap: U-006

## Context

Errors are what users see when something goes wrong; they must say what happened, where, and how to fix it. Python is the first binding and others, starting with Java, are planned on the same Rust core ([10-roadmap.md §28](../../specs/10-roadmap.md)). If each binding composed its own messages, wording, coverage and documentation would drift between languages.

## Decision

- An error is a boxed value holding a code, a complete message, an optional field path, ordered structured details and, for batches, the position of the record. `Result<T, Error>` therefore costs one pointer on the success path.
- Each `ErrorCode` has a stable string (`record.wrong_type`), a kind, a title and a hint, declared once in a table in `crates/univo-core/src/error.rs`. Kinds map to one exception type per language (`InvalidRecord` to `univo.InvalidRecordError` in Python and to an `InvalidRecordException` in Java), under a common base.
- Messages, hints and details are built only in Rust, by one constructor per code. Bindings copy them into exception attributes and never compose text; bindings that convert foreign values use the public constructors (`unsupported_type`, `non_finite_number`, `integer_out_of_range`, `duplicate_key`).
- Messages and details name paths, types, sizes and limits, never field values. Record identifiers are kept as an attribute and are not printed.
- `docs/reference/errors.md` is generated from real calls by `crates/univo-core/tests/error_catalog.rs`, which fails when a code has no example, when a message leaks a value, or when the document is stale.

## Alternatives

- One enum variant per error with typed fields (the previous design): precise for Rust callers, but every binding must match every variant to build messages, and adding a cause changes every binding.
- `thiserror` and per-binding wording: less code in the core, but messages would differ between languages and the documentation would have no single source.
- Error values in messages: easier debugging, but record contents would end up in logs, contrary to [AGENTS.md](../../AGENTS.md).

## Evidence

- 21 codes, each triggered by a real call in the catalog test; no message contains the marker placed in every field value of the examples.
- `size_of::<Result<(), Error>>()` equals one pointer; preparation of the standard workload is unchanged (about 850 ns per record), since errors are only built on failure.

## Consequences

- A new failure cause requires a code, a constructor, an example and a regenerated catalog; the tests enforce all four.
- Rust callers match on `ErrorCode` and `ErrorKind`, both `#[non_exhaustive]`, rather than on struct variants.
- Codes are part of the public contract: renaming one is a breaking change.

## Revisit when

- A binding needs localized messages: the code and details already allow rendering them outside the core.
- Structured details need nested values, such as a list of candidate fields.
