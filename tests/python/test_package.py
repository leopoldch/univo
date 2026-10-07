import os
import pathlib
import platform
import sysconfig

import univo

from ._subprocess import run_isolated

MACHINE_ALIASES = {
    "x86_64": {"x86_64", "amd64"},
    "aarch64": {"aarch64", "arm64"},
}

MODULES_ADDED_BY_IMPORT = """
import json, sys
before = set(sys.modules)
import univo
print(json.dumps(sorted(set(sys.modules) - before)))
"""


def test_version_is_exposed_by_native_extension() -> None:
    from univo import _univo

    assert isinstance(univo.__version__, str)
    assert univo.__version__ == _univo.__version__


def test_package_is_typed() -> None:
    assert (pathlib.Path(univo.__file__).parent / "py.typed").is_file()


def test_import_loads_only_univo_modules() -> None:
    added = run_isolated(MODULES_ADDED_BY_IMPORT)

    assert added, "import univo did not load any module"
    assert [name for name in added if name.split(".")[0] != "univo"] == []


def test_installed_package_is_used_when_required() -> None:
    if os.environ.get("UNIVO_EXPECT_INSTALLED") != "1":
        return
    site_packages = pathlib.Path(sysconfig.get_paths()["purelib"]).resolve()
    assert pathlib.Path(univo.__file__).resolve().is_relative_to(site_packages)


def test_interpreter_runs_on_expected_architecture() -> None:
    """Guards CI against emulated runs (Rosetta, x64 emulation on Windows ARM)."""
    expected = os.environ.get("UNIVO_EXPECT_MACHINE")
    if expected is None:
        return
    assert platform.machine().lower() in MACHINE_ALIASES[expected]
