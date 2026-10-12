# Python Finder

U-011 exposes the Rust engine through `Finder`, `Schema`, `Text`, `Exact`, `Match` and `Report`. The package is not published yet; use the [development setup](development.md) or an internally built wheel. No Python runtime dependency is required. The examples use synthetic records.

```python
from univo import Finder

finder = Finder(offline=True)
report = finder.find(["Amazon", "Amiazon", "Microsoft"])
for match in report.matches:
    print(match.left_id, match.right_id, match.kind, match.score)
```

This example returns the Amazon/Amiazon pair as a candidate. Its score ranks lexical resemblance; it is not an identity probability. `Finder()` and `fields=[...]` use `generic-v1`: equal-weight hybrid text, Unicode NFC and case folding, scored lexically under [ADR 0009](adr/0009-semantic-evidence-in-scoring.md). This path needs no model, download or worker pool. Explicit semantic fields load the configured model lazily on the first operation, through the same verified registry and strict offline path as [univo.models](models.md).

```python
from univo import Exact, Finder, Schema, Text

schema = Schema(
    id_field="id",
    partition_by=["tenant"],
    fields={
        "name": Text(),
        "environment": Exact(conflict="reject"),
    },
)
finder = Finder(schema=schema, offline=True)
records = [
    {"id": "org-1", "tenant": "shop", "name": "Amazon", "environment": "prod"},
    {"id": "org-2", "tenant": "shop", "name": "AMAZON", "environment": "prod"},
    {"id": "org-3", "tenant": "shop", "name": "Amazon", "environment": "test"},
]
ids = finder.add(records)
report = finder.search(records[0])
assert [match.right_id for match in report.matches] == ["org-2"]
```

`find` creates a temporary index and never changes the stored records or generation. It reports each unordered pair once. `add` validates and encodes a bounded batch before committing; it returns typed external IDs or generated integer slots when no `id_field` is configured. An ID collision rejects the entire batch, across partitions too. Integer `1` and string `"1"` are distinct IDs. Generated slots are stable within the instance; updates by ID belong to U-015. With no ID field, a query has no external ID to exclude. `search` is read-only and excludes an existing query ID when the schema provides one.

Only the selected fields, identifier and partition keys are converted. Values outside that projection can be arbitrary Python objects. Supported selected values are strings, signed 64-bit integers, finite floats, booleans, null, lists and dictionaries with string keys. No implicit `str()` call is made. Inputs are never changed. Nested dotted paths are supported; an object and its child cannot both be selected. Required fields, types, normalizers, vetoes and missing evidence follow the shared Rust contracts. Errors expose a stable `code`, `path`, `hint`, structured `details` and, for batches, `record_index`; see the [error catalog](errors.md).

## Limits and reports

| Option | Default | Meaning |
| --- | --- | --- |
| `min_score` on find/search | `0.8` | Uncalibrated convenience cutoff, in `(0, 1]`; calibration is U-012 |
| `find(..., limit=...)` | `1000` | Maximum returned pairs, globally ranked |
| `search(..., limit=...)` | `10` | Maximum returned records |
| `max_entries` on find/search | `None` | Work per lexical/value lane; `None` uses the largest supported budget; exact and semantic lanes are not capped by it |
| `exhaustive` on find/search | `False` | Score every admissible pair/record rather than use candidate lanes |
| `Finder(max_records=...)` | `100000` | Capacity of each persistent or temporary index; iterables stop at capacity plus one |
| `Finder(max_exhaustive_pairs=...)` | `1000000` | Maximum pair-count upper bound for an exhaustive `find`, including when output limit is zero |
| `Schema(max_selected_bytes=...)` | `1048576` | Bytes per selected record, with node/key accounting |
| `Schema(max_depth=...)` | `16` | Nesting in a selected value; cycles hit this bound |
| `Schema(max_fields=...)` | `64` | Selected field count |
| `Finder(max_semantic_comparisons=...)` | `1000000` | Maximum dense semantic lane entries for candidate `find`; raise explicitly for large jobs |
| `threads` | `1` | Semantic encoding pool budget; lexical retrieval/scoring currently use one thread |

`Report.matches` is a bounded tuple of `Match` values. `processed` counts input records (one for search). `total_matches` counts qualifying matches found, including output omitted by `limit`. `candidates` counts comparisons attempted after self/ID exclusion; find counts unordered pairs, search counts records. Hash collisions may contribute attempted cross-partition comparisons, which Rust still rejects.

`output_truncated` means more qualifying matches were found than returned. `candidate_truncated` means a candidate lane exhausted its work budget and qualifying matches might have been missed. `warnings` distinguishes these two cases. `search_profile` is `exhaustive` only with the explicit oracle; the candidate union reports `bounded` even when its lanes are complete. No ANN is used in U-011. Verified whole-record equality remains exact even when semantic texts contain no known token ([ADR 0010](adr/0010-python-finder-and-exact-fallback.md)); distinct unknown texts give no semantic evidence. Ranking is by descending score, then left input position and right typed ID/slot for find, or typed ID/slot for search.

The pair budget sums `n*(n-1)/2` within partition fingerprint buckets. Collisions can conservatively overestimate it, never underestimate it. Above the budget Rust raises `ResourceLimitError` (`limit.pair_count`) before scoring. Candidate semantic find also checks `semantic_fields * sum(n*n)` over partition fingerprint buckets before retrieval, including both directions and self entries. Above `max_semantic_comparisons`, Rust raises `ResourceLimitError` (`limit.semantic_comparisons`). Missing vectors and fingerprint collisions can conservatively overestimate this bound. Neither `limit=0` nor `max_entries=0` bypasses it. For example, explicitly pass `Finder(max_semantic_comparisons=10_000_000)` to authorize a larger scan. This guard runs after input preparation/encoding; it bounds retrieval, not model loading. Invalid `min_score` is validated in Rust before model resolution, including non-finite values and values that underflow the engine’s float32 representation. Candidate-mode find still has a worst case of quadratic work for dense matches; `limit` bounds output memory, not total work. Temporary find uses an additional index. The per-record/capacity limits do not enforce a total RSS cap. Batch generators must be finite within these limits; there is no transaction over an unbounded stream.

`config()` returns field options, partitions, policy/normalizer/scoring/search versions, configured and resolved model, requested/effective thread counts and limits. `config_fingerprint` identifies schema/scoring/model evidence within this build, including verified model descriptors after loading. It is not a snapshot compatibility format. A fresh configured semantic Finder has an unresolved fingerprint until the model loads. `stats()` returns record count, generation and estimated index bytes; the estimate excludes prepared records, model weights, Python input/output and allocator overhead. No business values are reported.

The Python wrapper serializes calls on each Finder with a reentrant lock. Long Rust work detaches from the interpreter. Semantic encoding uses an instance index pool rather than multiplying it with a global tokenizer pool; temporary and persistent operations run serially. This is an instance guarantee, not database or distributed atomicity.

## Local semantic model

```python
from univo import Finder, Schema, Text

finder = Finder(
    schema=Schema(fields={"description": Text(mode="semantic")}),
    model="/opt/models/univo/default-v1",
    offline=True,
    threads=2,
)
# The directory must contain a verified, supported Model2Vec model before add/search/find.
```

`model_path` accepts a filesystem path explicitly and is exclusive of `model`. `model="org/name"` is always a hub repository, never guessed to be a relative directory. `hub_url` and `cache_dir` retain the U-009 semantics. `Finder(model="minishlab/potion-multilingual-128M")` accepts the reference, but only an explicit semantic field uses it; the current model resolver also enforces its download-size limit. This task does not change the 100 MiB default-model budget or select a multilingual replacement.

Persistence, upsert/remove, embedding reuse across operations, calibrated thresholds and ANN remain later roadmap tasks.

The first [offline quality benchmark](../benchmarks/quality/README.md) does not meet the precision/recall gates on organization-name aliases. Its validation thresholds are experimental measurements, not supported production profiles. The default score is still uncalibrated.
