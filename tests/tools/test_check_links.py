import pathlib

from check_links import broken_links


def write(path: pathlib.Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def test_reports_missing_relative_target(tmp_path: pathlib.Path) -> None:
    write(tmp_path / "doc.md", "[ok](other.md) [missing](missing.md) [web](https://example.com)")
    write(tmp_path / "other.md", "")

    assert broken_links(tmp_path) == ["doc.md: broken link missing.md"]


def test_ignores_links_in_fenced_blocks_and_inline_code(tmp_path: pathlib.Path) -> None:
    write(
        tmp_path / "doc.md",
        "```markdown\n[a](fenced.md)\n```\n~~~~\n[b](tilde.md)\n```\n~~~~\nUse `[c](inline.md)`.\n",
    )

    assert broken_links(tmp_path) == []


def test_skips_tool_directories(tmp_path: pathlib.Path) -> None:
    for directory in (".venv", "target", ".git", "node_modules"):
        write(tmp_path / directory / "nested" / "doc.md", "[x](missing.md)")

    assert broken_links(tmp_path) == []
