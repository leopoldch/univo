"""MkDocs hooks.

- Links to files that are not pages of the site point to GitHub: contributor
  pages (docs/design, docs/development) and repository files outside docs/.
- Our stylesheet and script are linked with a hash of their content, so a
  browser never keeps an old copy after an update (GitHub Pages caches them
  for 10 minutes).
"""

import hashlib
import posixpath
import re
from pathlib import Path
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


ASSETS = ("assets/site.css", "assets/charts.js")


def on_post_page(output: str, config: Any, **_: Any) -> str:
    for asset in ASSETS:
        digest = hashlib.sha256((Path(config["docs_dir"]) / asset).read_bytes()).hexdigest()[:10]
        output = re.sub(rf'((?:\.\./)*{re.escape(asset)})"', rf'\1?v={digest}"', output)
    return output
