"""Build an entity-disjoint name benchmark from the pinned CC0 ROR dump.

No network access. Supply the versioned ZIP; only names and ROR IDs are used.
Location metadata (GeoNames CC BY) is deliberately not exported.
"""

import argparse
import hashlib
import json
import zipfile
from collections import defaultdict
from pathlib import Path
from typing import Any

ROR_SHA256 = "555b38a4b586759de1fa278b0e751433a13dd541f375097db53b186cfcdc768a"
ROR_MEMBER = "v2.14-2026-10-06-ror-data.json"
PROTOCOL = "ror-alias-v2"
# Distinct organizations of the same split that ROR lists with the same
# acronym, paired with that acronym: at most this many per acronym alias.
ACRONYM_NEGATIVES = 2


def split(entity: str) -> str:
    bucket = int(hashlib.sha256(entity.encode()).hexdigest()[:8], 16) % 10
    return "train" if bucket < 6 else "validation" if bucket < 8 else "test"


def entity_order(entity: str) -> bytes:
    return hashlib.sha256(entity.encode()).digest()


def display_name(record: dict[str, Any]) -> dict[str, Any]:
    return next(n for n in record["names"] if "ror_display" in n["types"])


def shared_acronyms(records: list[dict[str, Any]]) -> dict[tuple[str, str], list[Any]]:
    """Active organizations of the whole dump by split and case-folded acronym."""
    index: dict[tuple[str, str], list[Any]] = defaultdict(list)
    for record in records:
        if record["status"] != "active":
            continue
        for name in record["names"]:
            if "acronym" in name["types"]:
                key = (split(record["id"]), name["value"].casefold())
                index[key].append((entity_order(record["id"]), record["id"], display_name(record)))
    for candidates in index.values():
        candidates.sort(key=lambda candidate: candidate[0])
    return index


def build(records: list[dict[str, Any]], entities: int = 4000) -> list[dict[str, Any]]:
    acronyms = shared_acronyms(records)
    eligible = []
    for record in records:
        names = record["names"]
        display = display_name(record)
        aliases = sorted(
            (n for n in names if n["value"] != display["value"]), key=lambda n: n["value"]
        )[:3]
        if record["status"] == "active" and aliases:
            eligible.append((record["id"], display, aliases))
    eligible.sort(key=lambda r: entity_order(r[0]))
    groups: dict[str, list[Any]] = defaultdict(list)
    for entity, display, aliases in eligible[:entities]:
        groups[split(entity)].append((entity, display, aliases))
    pairs = []
    for subset, group in sorted(groups.items()):
        seen_negatives: set[tuple[str, str]] = set()
        group.sort(key=lambda r: r[1]["value"].casefold())
        for index, (entity, display, aliases) in enumerate(group):
            for alias in aliases:
                pairs.append(pair(subset, entity, entity, display, alias, True))
            # An acronym that ROR also lists for other organizations of the
            # split is ambiguous: pair it with their names, in hash order.
            # Aliases of an entity are distinct, so these rows are too.
            for alias in aliases:
                if "acronym" not in alias["types"]:
                    continue
                others = acronyms[(subset, alias["value"].casefold())]
                chosen = [c for c in others if c[1] != entity][:ACRONYM_NEGATIVES]
                for _, other, name in chosen:
                    pairs.append(pair(subset, entity, other, alias, name, False, "shared_acronym"))
            # Adjacent sorted names are a reproducible hard-negative proxy.
            # Mid-list offset adds an easier negative; neither uses model scores.
            neighbors = {(index + 1) % len(group), (index + len(group) // 2) % len(group)} - {index}
            for neighbor in sorted(neighbors):
                other, name, _ = group[neighbor]
                key = tuple(sorted((entity, other)))
                if key not in seen_negatives:
                    pairs.append(pair(subset, entity, other, display, name, False))
                    seen_negatives.add(key)
    return pairs


def pair(
    subset: str,
    left_entity: str,
    right_entity: str,
    left: dict[str, Any],
    right: dict[str, Any],
    duplicate: bool,
    negative: str = "other_entity",
) -> dict[str, Any]:
    return {
        "split": subset,
        "left_entity": left_entity,
        "right_entity": right_entity,
        "left": left["value"],
        "right": right["value"],
        "label": duplicate,
        "lang": (left["lang"] if left["lang"] == right["lang"] else "mixed")
        if left["lang"] and right["lang"]
        else "unknown",
        "family": "alias" if duplicate else negative,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    with args.archive.open("rb") as stream:
        checksum = hashlib.file_digest(stream, "sha256").hexdigest()
    if checksum != ROR_SHA256:
        raise ValueError("ROR archive checksum differs from the pinned Zenodo version")
    with zipfile.ZipFile(args.archive) as archive:
        records = json.loads(archive.read(ROR_MEMBER))
    rows = build(records)
    args.output.write_text(
        "".join(json.dumps(row, ensure_ascii=False) + "\n" for row in rows),
        encoding="utf-8",
        newline="\n",
    )
    print(f"{len(rows)} pairs written to {args.output}")


if __name__ == "__main__":
    main()
