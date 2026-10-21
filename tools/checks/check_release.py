"""Release checks run by .github/workflows/release.yml before publishing.

python tools/checks/check_release.py version v0.1.0
    The tag names the workspace version of Cargo.toml; prints the
    version as PyPI spells it (PEP 440).
python tools/checks/check_release.py dist 0.1.0 dist/
    The directory holds exactly the files to upload: one source
    distribution and one wheel per supported platform, all of that version.
"""

import argparse
import pathlib
import re
import sys
import tomllib

ROOT = pathlib.Path(__file__).resolve().parents[2]
SEMVER = re.compile(r"(\d+)\.(\d+)\.(\d+)(?:-(alpha|beta|rc)\.(\d+))?")
PRERELEASE = {"alpha": "a", "beta": "b", "rc": "rc"}
# One wheel per advertised platform, built by ci.yml.
PLATFORMS = {
    "linux-x86_64": re.compile(r"manylinux_2_17_x86_64(\.manylinux2014_x86_64)?"),
    "linux-aarch64": re.compile(r"manylinux_2_17_aarch64(\.manylinux2014_aarch64)?"),
    "macos-x86_64": re.compile(r"macosx_\d+_\d+_x86_64"),
    "macos-arm64": re.compile(r"macosx_\d+_\d+_arm64"),
    "windows-x86_64": re.compile(r"win_amd64"),
    "windows-arm64": re.compile(r"win_arm64"),
}


def workspace_version(cargo: pathlib.Path = ROOT / "Cargo.toml") -> str:
    """The workspace version, which the internal `univo-*` dependencies must repeat."""
    with cargo.open("rb") as file:
        workspace = tomllib.load(file)["workspace"]
    version = str(workspace["package"]["version"])
    for name, dependency in workspace["dependencies"].items():
        if name.startswith("univo-") and dependency.get("version") != version:
            raise ValueError(f"{name} requires version {dependency.get('version')}, not {version}")
    return version


def python_version(version: str) -> str:
    """The PEP 440 spelling of a Cargo version, as maturin writes it."""
    match = SEMVER.fullmatch(version)
    if match is None:
        raise ValueError(f"version {version} is not X.Y.Z or X.Y.Z-(alpha|beta|rc).N")
    major, minor, patch, stage, number = match.groups()
    release = f"{major}.{minor}.{patch}"
    if release == "0.0.0":
        raise ValueError("version 0.0.0 is the unreleased placeholder; set the version first")
    return release if stage is None else f"{release}{PRERELEASE[stage]}{number}"


def check_tag(tag: str, version: str) -> str:
    if tag != f"v{version}":
        raise ValueError(
            f"tag {tag} does not name the workspace version {version}; expected v{version}"
        )
    return python_version(version)


def check_dist(version: str, directory: pathlib.Path) -> list[str]:
    names = sorted(path.name for path in directory.iterdir())
    expected_sdist = f"univo-{version}.tar.gz"
    wheels = [name for name in names if name.endswith(".whl")]
    others = [name for name in names if not name.endswith(".whl")]
    if others != [expected_sdist]:
        raise ValueError(f"expected one source distribution {expected_sdist}, found {others}")
    found: dict[str, str] = {}
    for name in wheels:
        parts = name[: -len(".whl")].split("-")
        if len(parts) != 5 or parts[:2] != ["univo", version] or parts[2:4] != ["cp311", "abi3"]:
            raise ValueError(f"wheel {name} is not univo {version} for the cp311 stable ABI")
        platform = next((key for key, tag in PLATFORMS.items() if tag.fullmatch(parts[4])), None)
        if platform is None:
            raise ValueError(f"wheel {name} is for an unsupported platform")
        if platform in found:
            raise ValueError(f"two wheels for {platform}: {found[platform]} and {name}")
        found[platform] = name
    missing = sorted(set(PLATFORMS) - set(found))
    if missing:
        raise ValueError(f"no wheel for {', '.join(missing)}")
    return names


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest="command", required=True)
    version = commands.add_parser("version")
    version.add_argument("tag")
    dist = commands.add_parser("dist")
    dist.add_argument("version")
    dist.add_argument("directory", type=pathlib.Path)
    args = parser.parse_args()
    try:
        if args.command == "version":
            print(check_tag(args.tag, workspace_version()))
        else:
            print("\n".join(check_dist(args.version, args.directory)))
    except ValueError as error:
        sys.exit(f"error: {error}")


if __name__ == "__main__":
    main()
