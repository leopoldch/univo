import pathlib
import re

import pytest

ROOT = pathlib.Path(__file__).resolve().parents[2]
WORKFLOWS = sorted((ROOT / ".github/workflows").glob("*.yml"))


def jobs(workflow: str) -> dict[str, str]:
    """Each job's block of lines, keyed by job name."""
    body = workflow[workflow.index("\njobs:\n") :]
    starts = list(re.finditer(r"^  ([\w-]+):$", body, re.M))
    return {
        match.group(1): body[match.end() : next_match.start() if next_match else len(body)]
        for match, next_match in zip(starts, [*starts[1:], None], strict=True)
    }


@pytest.mark.parametrize("path", WORKFLOWS, ids=lambda path: path.name)
def test_jobs_that_check_out_the_private_repository_can_read_it(path: pathlib.Path) -> None:
    # A job-level `permissions` block sets every scope it omits to none.
    for name, block in jobs(path.read_text(encoding="utf-8")).items():
        permissions = re.search(r"^    permissions:\n((?:      .*\n)+)", block, re.M)
        if permissions and "actions/checkout" in block:
            assert re.search(r"^      contents: (read|write)$", permissions.group(1), re.M), name
