import pathlib
import zipfile

from check_wheel import check

VALID_FILES = (
    "univo/__init__.py",
    "univo/_api.py",
    "univo/py.typed",
    "univo/_transport.py",
    "univo/_univo.pyi",
    "univo/errors.py",
    "univo/models/__init__.py",
    "univo/models/__main__.py",
    "univo/_univo.abi3.so",
    "univo-0.0.0.dist-info/METADATA",
    "univo-0.0.0.dist-info/licenses/LICENSE",
)


def make_wheel(directory: pathlib.Path, name: str, files: tuple[str, ...]) -> pathlib.Path:
    path = directory / name
    with zipfile.ZipFile(path, "w") as archive:
        for file in files:
            archive.writestr(file, "x")
    return path


def test_accepts_expected_layout(tmp_path: pathlib.Path) -> None:
    wheel = make_wheel(tmp_path, "univo-0.0.0-cp311-abi3-win_arm64.whl", VALID_FILES)

    assert check(wheel, max_mib=1.0, platform_tag="win_arm64") == []


def test_rejects_wrong_platform_tag(tmp_path: pathlib.Path) -> None:
    wheel = make_wheel(tmp_path, "univo-0.0.0-cp311-abi3-win_amd64.whl", VALID_FILES)

    assert any("platform tag" in error for error in check(wheel, 1.0, "win_arm64"))


def test_rejects_unexpected_and_missing_files(tmp_path: pathlib.Path) -> None:
    files = (*VALID_FILES[1:], "univo/secret.rs")
    wheel = make_wheel(tmp_path, "univo-0.0.0-cp311-abi3-win_amd64.whl", files)

    errors = check(wheel, 1.0, None)

    assert any("missing univo/__init__.py" in error for error in errors)
    assert any("unexpected file univo/secret.rs" in error for error in errors)


def test_rejects_oversized_wheel(tmp_path: pathlib.Path) -> None:
    wheel = make_wheel(tmp_path, "univo-0.0.0-cp311-abi3-win_amd64.whl", VALID_FILES)

    assert any("exceeds" in error for error in check(wheel, max_mib=1e-6, platform_tag=None))
