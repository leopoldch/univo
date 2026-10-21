#!/usr/bin/env bash
# Builds the manylinux wheel of one architecture in Docker and tests it as CI's
# wheel-test does: installed without compiling, in Python 3.11 and 3.14 slim
# images without Rust, as a non-root user, from outside the source tree.
#
#   tools/checks/linux_wheels.sh aarch64|x86_64 [model-dir]
#
# x86_64 runs under emulation on Apple silicon: slower, and timing budgets
# may need a rerun. The source is the committed HEAD, not the working tree.
set -euo pipefail

arch=${1:?usage: linux_wheels.sh aarch64|x86_64 [model-dir]}
case $arch in
  aarch64) platform=linux/arm64 ;;
  x86_64) platform=linux/amd64 ;;
  *) echo "unknown architecture: $arch" >&2; exit 2 ;;
esac
model=${2:-}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
git archive HEAD | tar -x -C "$work"
mkdir -p "$work/dist"
if [ -n "$model" ]; then cp -R "$model" "$work/model"; fi
# mktemp creates a 0700 directory: let the test user, whatever the host UID,
# read the sources, the model and the wheel.
chmod -R a+rX "$work"

# The build runs as root: its target directory stays in the container so the
# cleanup never meets root-owned files.
docker run --rm --platform "$platform" -v "$work:/io" -w /io -e CARGO_TARGET_DIR=/tmp/target \
  -v "univo-cargo-$arch:/root/.cargo/registry" \
  ghcr.io/pyo3/maturin:latest build --release --locked --out /io/dist
python3 tools/checks/check_wheel.py --platform-tag "manylinux_2_17_$arch" "$work"/dist/*.whl

model_args=()
if [ -n "$model" ]; then
  model_args=(-e UNIVO_TEST_MODEL_DIR=/src/model -e UNIVO_TEST_HUB=1)
fi
for python in 3.11 3.14; do
  echo "== Python $python, $platform"
  docker run --rm --platform "$platform" -u 1000:1000 -e HOME=/tmp \
    -e UNIVO_EXPECT_INSTALLED=1 -e UNIVO_EXPECT_MACHINE="$arch" -e UNIVO_PERF_SLACK=2.0 \
    "${model_args[@]}" -v "$work:/src:ro" "python:$python-slim" sh -euc '
      if command -v cargo || command -v rustc; then echo "Rust is installed" >&2; exit 1; fi
      python -m venv /tmp/venv
      /tmp/venv/bin/pip install -q --no-index --only-binary :all: /src/dist/*.whl
      /tmp/venv/bin/pip install -q pytest
      cd /tmp
      /tmp/venv/bin/python -m pytest -q -rs -p no:cacheprovider \
        -c /src/pyproject.toml --rootdir /src /src/tests/python'
done
