"""Validate a built wheel against the size budget and the expected file layout."""

import argparse
import pathlib
import re
import sys
import tomllib
import zipfile

REQUIRED = {
    "univo/__init__.py",
    "univo/_transport.py",
    "univo/_univo.pyi",
    "univo/errors.py",
    "univo/models/__init__.py",
    "univo/models/__main__.py",
    "univo/py.typed",
}
NATIVE_MODULE = re.compile(r"^univo/_univo(\.abi3)?\.(so|pyd)$")
ALLOWED_METADATA = re.compile(r"^univo-[^/]+\.dist-info/")
MIB = 1024 * 1024


def check(wheel: pathlib.Path, max_mib: float, platform_tag: str | None) -> list[str]:
    errors = []
    if platform_tag is not None and platform_tag not in wheel.name:
        errors.append(f"{wheel.name}: platform tag does not contain {platform_tag!r}")
    size_mib = wheel.stat().st_size / MIB
    if size_mib > max_mib:
        errors.append(f"{wheel.name}: {size_mib:.2f} MiB exceeds {max_mib:.2f} MiB")

    with zipfile.ZipFile(wheel) as archive:
        names = set(archive.namelist())
    native = {name for name in names if NATIVE_MODULE.match(name)}
    if len(native) != 1:
        errors.append(f"{wheel.name}: expected one native module, found {sorted(native)}")
    for missing in sorted(REQUIRED - names):
        errors.append(f"{wheel.name}: missing {missing}")
    if not any(name.endswith(".dist-info/licenses/LICENSE") for name in names):
        errors.append(f"{wheel.name}: missing license file")
    unexpected = names - REQUIRED - native
    for name in sorted(unexpected):
        if not ALLOWED_METADATA.match(name):
            errors.append(f"{wheel.name}: unexpected file {name}")

    print(f"{wheel.name}: {size_mib:.3f} MiB, {len(names)} files")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("wheels", nargs="+", type=pathlib.Path)
    parser.add_argument(
        "--budgets", type=pathlib.Path, default=pathlib.Path("benchmarks/budgets.toml")
    )
    parser.add_argument(
        "--platform-tag",
        help="substring the wheel platform tag must contain, such as win_arm64",
    )
    arguments = parser.parse_args()

    with arguments.budgets.open("rb") as handle:
        max_mib = tomllib.load(handle)["wheel"]["max_compressed_mib"]
    errors = [
        error
        for wheel in arguments.wheels
        for error in check(wheel, max_mib, arguments.platform_tag)
    ]
    for error in errors:
        print(error, file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
