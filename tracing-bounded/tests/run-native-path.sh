#!/bin/sh
# A nested Cargo.toml would cause Cargo to omit the fixture from the archive.
# Materialize the packaged template in a private workspace; never edit sources.
set -eu

audit_mode=${1:-all}
case "$audit_mode" in
    all|miri|inventory) ;;
    *) echo "usage: sh tests/run-native-path.sh [all|miri|inventory]" >&2; exit 2 ;;
esac
if [ "$#" -gt 1 ]; then
    echo "expected at most one audit mode" >&2
    exit 2
fi

audit_tests=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
audit_fixture="$audit_tests/fixtures/native-path"
audit_dir=$(mktemp -d "${TMPDIR:-/tmp}/tracing-bounded-audit.XXXXXXXX")
# This path is the fresh mktemp directory above, never a caller-supplied target.
trap 'rm -rf -- "$audit_dir"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
cp "$audit_fixture/Cargo.toml.in" "$audit_dir/Cargo.toml"
cp "$audit_fixture/Cargo.lock" "$audit_dir/Cargo.lock"
cp -R "$audit_fixture/src" "$audit_dir/src"
# Prevent an inherited target directory from selecting stale fixture artifacts.
export CARGO_TARGET_DIR="$audit_dir/target"

case "$audit_mode" in
    all)
        for audit_case in allocator-control first-use repeated different-sites same-site filtered unsupported; do
            cargo run --offline --locked --manifest-path "$audit_dir/Cargo.toml" -- "$audit_case"
            cargo run --offline --locked --release --manifest-path "$audit_dir/Cargo.toml" -- "$audit_case"
        done
        cargo tree --offline --locked --manifest-path "$audit_dir/Cargo.toml" -e features
        cargo clippy --offline --locked --manifest-path "$audit_dir/Cargo.toml" --all-targets -- -D warnings
        cargo fmt --manifest-path "$audit_dir/Cargo.toml" --check
        ;;
    miri)
        for audit_case in first-use unsupported; do
            cargo +nightly miri run --offline --locked --manifest-path "$audit_dir/Cargo.toml" -- "$audit_case"
        done
        ;;
    inventory)
        cargo build --offline --locked --manifest-path "$audit_dir/Cargo.toml"
        cargo build --offline --locked --release --manifest-path "$audit_dir/Cargo.toml"
        for audit_profile in debug release; do
            echo "$audit_profile callsite symbols (META entries are metadata):"
            nm "$audit_dir/target/$audit_profile/tracing-bounded-native-path-audit" > "$audit_dir/symbols-$audit_profile.txt"
            grep '__CALLSITE' "$audit_dir/symbols-$audit_profile.txt"
        done
        ;;
esac
