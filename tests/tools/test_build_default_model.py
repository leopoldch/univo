import array
import json
import pathlib
import re
import struct

import build_default_model as tool
import pytest
from build_default_model import DIMENSION, OUTPUT_FILES, quantize, safetensors, tokenizer, verify

ROOT = pathlib.Path(__file__).resolve().parents[2]


def matrix(rows: list[list[float]]) -> array.array[float]:
    return array.array("f", [value for row in rows for value in row])


def test_quantization_scales_by_every_column_and_keeps_the_leading_ones() -> None:
    columns = DIMENSION + 2
    first = [0.0] * columns
    first[:4] = [0.5, -0.25, 1.0, 0.0]
    second = [0.0] * columns
    second[-1] = 2.0  # dropped column, but it sets the scale
    data = quantize(matrix([first, second]), 2, columns)
    assert len(data) == 2 * DIMENSION
    values = struct.unpack(f"<{2 * DIMENSION}b", data)
    # Scale 2 / 127: 0.5 -> 31.75 -> 32, -0.25 -> -15.875 -> -16, 1.0 -> 63.5 -> 64.
    assert values[:4] == (32, -16, 64, 0)
    assert set(values[DIMENSION:]) == {0}


def test_quantization_rounds_half_to_even_at_the_extremes() -> None:
    row = [0.0] * DIMENSION
    row[:4] = [127.0, -127.0, 0.5, 1.5]
    values = struct.unpack(f"<{DIMENSION}b", quantize(matrix([row]), 1, DIMENSION))
    assert values[:4] == (127, -127, 0, 2)


def test_safetensors_header_is_aligned_and_describes_int8_rows() -> None:
    data = bytes(range(2 * DIMENSION))
    encoded = safetensors(data, 2)
    (length,) = struct.unpack_from("<Q", encoded)
    assert length % 8 == 0
    header = json.loads(encoded[8 : 8 + length])
    assert header == {
        "embeddings": {"dtype": "I8", "shape": [2, DIMENSION], "data_offsets": [0, len(data)]}
    }
    assert encoded[8 + length :] == data


def test_tokenizer_enables_truncation_and_keeps_the_rest(tmp_path: pathlib.Path) -> None:
    source = tmp_path / "tokenizer.json"
    source.write_text(
        json.dumps({"version": "1.0", "truncation": None, "model": {"vocab": {"é": 0}}}),
        encoding="utf-8",
    )
    content = tokenizer(source)
    assert json.loads(content) == {
        "version": "1.0",
        "truncation": {
            "direction": "Right",
            "max_length": 512,
            "strategy": "LongestFirst",
            "stride": 0,
        },
        "model": {"vocab": {"é": 0}},
    }
    assert "é".encode() in content


def test_verify_rejects_a_changed_file(tmp_path: pathlib.Path) -> None:
    path = tmp_path / "file"
    path.write_bytes(b"content")
    verify(path, 7, tool.sha256(path))
    with pytest.raises(SystemExit, match="does not match"):
        verify(path, 7, "0" * 64)
    with pytest.raises(SystemExit, match="does not match"):
        verify(path, 8, tool.sha256(path))


def test_the_release_pins_the_checksums_the_tool_produces() -> None:
    registry = (ROOT / "crates/univo-embed/src/registry.rs").read_text(encoding="utf-8")
    pinned = {
        name: (int(size.replace("_", "")), checksum)
        for name, size, checksum in re.findall(
            r'file\(\s*"([^"]+)",\s*([\d_]+),\s*"([0-9a-f]{64})",?\s*\)', registry
        )
    }
    assert pinned == OUTPUT_FILES


def test_the_model_card_names_the_license_and_every_checksum() -> None:
    card = (ROOT / "tools/default_model/model-card.md").read_text(encoding="utf-8")
    assert "license: apache-2.0" in card
    assert tool.UPSTREAM in card
    assert tool.UPSTREAM_REVISION in card
    for name, (size, checksum) in OUTPUT_FILES.items():
        assert f"| `{name}` | {size:,} B | `{checksum}` |" in card
    license_text = (ROOT / "tools/default_model/LICENSE").read_text(encoding="utf-8")
    assert "Apache License" in license_text
    assert "Version 2.0, January 2004" in license_text
