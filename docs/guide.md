# Guide

Everything you need to use Univo, on one page: find duplicates, keep the matches your rules forbid out, and check new records as they arrive. Every option is listed in the [API reference](reference/api.md).

## Install

```bash
pip install univo
```

Python 3.11 or later, on Linux, macOS and Windows, x86-64 and ARM. The wheel has no Python dependency and needs no compiler.

## Find duplicates in a list

```python
from univo import Finder

names = ["Northwind Traders", "Contoso", "Northwnd Traders", "NORTHWIND TRADERS"]
for match in Finder().find(names).matches:
    print(names[match.left_id], "~", names[match.right_id], round(match.score, 2))
```

```text
Northwind Traders ~ NORTHWIND TRADERS 1.0
Northwind Traders ~ Northwnd Traders 0.94
Northwnd Traders ~ NORTHWIND TRADERS 0.94
```

`find` returns every pair scoring at least `min_score` (0.8 by default), best first. A score ranks pairs from 0 to 1. **It is not a probability**: choose your threshold on your data ([best practices](#best-practices)).

## Describe your records

A `Schema` says which fields to compare, how, and which records can never match. Nested fields are reached with dots.

```python
from univo import Exact, Finder, Schema, Text

schema = Schema(
    id_field="id",
    partition_by=["tenant"],
    fields={"name": Text(), "address.city": Text(), "vat": Exact(conflict="reject")},
)
records = [
    {"id": 1, "tenant": "shop", "name": "Northwind Traders SAS", "address": {"city": "LYON"}, "vat": "FR401"},
    {"id": 2, "tenant": "shop", "name": "Northwind Traders", "address": {"city": "Lyon"}, "vat": "FR401"},
    {"id": 3, "tenant": "shop", "name": "Northwind Traders", "address": {"city": "Lyon"}, "vat": "FR999"},
    {"id": 4, "tenant": "bank", "name": "Northwind Traders", "address": {"city": "Lyon"}, "vat": "FR401"},
]
for match in Finder(schema=schema).find(records).matches:
    print(match.left_id, match.right_id, round(match.score, 2), match.reasons)
```

```text
1 2 0.94 ['name: trigrams', 'address.city: edit_distance', 'vat: exact']
```

Record 3 has another VAT number, and record 4 belongs to another tenant: neither can match, however alike they look.

| Field | Compares | Use it for |
| --- | --- | --- |
| `Text()` | Spelling: edit distance and shared trigrams, ignoring case | Names, addresses, titles, short text with typos |
| `Text(mode="semantic")` | Meaning, with a local model | Descriptions and rewordings ([models](#semantic-fields-and-models)) |
| `Exact()` | Equality: codes, numbers, emails, booleans, lists, objects | Identifiers and values that are right or wrong |
| `Exact(normalizer="hostname")` | Equality, ignoring ASCII case and a trailing dot | Hostnames and domains |

- **Vetoes.** `Exact(conflict="reject")` makes a difference rule out the pair, whatever its score: tax numbers, environments, product variants.
- **Partitions.** `partition_by` compares only records with the same values: tenants, countries, customers. Each search then reads only its partition.
- **Missing values.** `None`, an absent field or a blank text gives no evidence either way. In an `Exact` field, `""` is a value like any other.
- **Weights.** `Text(weight=2.0)` counts a field twice as much as the others.
- **IDs.** `id_field` names the field holding each record's ID, `str` or `int`. Without it, a record's ID is its position in the input.

Values can be strings, integers, floats, booleans, `None`, lists and dicts. Fields outside the schema are ignored and records are never modified. Text is compared in Unicode NFC and case folded, but whitespace is compared as it is. Five-letter acronyms and longer, such as "NIOSH", match their full names.

## Pairs or groups

`find` returns pairs. `group` returns groups of records to review together:

```python
from univo import Finder, Schema, Text

finder = Finder(schema=Schema(id_field="id", fields={"name": Text()}))
groups = finder.group(
    [
        {"id": 1, "name": "Acme Corporation"},
        {"id": 2, "name": "ACME corporation"},
        {"id": 3, "name": "Acme Corporatoin"},
        {"id": 4, "name": "Globex"},
    ]
)
print(groups.exact, [group.members for group in groups.groups])
```

```text
((1, 2),) [(1, 2, 3)]
```

- **`groups`**: every pair inside a group reaches `min_score`. If A matches B and B matches C but A does not match C, the chain stays open: similarity is not transitive.
- **`exact`**: records equal on every compared field after normalization.
- **`split_pairs`** counts matching pairs left in different groups; `find` shows them.

Neither `find` nor `group` stores the records.

## Keep an index

A `Finder` also holds an index you update and search.

```python
from univo import Exact, Finder, Schema, Text

finder = Finder(schema=Schema(id_field="id", fields={"name": Text(), "email": Exact()}))
finder.add([{"id": 1, "name": "Ada Lovelace", "email": "ada@example.org"}])

for record in [
    {"id": 2, "name": "Ada Lovelace", "email": "ada@example.org"},
    {"id": 3, "name": "Ada Lovelce", "email": None},
    {"id": 4, "name": "Grace Hopper", "email": "grace@example.org"},
]:
    print(record["id"], finder.check_or_add(record).status)
```

```text
2 matched
3 review
4 inserted
```

| Method | Does |
| --- | --- |
| `add(records)` | Adds a batch, all or nothing. |
| `upsert(records)` | Replaces records by ID, or adds them. |
| `remove(ids)` | Removes records by ID. |
| `search(record)` | Finds indexed records similar to `record`, without changing anything. |
| `check_or_add(record)` | Searches, and adds the record only if nothing reaches `min_score`. |

`check_or_add` answers `inserted` (added), `matched` (equal on every field; not added) or `review` (similar records exist; not added, your application decides). `decision.report` holds the matches. **Univo never merges or deletes records itself.**

**Threads.** One `Finder` can be shared by the threads of a process: searches run in parallel, changes one at a time, and two threads never both add the same duplicate. This does not extend to several processes or to a database: enforce uniqueness where the data lives, with a unique constraint or a transaction. `threads` sets the workers for encoding, `find` and `group`; with several processes, give each its share of the cores.

## Save and load

```python
from univo import Finder

finder = Finder()
finder.add(["Northwind Traders", "NORTHWIND TRADERS", "Contoso"])
finder.save("./company-index")

restored = Finder.load("./company-index")
print(restored.stats()["records"])
```

```text
3
```

- **What is saved.** IDs, partition keys, compared fields in normalized form, and vectors. Loading does not encode again. Changes made after `save` are lost unless you save again.
- **Compatibility.** A snapshot from a release with another format or scoring version is refused with `IndexCompatibilityError`: rebuild it from your records. A semantic snapshot reloads the exact model it was built with.
- **Safety.** Files are checksummed, not encrypted: keep them as you keep the data. One process saves at a time, and readers always see a complete snapshot. If `save` raises an error, load the snapshot to know whether the new generation was published.
- **Disk space.** Each save adds a generation and keeps older ones for open readers. When no process uses the snapshot, delete the old `g-*` directories, keeping `CURRENT` and the one it names.

## Semantic fields and models

Semantic fields compare meaning with a local [Model2Vec](https://github.com/MinishLab/model2vec) model. Records never leave the machine: the only network access is the model download, once.

```python
from univo import Finder, Schema, Text

finder = Finder(schema=Schema(fields={"description": Text(mode="semantic")}))
```

| `model=` | Uses |
| --- | --- |
| omitted or `"default"` | 15 MB, 128 dimensions, 50 languages, pinned and verified. It catches close rewordings. |
| `"org/name"` or `"org/name@revision"` | Any Model2Vec model on Hugging Face. A larger one may do better on paraphrases; measure it on your data. |
| `model_path="./my-model"` | A local directory with `config.json`, `model.safetensors` and `tokenizer.json`. |

- **When it loads.** At the first semantic operation, never at import or `Finder()`.
- **Offline.** `offline=True` never connects and uses only the cache or a local model. Prepare a machine or an image with `python -m univo.models download`.
- **Cache.** Models are kept in `UNIVO_CACHE_DIR`, or the platform's cache directory, and verified on every load.
- **Mirrors.** `hub_url=` or `HF_ENDPOINT` names a Hugging Face compatible mirror. Requests carry only repository and file names.
- **Repeated texts** are encoded once, within `embedding_cache_bytes` (4 MiB by default).

## Large datasets

- **Partition.** Search cost grows with the records of a partition: `partition_by` splits it.
- **Raise the threshold.** Fewer candidates reach 0.8 than 0.6.
- **Raise the limits.** An index holds 100,000 records by default: pass `max_records`. Going over a limit raises `ResourceLimitError`, never a silent partial result.
- **Semantic batches.** `find` and `group` with a semantic field first check a work budget: about n² vector comparisons for n records in one partition. The default `max_semantic_comparisons=1_000_000` covers about 1,000 records per partition; raise it for larger batches.
- **Approximate semantic search.** Semantic search is exact by default. `Finder(retrieval="hnsw")` uses a graph for partitions of 10,000 records or more. It can be faster on very large partitions, depending on the data, and it can miss matches: compare it with the default on your data. Reports then say `search_profile == "approximate"`.

## Read a report

| Attribute | Means |
| --- | --- |
| `matches` | Best first, up to `limit`. |
| `total_matches` | All matches found, including those beyond `limit`. |
| `output_truncated` | More matches than `limit`. |
| `candidate_truncated` | A work limit (`max_entries`) was reached: matches may be missing. |
| `search_profile` | `"exhaustive"`, `"bounded"` (the default search, complete unless truncated) or `"approximate"`. |

Each `Match` explains itself: `field_scores`, `reasons`, `conflicts`, and `coverage`, the share of the schema that gave evidence. **A high score with a low coverage rests on few fields**, for example a shared first name when everything else is missing.

Errors carry a stable `code` and a `hint`; every code is in the [error catalog](reference/errors.md).

## Best practices

**Describe the records.**

- **Compare the fields that identify a record.** Adding an address, a phone number or a code raises recall at the same precision: from 69% to 77% on the [Chicago example](examples/chicago.md).
- **Turn hard rules into vetoes and partitions** rather than hoping scores keep records apart.

**Prepare the values.**

- **Clean formats that differ between sources.** Collapse spaces, and keep the digits or a canonical form of phone numbers and codes.
- **Leave missing values missing.** A placeholder such as `"N/A"` or `"0000"` is a value: records sharing it look alike.

**Choose the threshold.**

- **Measure on labelled pairs from your data.** Label a few hundred pairs across the score range, keep half to choose the threshold and half to check it.
- **Decide what an error costs.** Favour precision for automatic actions, recall for a review queue.
- **Check the coverage** of matches when many values are missing.

**Act on the results.**

- **Review before merging.** A match is a proposal with its reasons.
- **Check new records as they arrive** with `check_or_add`, and save the index with `save`.
- **Read the report flags** before trusting that nothing was missed.
