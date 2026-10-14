import pathlib
import re
from collections.abc import Callable

import pytest
from check_release import PLATFORMS, check_dist, check_tag, python_version, workspace_version

ROOT = pathlib.Path(__file__).resolve().parents[2]
WHEEL_TAGS = (
    "manylinux_2_17_x86_64.manylinux2014_x86_64",
    "manylinux_2_17_aarch64.manylinux2014_aarch64",
    "macosx_10_12_x86_64",
    "macosx_11_0_arm64",
    "win_amd64",
    "win_arm64",
)


def release(directory: pathlib.Path, version: str, tags: tuple[str, ...] = WHEEL_TAGS) -> None:
    (directory / f"univo-{version}.tar.gz").write_bytes(b"")
    for tag in tags:
        (directory / f"univo-{version}-cp311-abi3-{tag}.whl").write_bytes(b"")


@pytest.mark.parametrize(
    ("cargo", "python"),
    [
        ("0.1.0", "0.1.0"),
        ("1.2.3-alpha.1", "1.2.3a1"),
        ("1.2.3-beta.2", "1.2.3b2"),
        ("1.0.0-rc.10", "1.0.0rc10"),
    ],
)
def test_cargo_versions_map_to_pep_440(cargo: str, python: str) -> None:
    assert python_version(cargo) == python
    assert check_tag(f"v{cargo}", cargo) == python


@pytest.mark.parametrize(
    ("version", "message"),
    [
        ("0.0.0", "placeholder"),
        ("1.0", "is not X.Y.Z"),
        ("1.0.0-dev.1", "is not X.Y.Z"),
        ("1.0.0+local", "is not X.Y.Z"),
        ("1.0.0x", "is not X.Y.Z"),
    ],
)
def test_unpublishable_versions_are_refused(version: str, message: str) -> None:
    with pytest.raises(ValueError, match=message):
        python_version(version)


def test_the_tag_must_name_the_workspace_version() -> None:
    for tag in ("0.1.0", "v0.1.1", "release-0.1.0"):
        with pytest.raises(ValueError, match="does not name"):
            check_tag(tag, "0.1.0")


def test_internal_dependencies_must_repeat_the_workspace_version(tmp_path: pathlib.Path) -> None:
    cargo = tmp_path / "Cargo.toml"
    manifest = """
[workspace.package]
version = "0.1.0"
[workspace.dependencies]
univo-core = {{ path = "crates/univo-core", version = "{core}" }}
pyo3 = {{ version = "0.29.3" }}
"""
    cargo.write_text(manifest.format(core="0.1.0"), encoding="utf-8")
    assert workspace_version(cargo) == "0.1.0"
    cargo.write_text(manifest.format(core="0.0.0"), encoding="utf-8")
    with pytest.raises(ValueError, match=re.escape("univo-core requires version 0.0.0")):
        workspace_version(cargo)


def test_the_workspace_version_is_read_from_cargo() -> None:
    assert workspace_version() == re.search(
        r'^version = "([^"]+)"', (ROOT / "Cargo.toml").read_text(encoding="utf-8"), re.M
    ).group(1)  # type: ignore[union-attr]


def test_a_complete_release_is_accepted(tmp_path: pathlib.Path) -> None:
    release(tmp_path, "0.1.0rc1")
    assert len(check_dist("0.1.0rc1", tmp_path)) == 7


@pytest.mark.parametrize(
    ("change", "message"),
    [
        (
            lambda d: (d / "univo-0.1.0-cp311-abi3-win_arm64.whl").unlink(),
            "no wheel for windows-arm64",
        ),
        (
            lambda d: (d / "univo-0.1.0-cp311-abi3-macosx_14_0_arm64.whl").write_bytes(b""),
            "two wheels",
        ),
        (lambda d: (d / "univo-0.1.0.tar.gz").unlink(), "one source distribution"),
        (lambda d: (d / "univo-0.0.9.tar.gz").write_bytes(b""), "one source distribution"),
        (
            lambda d: (d / "univo-0.1.0-cp311-abi3-musllinux_1_2_x86_64.whl").write_bytes(b""),
            "unsupported",
        ),
        (
            lambda d: (d / "univo-0.1.0-cp311-abi3-macosx_11_0_universal2.whl").write_bytes(b""),
            "unsupported",
        ),
        (
            lambda d: (d / "univo-0.0.9-cp311-abi3-win_amd64.whl").write_bytes(b""),
            "is not univo 0.1.0",
        ),
        (lambda d: (d / "univo-0.1.0-cp314-cp314-win_amd64.whl").write_bytes(b""), "stable ABI"),
    ],
)
def test_an_incomplete_or_mixed_release_is_refused(
    tmp_path: pathlib.Path, change: Callable[[pathlib.Path], object], message: str
) -> None:
    release(tmp_path, "0.1.0")
    change(tmp_path)
    with pytest.raises(ValueError, match=message):
        check_dist("0.1.0", tmp_path)


def test_every_platform_built_by_ci_is_required() -> None:
    ci = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
    wheels = ci[ci.index("\n  wheels:") : ci.index("\n  wheel-test:")]
    assert set(re.findall(r"platform: ([\w-]+), runner", wheels)) == set(PLATFORMS)
