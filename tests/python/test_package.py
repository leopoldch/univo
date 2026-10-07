import os
import pathlib
import sysconfig

import univo


def test_version_is_exposed_by_native_extension() -> None:
    from univo import _univo

    assert isinstance(univo.__version__, str)
    assert univo.__version__ == _univo.__version__


def test_package_is_typed() -> None:
    assert (pathlib.Path(univo.__file__).parent / "py.typed").is_file()


def test_installed_package_is_used_when_required() -> None:
    if os.environ.get("UNIVO_EXPECT_INSTALLED") != "1":
        return
    site_packages = pathlib.Path(sysconfig.get_paths()["purelib"]).resolve()
    assert pathlib.Path(univo.__file__).resolve().is_relative_to(site_packages)
