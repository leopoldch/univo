"""Export local model vectors for disjoint ROR background/query name splits.

Use the pinned U-012 corpus prepared with tools/export_ror_corpus.py. No network,
no model training and no threshold tuning; the output is benchmark-only f32.
"""

import argparse
import hashlib
import json
import struct
import unicodedata
from pathlib import Path

from univo.models import _load


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("pairs", type=Path)
    parser.add_argument("model", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    names: dict[str, set[str]] = {"train": set(), "validation": set(), "test": set()}
    for line in args.pairs.read_text().splitlines():
        pair = json.loads(line)
        for name in (pair["left"], pair["right"]):
            names[pair["split"]].add(unicodedata.normalize("NFC", name.casefold()))
    background = names["train"] | names["validation"]
    queries = names["test"] - background
    model = _load(str(args.model.resolve()), hub_url=None, offline=True, cache_dir=None)
    args.output.mkdir(parents=True, exist_ok=True)
    counts = {}
    for label, texts in (("background", background), ("queries", queries)):
        rows: list[bytes] = []
        ordered = sorted(texts)
        for start in range(0, len(ordered), 1024):
            chunk = ordered[start : start + 1024]
            data, absent = model.encode(chunk)
            missing = set(absent)
            stride = model.dimension * 4
            rows.extend(
                data[i * stride : (i + 1) * stride] for i in range(len(chunk)) if i not in missing
            )
        (args.output / f"{label}.bin").write_bytes(
            struct.pack("<QQ", len(rows), model.dimension) + b"".join(rows)
        )
        counts[label] = {"texts": len(ordered), "vectors": len(rows)}
    (args.output / "metadata.json").write_text(
        json.dumps(
            {
                "counts": counts,
                "dimension": model.dimension,
                "model_revision": args.model.name,
                "corpus_sha256": hashlib.sha256(args.pairs.read_bytes()).hexdigest(),
                "splits": (
                    "background train+validation; queries held-out test, "
                    "identical normalized texts excluded"
                ),
            },
            indent=2,
        )
        + "\n"
    )


if __name__ == "__main__":
    main()
