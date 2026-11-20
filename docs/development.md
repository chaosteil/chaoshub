# Development

- [Checks](#checks)
- [How to extend](#how-to-extend)
- [Benchmark of the review open](#benchmark-of-the-review-open)
- [Screenshots](#screenshots)

To build chaoshub from source, see
[Build from source](install.md#build-from-source). To make a release,
see [RELEASING.md](../RELEASING.md).

## Checks

jj is recommended, never required: every feature must work in a
git-only repo (see [AGENTS.md](../AGENTS.md)).

Every change must pass the required checks of CI. Before each push, run
them with one command:

```sh
scripts/check.sh
```

The script runs `cargo build`, `cargo nextest run`, `cargo clippy` and
`cargo fmt --check` with the same flags and in the same order as CI.
First, it makes sure that rustc has the version of
`rust-toolchain.toml`. [AGENTS.md](../AGENTS.md) gives the details.

## How to extend

The code has three registries. A new action, tab or pane is one new
module plus one line in its registry. The names and the keys must be
unique. A test in each registry makes sure of this.

### Add an action

1. Make a module in `src/actions/`, for example `src/actions/label.rs`.
2. In it, make a struct and implement the `Action` trait of
   `src/actions/mod.rs`:
   - `id`: the action id. It is also the binding name in `[keys]` and
     the table name in `[actions.<id>]`.
   - `label`: the text in the help overlay.
   - `default_key`: the key when `[keys]` does not bind the id, or
     `None`.
   - `applies`: `Enabled`, `Hidden` (the key goes on to the tab), or
     `Disabled("reason")` (the status line shows the reason). It runs on
     every key press, so it must be pure and fast.
   - `prepare`: `Run`, `Confirm(ConfirmSpec)`, `Prompt(PromptSpec)`,
     `Terminal(LaunchRequest)` or `Command(Cmd)`.
   - `execute`: the async task for `Run`, `Confirm` and `Prompt`. It
     gets the `Selection`, the `ActionInput` and the `Services` (the
     GitHub client, the repo locks, a cancel token and the message
     channel). It returns `Ok(text)` or `Err(text)` for the status line.
3. In `src/actions/mod.rs`, add `mod label;` and add one line to
   `REGISTRY`:

   ```rust
   Box::new(label::Label),
   ```

The action reads its configuration from `Selection.action_config`, the
`[actions.<id>]` table (see
[Configuration](configuration.md#actionsid)). After the action is done,
chaoshub syncs the pull request (PR) again and scans the repo again,
for every action.

An action that changes GitHub also implements `pending`. It returns the
parts of the PRs that it changes (`bus::PendingPart`) and the change
that GitHub must show for each part (`bus::Expect`). chaoshub then
pulses the parts and ends them by the same rules for every action (see
[Changes on their way to GitHub](actions.md#changes-on-their-way-to-github)).
An action without `pending` changes nothing on GitHub.

### Add a tab

1. Make a module in `src/app/tabs/`, for example
   `src/app/tabs/starred.rs`.
2. Implement the `Tab` trait of `src/app/tabs/mod.rs`: `id`, `title`,
   `scope` (the sync scope that feeds the tab), `wants_local`,
   `init_state`, `model` (the `ListModel` of the tab), `interest`,
   `rescore` (the fuzzy filter), `authors` (the author picker),
   `handle_key` and `help`.
3. In `src/app/tabs/mod.rs`, add `mod starred;` and add the tab to
   `TABS`:

   ```rust
   LazyLock::new(|| {
       vec![
           Box::new(mine::Mine),
           Box::new(team::Team::Review),
           Box::new(team::Team::Rest),
           Box::new(starred::Starred),
       ]
   })
   ```

The order of `TABS` is the order of the tab bar and of the keys `1` to
`9`. There can be at most nine tabs.

### Add a pane

1. Make a module in `src/app/panes/`, for example
   `src/app/panes/files.rs`.
2. Implement the `Pane` trait of `src/app/panes/mod.rs`: `id`, `title`,
   `key` (the character that selects the pane in the sidebar),
   `applies`, `init_state`, `model` (a `PaneModel`: a table, lines or a
   tree) and `handle_key`.
3. In `src/app/panes/mod.rs`, add `mod files;` and add one line to
   `PANES`:

   ```rust
   Box::new(files::Files),
   ```

The sidebar shows the panes whose `applies` is true, in the order of
`PANES`.

## Benchmark of the review open

`bench/bench.sh` times the open of a PR in codeview, from the start of
nvim (or of the Rex split) to the first file on the screen and to the
review comments. It runs in scratch clones under `$BENCH_DIR`, because
codeview writes refs there. The Rex case starts a private Rex server,
so the panes of the benchmark never reach your own Rex session.

```sh
export BENCH_DIR=/tmp/chaoshub-bench
bench/bench.sh setup     # clone the repo: a warm clone and a stale base template
bench/bench.sh headless  # nvim without a terminal
BENCH_CASE=stale bench/bench.sh headless
bench/bench.sh rex       # rex split and rex zoom, as chaoshub runs them
bench/bench.sh network   # the GitHub calls alone
bench/bench.sh clean
```

Each case runs 5 times (`BENCH_RUNS`) and prints the median of each
stage in milliseconds. `bench/bench.sh` lists all options.

## Screenshots

The screenshots of the README in `docs/screenshots/` come from made-up
test data, not from a GitHub account. An ignored test draws each screen
and writes it as an SVG image (see `src/run/screenshots.rs`). The usual
test run does not write them. To write them again after a change to the
screens, run:

```sh
cargo test --locked readme_screenshots -- --ignored
```

The README links the images with absolute
`raw.githubusercontent.com` URLs of the `main` branch, so that
crates.io also shows them. A new image shows in the README after it is
on `main`.
