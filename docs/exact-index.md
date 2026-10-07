# Exact index (U-007)

`univo-engine` indexes prepared Rust records for equality after normalization. It does not yet provide lexical or semantic search, Python `Finder`, persistence or replacement/removal. Its prerequisite is U-006. Acceptance is agreement with the exhaustive `exact_match` oracle, no false match under forced hash collisions, no cross-partition match and no partial insertion on an invalid batch.

```rust
use univo_core::{Schema, Value, prepare};
use univo_engine::ExactIndex;

let schema = Schema::strings();
let records = ["Café", "CAFE\u{301}", "other"]
    .map(|text| prepare(&schema, &Value::Str(text.into())).unwrap());
let mut index = ExactIndex::new(schema, 100);
index.add(records.to_vec()).unwrap();
let result = index.search(&records[0], 10).unwrap();
assert_eq!(result.total_matches, 2);
assert!(!result.truncated());
```

For strings without IDs, results name temporary slots, so searching one of the indexed values also returns its own slot. An external query ID excludes that same typed ID. Integer `1` and string `"1"` remain different IDs. Duplicate IDs are rejected globally, including across tenants; identifiers are never printed in errors.

`add` owns a finite `Vec<PreparedRecord>` and returns the inserted slot range. It checks all records before changing the index: schema compatibility, the explicit record-count limit, and duplicates within the batch or existing index. Invalid batches leave the index unchanged. Individual raw values must first pass `prepare`; this layer does not consume unbounded generators or promise recovery from allocator exhaustion.

`search` verifies the entire fingerprint bucket using the core's `exact_match`. IDs, explanation traces and hash equality alone are not match evidence. Partitions and all selected states must agree, including absent versus null, with at least one informative field. Results retain normalization explanations. String IDs sort before integer IDs; each type uses its natural order. Without IDs, slots sort in insertion order.

The output limit bounds stored hits, not candidate retrieval. `total_matches` counts all verified matches, including when the output limit is zero. `truncated()` indicates omitted output, and `candidates_examined` includes every bucket entry visited, including an excluded query ID. These counters describe exact equality only; they do not certify the absence of lexical or semantic duplicates.

The index keeps one owned copy of each prepared record, an ID set and fingerprint-to-slot postings. Keys include typed partition values and selected field states, excluding IDs and normalization traces. Batches stage keys and ID references before insertion. Records without IDs append their slots to a vector, already in output order. Records with IDs insert into a `BTreeMap`, maintaining typed-ID order in O(log B) for a bucket of size B. No insertion re-sorts existing postings. Ordered postings hold another copy of each external ID and tree-node storage; unidentified records need neither. Search output memory is O(min(limit, matches)) plus per-pair explanation storage. A bucket can contain every record, so search can still cost O(N), and counting every hit necessarily scans the whole bucket.

`max_records` is a count limit, not a measured byte/RSS budget. The in-memory fingerprint is non-cryptographic and has no cross-version or cross-platform persistence contract. Collision tests protect correctness, not worst-case performance against adversarial hashing. No performance target or lightweight wheel improvement is claimed by this step; the engine is not yet linked into Python.
