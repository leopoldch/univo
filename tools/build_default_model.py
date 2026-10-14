"""Build Univo's default model from its pinned upstream source.

The default model is `sentence-transformers/static-similarity-mrl-multilingual-v1`
(Apache-2.0), converted to the Model2Vec layout, quantized to int8 and truncated
to its first 128 Matryoshka dimensions (ADR 0021). This script reproduces the
published files byte for byte with the standard library only:

    python tools/build_default_model.py --output build/default-model

It downloads the two upstream files at a pinned commit (or reads them from
`--source`), verifies them, writes `config.json`, `model.safetensors`,
`tokenizer.json`, the model card and the license, then checks every output
against the checksums below. A mismatch is an error: the published model and
the checksums compiled into Univo must come from this exact recipe.
"""

import argparse
import array
import hashlib
import json
import pathlib
import shutil
import struct
import sys
import tempfile
import urllib.request

UPSTREAM = "sentence-transformers/static-similarity-mrl-multilingual-v1"
UPSTREAM_REVISION = "b68f4122911bcffcd6e1f695f2d99cd6788972d8"
UPSTREAM_FILES = {
    "0_StaticEmbedding/model.safetensors": (
        433_680_480,
        "8245ab78ee71dded845a82d2270fcb9e785b29dad0e1619f69d5390c47d9ba00",
    ),
    "0_StaticEmbedding/tokenizer.json": (
        2_563_370,
        "11aaf894a4ccf3d95e8830e27c0f8152791fbbff2b988e29a265580b86edd216",
    ),
}

DIMENSION = 128
MAX_LENGTH = 512
# Checksums of the model files, also pinned in crates/univo-embed/src/registry.rs.
OUTPUT_FILES = {
    "config.json": (79, "78f117ff0287650054a2a596313e72fe2e3dec4e4cc0d6348985a6e50cc19028"),
    "model.safetensors": (
        13_552_600,
        "73e3c5e5496b5ddf9070238139b701839d763a479ab8023ec25bc951f780b6fc",
    ),
    "tokenizer.json": (
        1_715_270,
        "88d69c30a2a4395cf169631e47f3c4fbb95d300650028d541ec531ed18eb0ff1",
    ),
}
CARD_FILES = ("README.md", "LICENSE")
CARD_DIRECTORY = pathlib.Path(__file__).resolve().parent / "default_model"


def sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as file:
        for chunk in iter(lambda: file.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def verify(path: pathlib.Path, size: int, checksum: str) -> None:
    actual = (path.stat().st_size, sha256(path))
    if actual != (size, checksum):
        raise SystemExit(
            f"{path} does not match: size {actual[0]}, sha256 {actual[1]}; "
            f"expected size {size}, sha256 {checksum}"
        )


def fetch_upstream(source: pathlib.Path | None, directory: pathlib.Path) -> pathlib.Path:
    """Returns a directory holding the verified upstream files."""
    if source is None:
        source = directory
        for name in UPSTREAM_FILES:
            target = source / name
            target.parent.mkdir(parents=True, exist_ok=True)
            url = f"https://huggingface.co/{UPSTREAM}/resolve/{UPSTREAM_REVISION}/{name}"
            print(f"downloading {url}", file=sys.stderr)
            with urllib.request.urlopen(url) as response, target.open("wb") as file:
                shutil.copyfileobj(response, file, 1 << 20)
    for name, (size, checksum) in UPSTREAM_FILES.items():
        verify(source / name, size, checksum)
    return source


def read_embeddings(path: pathlib.Path) -> tuple[array.array[float], int, int]:
    data = path.read_bytes()
    (header_length,) = struct.unpack_from("<Q", data)
    header = json.loads(data[8 : 8 + header_length])
    (tensor,) = (value for key, value in header.items() if key != "__metadata__")
    if tensor["dtype"] != "F32" or len(tensor["shape"]) != 2:
        raise SystemExit(f"unexpected upstream tensor {tensor}")
    rows, columns = tensor["shape"]
    start, end = tensor["data_offsets"]
    values = array.array("f")
    values.frombytes(data[8 + header_length + start : 8 + header_length + end])
    if sys.byteorder == "big":
        values.byteswap()
    return values, rows, columns


def quantize(values: array.array[float], rows: int, columns: int) -> bytes:
    """Model2Vec's int8 recipe, then Matryoshka truncation.

    One scale for the whole matrix (largest magnitude over all 1024 columns
    divided by 127); each value goes through float16, is divided by the scale in
    float32, rounded half to even in float16 and clipped to [-127, 127] (a no-op
    kept from the recipe: the scale already bounds every value). Only
    the first DIMENSION columns are kept. Cosine similarity ignores the global
    scale, so it is not stored.
    """
    to_float32 = struct.Struct(f"<{DIMENSION}f")
    to_float16 = struct.Struct(f"<{DIMENSION}e")
    (scale,) = struct.unpack("<f", struct.pack("<f", max(max(values), -min(values)) / 127.0))
    output = bytearray(rows * DIMENSION)
    for row in range(rows):
        offset = row * columns
        halves = to_float16.unpack(to_float16.pack(*values[offset : offset + DIMENSION]))
        quotients = to_float32.unpack(to_float32.pack(*(half / scale for half in halves)))
        rounded = to_float16.unpack(to_float16.pack(*quotients))
        output[row * DIMENSION : (row + 1) * DIMENSION] = struct.pack(
            f"<{DIMENSION}b", *(max(-127, min(127, round(value))) for value in rounded)
        )
    return bytes(output)


def safetensors(data: bytes, rows: int) -> bytes:
    header = json.dumps(
        {"embeddings": {"dtype": "I8", "shape": [rows, DIMENSION], "data_offsets": [0, len(data)]}},
        separators=(",", ":"),
    ).encode()
    header += b" " * (-len(header) % 8)
    return struct.pack("<Q", len(header)) + header + data


def tokenizer(path: pathlib.Path) -> bytes:
    content = json.loads(path.read_text(encoding="utf-8"))
    content["truncation"] = {
        "direction": "Right",
        "max_length": MAX_LENGTH,
        "strategy": "LongestFirst",
        "stride": 0,
    }
    return json.dumps(content, ensure_ascii=False, separators=(",", ":")).encode()


def config() -> bytes:
    return json.dumps(
        {"max_length": MAX_LENGTH, "normalize": True, "embedding_dtype": "int8"}, indent=4
    ).encode()


def build(source: pathlib.Path, output: pathlib.Path) -> None:
    values, rows, columns = read_embeddings(source / "0_StaticEmbedding/model.safetensors")
    output.mkdir(parents=True, exist_ok=True)
    (output / "config.json").write_bytes(config())
    (output / "model.safetensors").write_bytes(safetensors(quantize(values, rows, columns), rows))
    (output / "tokenizer.json").write_bytes(tokenizer(source / "0_StaticEmbedding/tokenizer.json"))
    for name in CARD_FILES:
        shutil.copyfile(CARD_DIRECTORY / name, output / name)
    for name, (size, checksum) in OUTPUT_FILES.items():
        verify(output / name, size, checksum)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--output", type=pathlib.Path, required=True, help="directory to write")
    parser.add_argument(
        "--source",
        type=pathlib.Path,
        help="local copy of the upstream repository at the pinned commit; downloaded otherwise",
    )
    args = parser.parse_args()
    with tempfile.TemporaryDirectory() as directory:
        build(fetch_upstream(args.source, pathlib.Path(directory)), args.output)
    for name in (*OUTPUT_FILES, *CARD_FILES):
        print(f"{sha256(args.output / name)}  {name}")


if __name__ == "__main__":
    main()
