"""Fail when a Markdown file links to a repository path that does not exist.

Links inside fenced code blocks and inline code spans are examples, not links,
and are ignored.
"""

import os
import pathlib
import re
import sys
from collections.abc import Iterator

LINK = re.compile(r"\]\(([^)#\s]+)(?:#[^)]*)?\)")
FENCE = re.compile(r"^ {0,3}(`{3,}|~{3,})")
INLINE_CODE = re.compile(r"(`+).+?\1")
SKIPPED_DIRS = {".git", "target", ".venv", "node_modules", "dist"}
# Copies iCloud Drive makes of files changed by a checkout, ignored by git too.
ICLOUD_COPY = re.compile(r" [0-9]\.md$")


def markdown_files(root: pathlib.Path) -> Iterator[pathlib.Path]:
    for directory, subdirectories, files in os.walk(root):
        subdirectories[:] = sorted(name for name in subdirectories if name not in SKIPPED_DIRS)
        for name in sorted(files):
            if name.endswith(".md") and not ICLOUD_COPY.search(name):
                yield pathlib.Path(directory, name)


def prose_lines(text: str) -> Iterator[str]:
    """Yield lines outside fenced code blocks, with inline code removed."""
    fence: str | None = None
    for line in text.splitlines():
        marker = FENCE.match(line)
        if fence is None and marker:
            fence = marker.group(1)
        elif fence is not None:
            if marker and marker.group(1)[0] == fence[0] and len(marker.group(1)) >= len(fence):
                fence = None
        else:
            yield INLINE_CODE.sub("", line)


def broken_links(root: pathlib.Path) -> list[str]:
    errors = []
    for document in markdown_files(root):
        for line in prose_lines(document.read_text(encoding="utf-8")):
            for match in LINK.finditer(line):
                target = match.group(1)
                if target.startswith(("http://", "https://", "mailto:")):
                    continue
                if not (document.parent / target).exists():
                    errors.append(f"{document.relative_to(root)}: broken link {target}")
    return errors


def main() -> int:
    root = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else ".").resolve()
    errors = broken_links(root)
    for error in errors:
        print(error, file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
