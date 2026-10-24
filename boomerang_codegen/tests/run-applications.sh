#!/usr/bin/env bash
# Build application fixtures before the process tests. Cargo never runs inside those tests.
set -euo pipefail
cd "$(dirname "$0")/../.."

case "${1:-}" in
  "") ;;
  --coverage)
    export CARGO_TARGET_DIR="${CARGO_LLVM_COV_TARGET_DIR:-${CARGO_TARGET_DIR:-target}/llvm-cov-target}"
    coverage_env=$(cargo llvm-cov show-env --sh)
    eval "$coverage_env"
    ;;
  *) echo "usage: bash boomerang_codegen/tests/run-applications.sh [--coverage]" >&2; exit 2 ;;
esac

# Keep instrumentation from cargo-llvm-cov; remove inherited application policy.
for name in $(compgen -e); do
  case "$name" in BOOMERANG_*|NATIVE_VEHICLE_*) unset "$name" ;; esac
done
unset CARGO_BUILD_TARGET RUSTC_WORKSPACE_WRAPPER
if [[ -z "${CARGO_LLVM_COV:-}" ]]; then
  unset RUSTFLAGS CARGO_ENCODED_RUSTFLAGS RUSTC_WRAPPER
fi

mkdir -p "${CARGO_TARGET_DIR:-target}"
export CARGO_TARGET_DIR="$(cd "${CARGO_TARGET_DIR:-target}" && pwd)"
fixture_target="$CARGO_TARGET_DIR"
if [[ -n "${CARGO_LLVM_COV:-}" ]]; then
  fixture_target="$CARGO_TARGET_DIR/native-fixtures"
fi
executables="$CARGO_TARGET_DIR/native-fixtures/executables"
mkdir -p "$executables"
export BOOMERANG_TEST_BIN_DIR="$executables"

suffix=""
case "$(rustc -vV)" in *-windows-*) suffix=".exe" ;; esac
native=boomerang_codegen/tests/fixtures/native/Cargo.toml
federated=boomerang_codegen/tests/fixtures/federated/Cargo.toml
native_target="$fixture_target/codegen-native-consumer"
federated_target="$fixture_target/cargo-native-federation-slice4"

CARGO_TARGET_DIR="$native_target" cargo build --locked --offline --manifest-path "$native"
cp "$native_target/debug/codegen-native-consumer$suffix" "$executables/consumer$suffix"
CARGO_TARGET_DIR="$native_target" cargo build --locked --offline --manifest-path "$native" --features source-input
cp "$native_target/debug/codegen-native-consumer$suffix" "$executables/consumer-source$suffix"

CARGO_TARGET_DIR="$federated_target" cargo build --locked --offline --manifest-path "$federated" \
  -p native-vehicle-controller -p native-vehicle-sensor -p native-vehicle-rti
for role in controller sensor rti; do
  cp "$federated_target/debug/native-vehicle-$role$suffix" "$executables/$role$suffix"
done
# Generated-table and heterogeneous-target assertions run as ordinary crate tests.
CARGO_TARGET_DIR="$federated_target" cargo test --locked --offline --manifest-path "$federated" \
  -p native-vehicle-deployment -p native-vehicle-rti
CARGO_TARGET_DIR="$federated_target" cargo build --locked --offline --manifest-path "$federated" \
  -p native-vehicle-sensor --features runtime-failure
cp "$federated_target/debug/native-vehicle-sensor$suffix" "$executables/sensor-failure$suffix"
NATIVE_VEHICLE_DELAY_NS=2000000 CARGO_TARGET_DIR="$federated_target" cargo build --locked --offline \
  --manifest-path "$federated" -p native-vehicle-sensor
cp "$federated_target/debug/native-vehicle-sensor$suffix" "$executables/sensor-mixed$suffix"

# Dependency isolation is a Cargo contract, independent of process execution.
for role in controller sensor rti; do
  CARGO_TARGET_DIR="$federated_target" cargo tree --locked --offline --color never --manifest-path "$federated" \
    -p "native-vehicle-$role" --edges normal,no-proc-macro --prefix none > "$executables/$role.dependencies"
  if grep -Eq '^(boomerang_builder|boomerang_codegen|cargo-boomerang) ' "$executables/$role.dependencies"; then
    echo "$role links a host compiler dependency" >&2; exit 1
  fi
  for component in vehicle-control sensor-host; do
    owner=controller
    if [[ "$component" == sensor-host ]]; then owner=sensor; fi
    if grep -q "^$component " "$executables/$role.dependencies"; then
      if [[ "$role" != "$owner" ]]; then echo "$role links remote payload $component" >&2; exit 1; fi
    elif [[ "$role" == "$owner" ]]; then
      echo "$role lacks local payload $component" >&2; exit 1
    fi
  done
done
cargo test -p boomerang_codegen --all-features --locked --offline --test applications -- --ignored
