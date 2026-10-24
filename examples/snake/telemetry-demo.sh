#!/bin/sh
# Launch Snake above its live telemetry dashboard in a Zellij session.
set -eu

if ! command -v zellij >/dev/null 2>&1; then
    echo "telemetry demo requires zellij" >&2
    exit 127
fi

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$repo_root"

cargo build -p cargo-boomerang --features monitor
cargo build -p snake-app --features telemetry

BOOMERANG_TELEMETRY_RUN_ID=$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')
export BOOMERANG_TELEMETRY_RUN_ID

exec zellij --new-session-with-layout examples/snake/telemetry-demo.kdl
