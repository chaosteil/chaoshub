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

Every change must pass these four checks:

```sh
cargo build
cargo nextest run
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```
