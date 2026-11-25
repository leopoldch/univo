"""Public record datasets with known duplicates, in one common form.

Each dataset is downloaded from its publisher at a pinned location, checked
by SHA-256, and turned into records `{"id", "source", "entity", ...fields}`:

- `febrl3`: 5,000 synthetic persons, 2,000 of them originals with up to five
  altered copies each. Deduplication.
- `febrl4`: 5,000 synthetic persons and one altered copy of each. Linking.
- `historical_50k`: 50,578 records of 5,156 historical persons from Wikidata,
  with errors introduced. Deduplication.
- `dblp_acm`: 2,616 publications of DBLP and 2,294 of the ACM Digital
  Library, 2,224 of them the same. Linking.
- `abt_buy`: 1,081 products of Abt and 1,092 of Buy, 1,097 matching pairs.
  Linking.

`source` is `""` for deduplication and `"a"` or `"b"` for linking. Records
are split in two halves by a hash of their entity, so every record of an
entity, and both sides of a true link, land in the same half.

    python benchmarks/records/record_datasets.py --cache /tmp/records-data

writes `<name>.jsonl` and `<name>.truth.jsonl` for every dataset. Only
`historical_50k` needs a library, `pyarrow`, to read its source file.
"""

from __future__ import annotations

import argparse
import csv
import hashlib
import io
import itertools
import json
import re
import urllib.request
import zipfile
from collections import defaultdict
from dataclasses import dataclass
from pathlib import Path

Record = dict[str, str | None]
Pair = tuple[str, str]

SPLINK_DATA = (
    "https://raw.githubusercontent.com/moj-analytical-services/splink_datasets/"
    "75876d806d9eff72072d21878150ee50a96d5f41/data/"
)
LEIPZIG = "https://dbs.uni-leipzig.de/files/datasets/"
SOURCES = {
    "dataset3.csv": (
        SPLINK_DATA + "febrl/dataset3.csv",
        "0e667330458ae88dd3d6b9cab39af4e7629a2fef98a810d0ea5f15e48220bdbf",
    ),
    "dataset4a.csv": (
        SPLINK_DATA + "febrl/dataset4a.csv",
        "797ffc5072b5d6cbfdcd0bcb603b92be8916fa67fe8e180db366994c5389d3a0",
    ),
    "dataset4b.csv": (
        SPLINK_DATA + "febrl/dataset4b.csv",
        "af0330f0c355b7b12006ac0b8a2de23539d3bc868153f9416fc5ae43cc5f401c",
    ),
    "historical_figures_with_errors_50k.parquet": (
        SPLINK_DATA + "historical_figures_with_errors_50k.parquet",
        "b8b748402ff37b15c2d5e8a5b9374abe269b9d297aacb7f4c9498365b7cace43",
    ),
    "DBLP-ACM.zip": (
        LEIPZIG + "DBLP-ACM.zip",
        "e37e7ed8e06722499e6d9e94583fdec279207b13ce6270a37595b6a5d594bc40",
    ),
    "Abt-Buy.zip": (
        LEIPZIG + "Abt-Buy.zip",
        "b9ff2937d97371b7a00b0a97033213bfbdee24e8f1a0718a41ea8364472a8ff8",
    ),
}
NAMES = ("febrl3", "febrl4", "historical_50k", "dblp_acm", "abt_buy")
HALVES = ("validation", "test")


@dataclass
class Dataset:
    name: str
    linking: bool
    fields: tuple[str, ...]
    records: list[Record]
    truth: set[Pair]

    def half(self, half: str) -> list[Record]:
        return [record for record in self.records if half_of(record) == half]

    def truth_of(self, half: str) -> set[Pair]:
        ids = {record["id"] for record in self.half(half)}
        return {pair for pair in self.truth if pair[0] in ids}


def pair(left: str, right: str) -> Pair:
    return (left, right) if left <= right else (right, left)


def half_of(record: Record) -> str:
    digest = hashlib.sha256(str(record["entity"]).encode()).digest()
    return HALVES[digest[0] % 2]


def fetch(name: str, cache: Path) -> bytes:
    """The source file `name`, downloaded once into `cache` and checked."""
    url, sha256 = SOURCES[name]
    path = cache / "sources" / name
    if not path.exists():
        path.parent.mkdir(parents=True, exist_ok=True)
        with urllib.request.urlopen(url, timeout=120) as response:
            path.write_bytes(response.read())
    content = path.read_bytes()
    if hashlib.sha256(content).hexdigest() != sha256:
        raise RuntimeError(f"unexpected content for {name}: delete {path} and retry")
    return content


def clean(value: object) -> str | None:
    text = " ".join(str(value).split()) if value is not None else ""
    return text or None


def table(content: bytes, encoding: str = "utf-8") -> list[dict[str, str]]:
    text = io.StringIO(content.decode(encoding), newline="")
    return list(csv.DictReader(text, skipinitialspace=True))


def within_entities(records: list[Record]) -> set[Pair]:
    members: dict[str, list[str]] = defaultdict(list)
    for record in records:
        members[str(record["entity"])].append(str(record["id"]))
    return {pair(a, b) for ids in members.values() for a, b in itertools.combinations(ids, 2)}


def febrl(rows: list[dict[str, str]], source: str) -> list[Record]:
    records = []
    for row in rows:
        entity = re.fullmatch(r"rec-(\d+)-(org|dup-\d+)", row["rec_id"].strip())
        if entity is None:
            raise RuntimeError(f"unexpected Febrl identifier {row['rec_id']!r}")
        record: Record = {"id": row["rec_id"].strip(), "source": source, "entity": entity[1]}
        record.update({key: clean(value) for key, value in row.items() if key != "rec_id"})
        records.append(record)
    return records


FEBRL_FIELDS = (
    "given_name",
    "surname",
    "street_number",
    "address_1",
    "address_2",
    "suburb",
    "postcode",
    "state",
    "date_of_birth",
    "soc_sec_id",
)


def febrl3(cache: Path) -> Dataset:
    records = febrl(table(fetch("dataset3.csv", cache)), "")
    return Dataset("febrl3", False, FEBRL_FIELDS, records, within_entities(records))


def febrl4(cache: Path) -> Dataset:
    records = febrl(table(fetch("dataset4a.csv", cache)), "a")
    records += febrl(table(fetch("dataset4b.csv", cache)), "b")
    return Dataset("febrl4", True, FEBRL_FIELDS, records, within_entities(records))


def historical_50k(cache: Path) -> Dataset:
    import pyarrow.parquet as parquet

    content = fetch("historical_figures_with_errors_50k.parquet", cache)
    fields = (
        "full_name",
        "first_name",
        "surname",
        "dob",
        "birth_place",
        "postcode_fake",
        "gender",
        "occupation",
    )
    records = []
    for row in parquet.read_table(io.BytesIO(content)).to_pylist():
        record: Record = {"id": row["unique_id"], "source": "", "entity": row["cluster"]}
        record.update({field: clean(row[field]) for field in fields})
        records.append(record)
    return Dataset("historical_50k", False, fields, records, within_entities(records))


def linked(
    name: str,
    cache: Path,
    archive: str,
    files: tuple[str, str, str],
    fields: tuple[str, ...],
) -> Dataset:
    """Two tables of a Leipzig benchmark and their published mapping."""
    with zipfile.ZipFile(io.BytesIO(fetch(archive, cache))) as zipped:
        left, right, mapping = (table(zipped.read(file), "latin-1") for file in files)
    # An entity is a connected set of mapped records: it never straddles halves.
    parent: dict[str, str] = {}

    def root(node: str) -> str:
        while parent.setdefault(node, node) != node:
            parent[node] = node = parent[parent[node]]
        return node

    truth = set()
    for row in mapping:
        a, b = (f"{side}:{value.strip()}" for side, value in zip("ab", row.values(), strict=True))
        parent[root(a)] = root(b)
        truth.add(pair(a, b))
    records = []
    for side, rows in (("a", left), ("b", right)):
        for row in rows:
            key = f"{side}:{row['id'].strip()}"
            record: Record = {"id": key, "source": side, "entity": root(key)}
            record.update({field: clean(row.get(field)) for field in fields})
            records.append(record)
    return Dataset(name, True, fields, records, truth)


def dblp_acm(cache: Path) -> Dataset:
    files = ("DBLP2.csv", "ACM.csv", "DBLP-ACM_perfectMapping.csv")
    return linked("dblp_acm", cache, "DBLP-ACM.zip", files, ("title", "authors", "venue", "year"))


def abt_buy(cache: Path) -> Dataset:
    files = ("Abt.csv", "Buy.csv", "abt_buy_perfectMapping.csv")
    fields = ("name", "description", "manufacturer", "price")
    return linked("abt_buy", cache, "Abt-Buy.zip", files, fields)


BUILDERS = {
    "febrl3": febrl3,
    "febrl4": febrl4,
    "historical_50k": historical_50k,
    "dblp_acm": dblp_acm,
    "abt_buy": abt_buy,
}


def load(name: str, cache: Path) -> Dataset:
    """The dataset, from `<cache>/<name>.jsonl` when it was already written."""
    records_path, truth_path = cache / f"{name}.jsonl", cache / f"{name}.truth.jsonl"
    if not (records_path.exists() and truth_path.exists()):
        dataset = BUILDERS[name](cache)
        cache.mkdir(parents=True, exist_ok=True)
        header = {"name": name, "linking": dataset.linking, "fields": dataset.fields}
        lines = [header, *dataset.records]
        records_path.write_text("".join(json.dumps(line) + "\n" for line in lines))
        truth_path.write_text("".join(json.dumps(p) + "\n" for p in sorted(dataset.truth)))
    header, *records = (json.loads(line) for line in records_path.read_text().splitlines())
    truth = {pair(*json.loads(line)) for line in truth_path.read_text().splitlines()}
    return Dataset(name, header["linking"], tuple(header["fields"]), records, truth)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--cache", type=Path, required=True)
    arguments = parser.parse_args()
    for name in NAMES:
        dataset = load(name, arguments.cache)
        entities = {record["entity"] for record in dataset.records}
        for half in HALVES:
            print(
                f"{name} {half}: {len(dataset.half(half))} records, "
                f"{len(dataset.truth_of(half))} true pairs"
            )
        print(f"{name}: {len(dataset.records)} records, {len(entities)} entities")


if __name__ == "__main__":
    main()
