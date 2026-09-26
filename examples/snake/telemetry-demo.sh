#!/bin/sh
# Launch Snake and one completed telemetry snapshot in a two-pane Zellij session.
set -eu

if ! command -v zellij >/dev/null 2>&1; then
    echo "telemetry demo requires zellij" >&2
    exit 127
fi

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$repo_root"

cargo build -p cargo-boomerang --features monitor
cargo run -p cargo-boomerang --features monitor -- boomerang --workspace examples/snake build --deployment snake

exec zellij --new-session-with-layout examples/snake/telemetry-demo.kdl
