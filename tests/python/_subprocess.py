import json
import subprocess
import sys
from typing import Any


def run_isolated(code: str) -> Any:
    """Run code in a fresh isolated interpreter and decode the JSON it prints."""
    completed = subprocess.run(
        [sys.executable, "-I", "-c", code],
        capture_output=True,
        check=True,
        text=True,
    )
    return json.loads(completed.stdout)
