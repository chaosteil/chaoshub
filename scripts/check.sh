#!/usr/bin/env bash
# Runs the required checks of AGENTS.md. CI runs the same steps from this
# script, so a pass here gives a pass in CI.
#
# Usage: scripts/check.sh [step...]
#
# Steps, in the order of CI:
#   build   cargo build --locked
#   test    cargo nextest run --locked
#   clippy  cargo clippy --locked --all-targets -- -D warnings
#   fmt     cargo fmt --check
#
# Without a step, the script runs all four steps. Before the first step,
# the script makes sure that rustc and clippy have the version of
# rust-toolchain.toml. With rustup, the version is correct. With Nix, use
# `nix develop`.
#
# Note: the Linux CI job runs the tests without jj on the PATH, and the
# macOS CI job runs them with jj. A local run uses the jj of your PATH.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

# The channel line of rust-toolchain.toml, for example 1.99.1.
want=$(sed -n 's/^channel = "\(.*\)"$/\1/p' rust-toolchain.toml)

toolchain() {
  local rustc clippy
  rustc=$(rustc -V)
  clippy=$(cargo clippy -V)
  echo "$rustc"
  echo "$clippy"
  cargo nextest --version | head -n 1
  # Rust 1.99.1 has clippy 0.1.99: the clippy version has no patch number.
  local minor=${want#1.}
  minor=${minor%%.*}
  if [[ $rustc != "rustc $want "* || ${clippy%% (*} != "clippy 0.1.$minor" ]]; then
    echo "error: rust-toolchain.toml has Rust $want, but this is $rustc and $clippy." >&2
    echo "Use rustup, or run the checks in \`nix develop\`." >&2
    exit 1
  fi
}

step() {
  echo "==> $*"
  "$@"
}

run() {
  case $1 in
  build) step cargo build --locked ;;
  test) step cargo nextest run --locked ;;
  clippy) step cargo clippy --locked --all-targets -- -D warnings ;;
  fmt) step cargo fmt --check ;;
  esac
}

if [[ $# -eq 0 ]]; then
  set -- build test clippy fmt
fi
for name in "$@"; do
  case $name in
  build | test | clippy | fmt) ;;
  *)
    echo "error: unknown step: $name (build, test, clippy or fmt)" >&2
    exit 2
    ;;
  esac
done
toolchain
for name in "$@"; do
  run "$name"
done
echo "==> all checks passed: $*"
