#!/bin/sh

set -eu

export CARGO_HOME="/cargo"
export RUSTUP_HOME="/rustup"

# Source cargo environment
. "${CARGO_HOME}/env"

cargo build --release --target-dir "${TARGET_DIR}" --bin "${BIN_TARGET}"
