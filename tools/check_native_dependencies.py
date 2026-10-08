"""Fail when a network crate is linked into a distributed Rust artifact.

Python audit hooks cannot see sockets opened by native code, so the absence
of network access at import and Finder construction is also enforced
statically: no HTTP, TLS or async-runtime crate may be reachable through
normal (linked) dependencies of the listed packages. Build scripts and
procedural macros run at compile time only and are not traversed.

Model downloads (ADR 0008) are performed by each binding with its
language's standard library; Rust crates only plan and verify them, so no
listed package may link a network crate either.
"""

import argparse
import json
import subprocess
import sys
from typing import Any

NETWORK_CRATES = frozenset(
    {
        "async-std",
        "attohttpc",
        "curl",
        "curl-sys",
        "h2",
        "http",
        "hyper",
        "isahc",
        "mio",
        "native-tls",
        "openssl",
        "openssl-sys",
        "reqwest",
        "rustls",
        "socket2",
        "surf",
        "tokio",
        "ureq",
    }
)


def linked_packages(metadata: dict[str, Any], root_name: str) -> dict[str, list[str]]:
    """Map each linked package name to one dependency path from the root."""
    packages = {package["id"]: package for package in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    roots = [
        package_id
        for package_id in metadata["workspace_members"]
        if packages[package_id]["name"] == root_name
    ]
    if len(roots) != 1:
        raise SystemExit(f"workspace package {root_name!r} not found")

    paths = {roots[0]: [root_name]}
    pending = [roots[0]]
    while pending:
        current = pending.pop()
        for dependency in nodes[current]["deps"]:
            if not any(kind["kind"] is None for kind in dependency["dep_kinds"]):
                continue
            target = dependency["pkg"]
            if target in paths or _is_proc_macro(packages[target]):
                continue
            paths[target] = [*paths[current], packages[target]["name"]]
            pending.append(target)
    return {packages[package_id]["name"]: path for package_id, path in paths.items()}


def _is_proc_macro(package: dict[str, Any]) -> bool:
    return any("proc-macro" in target["kind"] for target in package["targets"])


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("packages", nargs="*", default=["univo-python"])
    arguments = parser.parse_args()

    output = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked"],
        capture_output=True,
        check=True,
        text=True,
    ).stdout
    metadata = json.loads(output)

    failed = False
    for package in arguments.packages:
        linked = linked_packages(metadata, package)
        network = sorted(NETWORK_CRATES & linked.keys())
        for name in network:
            print(f"{package}: links {name} via {' -> '.join(linked[name])}", file=sys.stderr)
        if network:
            failed = True
        else:
            print(f"{package}: {len(linked)} linked packages, no network crate")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
