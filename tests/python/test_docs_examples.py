"""The Python examples of the documentation run, and print what the page shows.

A ```python block followed by a ```text block must print exactly that text.
Blocks that load a semantic model run only when UNIVO_TEST_MODEL_DIR is set.
"""

import contextlib
import io
import os
import re
import textwrap
from pathlib import Path

import pytest

DOCS = Path(__file__).parents[2] / "docs"
PAGES = ["index.md", "guide.md"]
# Blocks may be indented, inside tabs; the output block has the same indent.
BLOCK = re.compile(
    r"^( *)```python\n(.*?)^\1```\n(?:\s*^\1```text\n(.*?)^\1```)?", re.DOTALL | re.MULTILINE
)


def examples() -> list[tuple[str, str, str | None]]:
    found = []
    for page in PAGES:
        for number, match in enumerate(BLOCK.finditer((DOCS / page).read_text()), 1):
            indent, code, expected = match.groups()
            if indent:
                code = textwrap.dedent(code)
                expected = None if expected is None else textwrap.dedent(expected)
            found.append((f"{page}:{number}", code, expected))
    return found


@pytest.mark.parametrize(("name", "code", "expected"), examples(), ids=lambda value: value)
def test_example(
    name: str, code: str, expected: str | None, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    semantic = 'mode="semantic"' in code
    if semantic and not os.environ.get("UNIVO_TEST_MODEL_DIR"):
        pytest.skip("needs the default model (UNIVO_TEST_MODEL_DIR)")
    monkeypatch.chdir(tmp_path)
    output = io.StringIO()
    with contextlib.redirect_stdout(output):
        exec(compile(code, name, "exec"), {})
    if expected is not None:
        assert output.getvalue() == expected
