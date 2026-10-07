"""Checks that CI ran the wheel in a process of the expected architecture.

- Windows: the interpreter's build platform. `platform.machine()` may report
  the host CPU for an emulated x64 process on an ARM64 machine.
- macOS: `uname` of the process, which Rosetta reports as x86_64, plus the
  kernel's translation flag.
- Linux: `uname` of the process. User-mode emulation (qemu-user) reports the
  emulated architecture and is not detected; CI relies on native runner labels.
"""

import ctypes
import os
import platform
import sys
import sysconfig

import pytest

ALIASES = {
    "x86_64": {"x86_64", "amd64"},
    "aarch64": {"aarch64", "arm64"},
}


def process_machine() -> str:
    if sys.platform == "win32":
        return sysconfig.get_platform().removeprefix("win-").lower()
    return platform.machine().lower()


def translated_by_rosetta() -> bool:
    value = ctypes.c_int(0)
    size = ctypes.c_size_t(ctypes.sizeof(value))
    status = ctypes.CDLL(None).sysctlbyname(
        b"sysctl.proc_translated", ctypes.byref(value), ctypes.byref(size), None, 0
    )
    return status == 0 and value.value == 1


@pytest.fixture
def expected_machine() -> str:
    expected = os.environ.get("UNIVO_EXPECT_MACHINE")
    if expected is None:
        pytest.skip("UNIVO_EXPECT_MACHINE is not set")
    return expected


def test_process_architecture_matches_expectation(expected_machine: str) -> None:
    assert process_machine() in ALIASES[expected_machine]


@pytest.mark.skipif(sys.platform != "darwin", reason="Rosetta exists only on macOS")
def test_process_is_not_translated_by_rosetta(expected_machine: str) -> None:
    assert not translated_by_rosetta()
