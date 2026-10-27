"""MkDocs hook: links to files that are not pages of the site point to GitHub.

Contributor pages (docs/design, docs/development) and repository files
outside docs/ are not part of the site; GitHub renders them.
"""

import posixpath
import re
from typing import Any

REPOSITORY = "https://github.com/leopoldch/univo/blob/main/"
LINK = re.compile(r"\]\(((?!https?:|mailto:|#)[^)#\s]+)(#[^)\s]*)?\)")


def on_page_markdown(markdown: str, page: Any, files: Any, **_: Any) -> str:
    folder = posixpath.dirname(page.file.src_uri)

    def rewrite(match: re.Match[str]) -> str:
        target = posixpath.normpath(posixpath.join(folder, match.group(1)))
        file = files.get_file_from_path(target)
        if file is not None and not file.inclusion.is_excluded():
            return match.group(0)
        return f"]({REPOSITORY}{posixpath.join('docs', target)}{match.group(2) or ''})"

    return LINK.sub(rewrite, markdown)
