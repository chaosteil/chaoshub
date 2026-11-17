# AGENTS.md

These are the base rules of the chaoshub repo. Every change must follow
them.

## jj is recommended, never required

jj is recommended, never required: every feature must work in a git-only
repo.

- chaoshub selects jj or git for each repo. A repo with a `.jj` directory
  uses jj when `jj` is on the `PATH`. Every other repo uses git (see
  `src/vcs/mod.rs`).
- A feature that reads or changes a local repo must have a git path. If
  the feature uses jj, it must also work when the repo has no `.jj` and
  when `jj` is not on the `PATH`.
- A test of such a feature must also run against a temporary git-only
  repo (see `src/vcs/testrepo.rs`).

## Required checks

Every change must pass the required checks. CI runs them on Linux and
on macOS. Before each push, run the checks with this command:

```sh
scripts/check.sh
```

The script runs these four checks, in the order of CI:

```sh
cargo build --locked
cargo nextest run --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --check
```

- Run the script, not the four commands one by one. CI runs the same
  script (see `.github/workflows/ci.yml`). Other flags can pass locally
  and fail in CI.
- Before the first check, the script makes sure that rustc and clippy
  have the exact version of `rust-toolchain.toml`. If the script stops
  with a version error, use rustup, or run the script in `nix develop`.
- To run one check, give its step: `scripts/check.sh clippy`. The steps
  are `build`, `test`, `clippy` and `fmt`.
- The Linux job of CI runs the tests without jj on the `PATH`, and the
  macOS job runs them with jj. A local run uses the jj of your `PATH`.
- If a nixpkgs update gives a new Rust version, `nix develop` stops.
  Then set the same version in `rust-toolchain.toml`.
