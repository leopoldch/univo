from typing import Any

from check_native_dependencies import NETWORK_CRATES, linked_packages


def package(name: str, kind: str = "lib") -> dict[str, Any]:
    return {"id": name, "name": name, "targets": [{"kind": [kind]}]}


def edge(target: str, kind: str | None = None) -> dict[str, Any]:
    return {"pkg": target, "dep_kinds": [{"kind": kind}]}


METADATA = {
    "workspace_members": ["root"],
    "packages": [
        package("root"),
        package("core"),
        package("macros", kind="proc-macro"),
        package("builder"),
        package("tester"),
        package("tokio"),
        package("ureq"),
        package("reqwest"),
    ],
    "resolve": {
        "nodes": [
            {
                "id": "root",
                "deps": [
                    edge("core"),
                    edge("macros"),
                    edge("builder", "build"),
                    edge("tester", "dev"),
                ],
            },
            {"id": "core", "deps": [edge("tokio")]},
            {"id": "macros", "deps": [edge("ureq")]},
            {"id": "builder", "deps": [edge("reqwest")]},
            {"id": "tester", "deps": [edge("reqwest")]},
            {"id": "tokio", "deps": []},
            {"id": "ureq", "deps": []},
            {"id": "reqwest", "deps": []},
        ]
    },
}


def test_follows_normal_dependencies_transitively() -> None:
    linked = linked_packages(METADATA, "root")

    assert linked["tokio"] == ["root", "core", "tokio"]
    assert NETWORK_CRATES & linked.keys() == {"tokio"}


def test_skips_build_dev_and_proc_macro_dependencies() -> None:
    linked = linked_packages(METADATA, "root")

    assert {"builder", "tester", "macros", "ureq", "reqwest"}.isdisjoint(linked)
