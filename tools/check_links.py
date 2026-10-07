"""Fail when a Markdown file links to a repository path that does not exist."""

import pathlib
import re
import sys

LINK = re.compile(r"\]\(([^)#\s]+)(?:#[^)]*)?\)")
SKIPPED_DIRS = {".git", "target", ".venv", "node_modules"}


def broken_links(root: pathlib.Path) -> list[str]:
    errors = []
    for document in sorted(root.rglob("*.md")):
        if SKIPPED_DIRS.intersection(document.relative_to(root).parts):
            continue
        for match in LINK.finditer(document.read_text(encoding="utf-8")):
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
