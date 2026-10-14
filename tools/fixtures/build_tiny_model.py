"""Build a tiny Model2Vec model for tests: tests/fixtures/models/tiny.

A word-level tokenizer over a small vocabulary and 8-dimensional token
vectors computed from a fixed formula, in the same three files as a real
Model2Vec model. It lets tests load, verify and cache a model without
network access. Regenerate with `python tools/fixtures/build_tiny_model.py`;
`--check` fails if the files are stale.
"""

import argparse
import json
import pathlib
import struct
import sys

OUTPUT = pathlib.Path(__file__).resolve().parents[2] / "tests" / "fixtures" / "models" / "tiny"
DIMENSION = 8
VOCABULARY = [
    "[UNK]",
    "amazon",
    "amiazon",
    "microsoft",
    "online",
    "retail",
    "retailer",
    "cloud",
    "payments",
    "software",
    "café",
    "société",
    "générale",
]


def vector(index: int) -> list[float]:
    """Distinct, non-zero rows of multiples of 1/4, exact in float32 on every platform."""
    return [((3 * index + 5 * component) % 11 - 5) / 4 for component in range(DIMENSION)]


def safetensors(rows: list[list[float]]) -> bytes:
    data = b"".join(struct.pack("<f", value) for row in rows for value in row)
    header = json.dumps(
        {
            "embeddings": {
                "dtype": "F32",
                "shape": [len(rows), DIMENSION],
                "data_offsets": [0, len(data)],
            }
        },
        separators=(",", ":"),
    ).encode()
    header += b" " * (-len(header) % 8)
    return struct.pack("<Q", len(header)) + header + data


def tokenizer() -> dict[str, object]:
    return {
        "version": "1.0",
        "truncation": None,
        "padding": None,
        "added_tokens": [
            {
                "id": 0,
                "content": "[UNK]",
                "single_word": False,
                "lstrip": False,
                "rstrip": False,
                "normalized": False,
                "special": True,
            }
        ],
        "normalizer": {"type": "Lowercase"},
        "pre_tokenizer": {"type": "Whitespace"},
        "post_processor": None,
        "decoder": None,
        "model": {
            "type": "WordLevel",
            "vocab": {token: index for index, token in enumerate(VOCABULARY)},
            "unk_token": "[UNK]",
        },
    }


def outputs() -> dict[pathlib.Path, bytes]:
    config = {"model_type": "model2vec", "normalize": True, "hidden_dim": DIMENSION}
    rows = [vector(index) for index in range(len(VOCABULARY))]
    return {
        OUTPUT / "config.json": (json.dumps(config, indent=2) + "\n").encode(),
        OUTPUT / "tokenizer.json": (
            json.dumps(tokenizer(), indent=2, ensure_ascii=False) + "\n"
        ).encode(),
        OUTPUT / "model.safetensors": safetensors(rows),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="fail if outputs are stale")
    arguments = parser.parse_args()

    files = outputs()
    if arguments.check:
        stale = [
            path
            for path, content in files.items()
            if not path.is_file() or path.read_bytes() != content
        ]
        for path in stale:
            print(f"{path} is stale; run tools/fixtures/build_tiny_model.py", file=sys.stderr)
        return 1 if stale else 0
    OUTPUT.mkdir(parents=True, exist_ok=True)
    for path, content in files.items():
        path.write_bytes(content)
    print(f"wrote {OUTPUT}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
