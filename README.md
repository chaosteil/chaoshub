# chaoshub

chaoshub is a terminal app that tracks your GitHub pull requests. The
Mine tab lists your own pull requests as stacks, and only pull requests.
A local change without a pull request gets no row. The change,
commit and workspace columns of each pull request come from the scan
of your local checkout, with jj or with git. A header row above the list names the columns;
`H` shows or hides it. The Review tab shows the pull requests of your
colleagues that request your review. The Team tab shows the other pull
requests of your colleagues. By default, these two tabs show only the
pull requests that are ready for review. From the list, you can review a
pull request in codeview, merge it, send a review, and do other actions.

chaoshub keeps a local cache. It shows the cached lists at once when it
starts, and then syncs with GitHub in the background.

## Requirements

- **git** on your `PATH`. chaoshub finds the repo of the current
  directory with `git remote get-url origin`, and prefetches pull request
  refs with `git fetch`. In a repo without jj, chaoshub also reads your
  local branches and worktrees with `git`.
- **jj** (Jujutsu, recommended, not required) on your `PATH`. In a jj
  repo, chaoshub reads your local changes and workspaces with `jj`. jj
  adds these features:
  - The change column shows the jj change id, not the branch name.
  - A pull request also matches a local change when the last `/` part
    of its branch name is a prefix of the change id.
  - The scan reads the stack of each workspace, not only the branch tips.
  - `i` copies the change id.
- **gh**, the GitHub command line tool, logged in. Run `gh auth login`
  once. chaoshub gets its token from `gh auth token`.
- **nvim** with the **codeview** plugin, for the review action (`v`).
- **Rex** (optional). Inside Rex, the review and the pull request
  detail open in Rex panes beside chaoshub. Without Rex, the review runs
  in the same terminal, and chaoshub comes back when nvim quits.

### jj and git

chaoshub selects the tool for each repo. It uses no configuration for
this:

- A repo with a `.jj` directory uses jj when `jj` is on your `PATH`. A
  colocated repo (a jj repo that also has `.git`) also uses jj.
- Every other repo uses git. A colocated repo without `jj` on your `PATH`
  also uses git.

In a git repo, the Mine tab shows the local branch name in the change
column, the short commit in the commit column, and the worktree in the
workspace column. The main worktree is `default`, the same as the
default jj workspace. Another worktree has the name of its directory. A
pull request matches a local branch when the branch tip is the head
commit of the pull request, or when the branch name is its head branch.
After a merge, chaoshub runs `jj git fetch` in a jj repo and
`git fetch origin` in a git repo.

chaoshub does not support a jj repo without `.git`. Use a colocated repo
(`jj git init --colocate`). For a jj repo without `.git`, chaoshub shows
an error in the status line.

## Install

chaoshub runs on macOS and Linux. It does not run on Windows, because it
uses Unix-only APIs. Each method below installs the `chaoshub` binary.
You must also install the programs of [Requirements](#requirements):
`git` and `gh` are necessary, and jj, nvim and Rex are optional. The
Homebrew cask and the `.deb` and `.rpm` packages also
install `git` and `gh`.

The releases are on the
[release page](https://github.com/chaosteil/chaoshub/releases).
[RELEASING.md](RELEASING.md) tells how a release is made.

### Homebrew

```sh
brew install chaosteil/tap/chaoshub
```

The cask is in
[chaosteil/homebrew-tap](https://github.com/chaosteil/homebrew-tap). The
macOS binary is not signed. The cask removes the quarantine flag of
macOS after the install, so that macOS starts the binary
([GoReleaser](https://goreleaser.com/customization/homebrew_casks/#signing-and-notarizing)).

### Cargo

From crates.io:

```sh
cargo install --locked chaoshub
```

From the main branch on GitHub:

```sh
cargo install --locked --git https://github.com/chaosteil/chaoshub
```

Cargo compiles chaoshub. You need Rust 1.91 or later. `--locked` uses
the dependency versions of `Cargo.lock`
([cargo install](https://doc.rust-lang.org/cargo/commands/cargo-install.html)).

### cargo-binstall

```sh
cargo binstall chaoshub
```

[cargo-binstall](https://github.com/cargo-bins/cargo-binstall) downloads
the prebuilt archive of the GitHub release, and does not compile.

### Nix

Run chaoshub without an install:

```sh
nix run github:chaosteil/chaoshub
```

Install it in your profile:

```sh
nix profile install github:chaosteil/chaoshub
```

Or add the flake as an input of your own flake, for example in a NixOS
configuration:

```nix
{
  inputs.chaoshub.url = "github:chaosteil/chaoshub";

  outputs = { nixpkgs, chaoshub, ... }: {
    nixosConfigurations.myhost = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        {
          environment.systemPackages = [
            chaoshub.packages.x86_64-linux.default
          ];
        }
      ];
    };
  };
}
```

The flake has packages for `x86_64-linux`, `aarch64-linux` and
`aarch64-darwin`. It does not install `git` and `gh`. Add them to your
packages too.

### Debian, Ubuntu and Fedora

Download the `.deb` or `.rpm` file for your CPU from the
[release page](https://github.com/chaosteil/chaoshub/releases/latest).
Then install it with your package manager:

```sh
sudo apt install ./chaoshub_X.Y.Z_amd64.deb         # Debian, Ubuntu
sudo dnf install ./chaoshub-X.Y.Z-1.x86_64.rpm      # Fedora
```

The packages depend on `git` and `gh`, and recommend `jujutsu` and
`neovim`. If your distribution has no `gh` package, install `gh` first
([GitHub CLI install](https://github.com/cli/cli#installation)).

### Prebuilt archives

The [release page](https://github.com/chaosteil/chaoshub/releases/latest)
has an archive `chaoshub_X.Y.Z_<target>.tar.gz` for each target:

| Target | System |
|---|---|
| `aarch64-apple-darwin` | macOS on Apple silicon |
| `x86_64-apple-darwin` | macOS on Intel |
| `x86_64-unknown-linux-musl` | Linux on x86_64 (static binary) |
| `aarch64-unknown-linux-musl` | Linux on ARM64 (static binary) |

Each archive has the binary, the README and the license files.
`checksums.txt` has the SHA-256 sum of each file.

1. Download the archive and `checksums.txt`.
2. Make sure that the sum is correct:

   ```sh
   sha256sum --check --ignore-missing checksums.txt         # Linux
   shasum -a 256 --check --ignore-missing checksums.txt     # macOS
   ```

3. Extract the binary to a directory on your `PATH`:

   ```sh
   tar -xzf chaoshub_X.Y.Z_aarch64-apple-darwin.tar.gz -C ~/.local/bin chaoshub
   ```

4. If you downloaded the archive with a browser on macOS, remove the
   quarantine flag:

   ```sh
   xattr -d com.apple.quarantine ~/.local/bin/chaoshub
   ```

### Build from source

```sh
git clone https://github.com/chaosteil/chaoshub
cd chaoshub
cargo build --release
```

The binary is `target/release/chaoshub`. To install it in
`~/.cargo/bin`, run `cargo install --locked --path .`.

The repo pins the exact Rust version in `rust-toolchain.toml`. rustup
installs this toolchain when you build in the repo. The devShell of
`flake.nix` (`nix develop`) gives the same version. The minimum Rust
version is 1.91 (`rust-version` in `Cargo.toml`).

## Start

```sh
chaoshub
```

chaoshub tracks the repos of the `[[repo]]` entries of the configuration.
It also adds the repo of the current directory, when its `origin` remote
is a GitHub repo.

Command line options:

| Option | What it does |
|---|---|
| `--repo OWNER/NAME` | Adds a repo for this session. Repeat the option for more repos. A repo from this option has no local path. |
| `--config PATH` | Reads the configuration from `PATH` instead of `~/.config/chaoshub/config.toml`. |
| `--no-prefetch` | Turns off the prefetch of pull request refs for this session. |
| `--theme STYLE` | Uses `punk`, `grunge` or `plain` for this session, in place of `[theme] style`. |
| `--palette NAME` | Uses `pierre` or `ansi16` for this session, in place of `[theme] palette`. |
| `cache clear` | Deletes the cache file. The next start syncs from GitHub again. |
| `list [--json] [--all]` | Prints the pull requests of the tabs from the cache, and makes no GitHub call. See "List the pull requests". |
| `pr OWNER/NAME NUMBER` | Shows one pull request and nothing else: the panes of the pull request detail. A pull request that is not in the cache loads from GitHub. `q` or `esc` quits. The detail pane inside Rex runs this command. |

To show one pull request, for example #7 of `chaosteil/chaoshub-test`,
run:

```sh
chaoshub pr chaosteil/chaoshub-test 7
```

### List the pull requests

`chaoshub list` prints the open pull requests of the cache that the
tabs show: first the Mine tab, then the Review tab, then the Team tab,
in the order of each tab. It is for scripts, for example the Rex action
of "Open a pull request from the Rex palette".

- The command opens the cache read-only. It makes no GitHub call, so
  the list is as old as the last sync of the TUI. If the cache does not
  exist, the list is empty.
- The rules of the normal view apply: the `[filters]` age limit for all
  tabs, and the ready rule for the Review tab and the Team tab. With
  `--all`, the command prints every open pull request of the cache, as
  the `show_hidden` key (`T`) does.
- The text filter and the author filter of the tabs do not apply.
- The command reads the configuration file (or `--config`) for the age
  limit and for the `path` of each repo.

Without `--json`, each line has five fields, split by tab characters:
the tab, `owner/name#N`, the author, the status and the title.

With `--json`, the command prints one JSON object on one line:

```json
{
  "version": 1,
  "synced_at": "2026-11-18T12:00:00Z",
  "prs": [
    {
      "repo": "chaosteil/chaoshub",
      "number": 7,
      "title": "Fix the merge",
      "author": "chaosteil",
      "url": "https://github.com/chaosteil/chaoshub/pull/7",
      "tab": "mine",
      "state": "open",
      "draft": false,
      "review_requested": false,
      "status": "ready · CI passed · approved",
      "label": "#7 Fix the merge · ready · CI passed · approved",
      "path": "/home/me/code/chaoshub"
    }
  ]
}
```

| Field | What it holds |
|---|---|
| `version` | The version of the format, now `1`. A change that removes or renames a field, or changes its meaning, increments it. A new field does not. |
| `synced_at` | The newest sync time of the cached pull requests, or `null` when the cache holds none. |
| `repo`, `number` | The pull request. `chaoshub pr <repo> <number>` shows it. |
| `title`, `author`, `url` | The title, the login of the author and the GitHub URL. |
| `tab` | `mine`, `review` or `team`. |
| `state` | `open`. The list holds only open pull requests. |
| `draft` | True for a draft. |
| `review_requested` | True when GitHub requests the review of you or of one of your teams. |
| `status` | The verdict, the CI and the reviews, as in the PR overview (see `[terminal]`). |
| `label` | The PR overview, for example `#7 Fix the merge · ready · CI passed · approved`. The detail pane inside Rex uses it as the block label. |
| `path` | The `path` of the `[[repo]]` entry of the repo, or `null`. |

## Files

| File | Location |
|---|---|
| Configuration | `~/.config/chaoshub/config.toml` (`$XDG_CONFIG_HOME/chaoshub/config.toml`) |
| Cache (SQLite) | `~/.cache/chaoshub/chaoshub.db` (`$XDG_CACHE_HOME/chaoshub/chaoshub.db`) |
| Log | `~/.local/state/chaoshub/chaoshub.log` (`$XDG_STATE_HOME/chaoshub/chaoshub.log`) |
| pr-agent results | `~/.local/state/chaoshub/agent/<owner>/<repo>/<n>/` (see "pr-agent") |
| List socket | `$XDG_RUNTIME_DIR/chaoshub/list-<pid>.sock`, else `~/.local/state/chaoshub/ipc/list-<pid>.sock` (see "How the PR view talks to the list") |

chaoshub uses these XDG paths on macOS too, not `~/Library`.

The cache holds the last lists, the local scans and the filters of the
tabs. If the cache does not open, chaoshub runs with an empty cache in
memory. To delete the cache, run:

```sh
chaoshub cache clear
```

The log gets messages at the info level and above. The log file grows
until you delete it.

## Configuration

The configuration file is TOML. Every key has a default, so an empty
file or no file is a valid configuration. A file that does not parse
stops chaoshub with an error that names the file. In paths, a leading
`~` is your home directory.

### Top-level keys

```toml
login = "your-github-login"
teams = ["my-org/my-team"]
launcher = "auto"
```

| Key | Default | Meaning |
|---|---|---|
| `login` | none | Your GitHub login. When it is not set, chaoshub asks GitHub for the login of the token. |
| `teams` | `[]` | Teams in the `org/team` form. A review request to one of these teams counts as a review request to you. |
| `launcher` | `"auto"` | Where the review pane and the detail pane open. `auto` and `rex` use a Rex pane when chaoshub runs inside Rex (`REX_BLOCK` is set). `inline` always runs the review in the same terminal. Any other value is an error. |

### `[[repo]]`

One entry per repo to track.

```toml
[[repo]]
name = "chaosteil/chaoshub"
path = "~/code/chaoshub"

[[repo]]
name = "my-org/deploy"
```

| Key | Default | Meaning |
|---|---|---|
| `name` | required | The repo in the `owner/name` form. |
| `path` | none | The local checkout, a jj repo or a git repo. Without a path, chaoshub shows no local data for the repo, and the review action is disabled. |

### `[filters]`

Set the maximum age of pull requests in the Mine, Review and Team lists:

```toml
[filters]
max_age = "2 weeks"
```

`max_age` measures the time since a pull request was created. For example,
`2 weeks` keeps PRs created within the last 14 days. The window moves with
the current time. A later update does not reset the age of a PR.

Use a positive duration such as `14 days`, `2 weeks` or `48 hours`.
The default has no age limit. Invalid durations and unknown filter keys
stop configuration loading. Restart chaoshub after a configuration change.

The limit applies to GitHub searches and cached lists, including the
author picker and the `t` switch. Old cache rows without creation times
stay hidden until a sync reads that field. No cache reset is needed.

The limit changes list membership. It does not change a stack on GitHub
or prevent `chaoshub pr OWNER/NAME NUMBER` from opening a specific PR.
The existing author and text filters apply within the included PRs.

The `T` switch skips the limit (see "Hidden rows"). While `T` is on in a
tab, the syncs search GitHub without the limit.

### `[refresh]`

The poll intervals, in seconds.

| Key | Default | Meaning |
|---|---|---|
| `mine_secs` | `60` | The sync of your own pull requests (only the changes since the last sync). |
| `team_secs` | `90` | The sync of the pull requests of your colleagues (only the changes). |
| `full_secs` | `900` | The full sync of both lists. The prefetch also fetches each base branch once in this period, and again when a sync reports a new base commit of a pull request. |
| `ci_secs` | `45` | The CI poll of the visible pull requests, and of the watched pull requests of `[notify]` (see "Notice of new pull requests"). |
| `jj_secs` | `30` | The local scan with jj or git, while the active tab shows local data. |

When the GitHub rate budget runs low, the sync intervals double.

The detail pane (`chaoshub pr`) refreshes its pull request every
`mine_secs`. GitHub computes the merge state of a pull request on
request, for example after its base changes. When a refresh gives an
unknown merge state, the detail pane refreshes again after 3 s, at most
10 times in a row. The `refresh` key starts this count again.

### `[prefetch]`

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `true` | Fetches the git refs of the selected and the visible pull requests into the local repo, so a review in codeview starts without a fetch. The refs go under `refs/chaoshub/`, which jj does not import. |

### `[codeview]`

The Rex pane of the review action, and the codeview comment files.

| Key | Default | Meaning |
|---|---|---|
| `zoom` | `true` | Zooms the pane, so nvim fills the window. |
| `split` | `"right"` | The direction of the new pane: `right`, `left`, `above` or `below`. |
| `ratio` | none | The percent of the split that the pane gets. Without it, Rex uses its default. |
| `comments_dir` | see "Meaning" | The directory of the codeview comment files. `~` expands to your home directory. The default is the default of codeview: `$XDG_DATA_HOME/<app>/codeview/review`, else `~/.local/share/<app>/codeview/review`. `<app>` is `$NVIM_APPNAME`, else `nvim`. Set it when you set the `comments.dir` option of codeview. |

### `[detail]`

Where `enter` opens the detail of a pull request (see "Pull request
detail").

```toml
[detail]
mode = "auto"
split = "right"
ratio = 50
```

| Key | Default | Meaning |
|---|---|---|
| `mode` | `"auto"` | `auto` opens a Rex pane inside Rex, and the sidebar elsewhere. `pane` always opens the pane. Without Rex, the pane runs in the same terminal, and the list comes back when it quits. `sidebar` always opens the sidebar. Any other value is an error. |
| `split` | `"right"` | The direction of the pane: `right`, `left`, `above` or `below`. |
| `ratio` | none | The percent of the split that the pane gets. Without it, Rex uses its default. |

### `[conversation]`

The conversation pane of the pull request detail (see "Pull request
detail").

```toml
[conversation]
order = "newest_first"
```

| Key | Default | Meaning |
|---|---|---|
| `order` | `"oldest_first"` | The order of the comments, the reviews and the threads, and of the comments in each thread. `oldest_first` shows them in time order, with the oldest at the top. `newest_first` shows the newest at the top. A thread header stays above its comments. Any other value is an error. |

### `[merge]`

The settings of the merge action (`m`). The `merge_stack` action (`s`)
uses the same settings.

```toml
[merge]
method = "squash"
bypass_rules = false

[merge.repos."my-org/deploy"]
method = "rebase"
bypass_rules = true
```

| Key | Default | Meaning |
|---|---|---|
| `method` | none | The merge method: `merge`, `squash` or `rebase`. Without it, chaoshub uses `squash` when the repo permits it, else `merge`, else `rebase`. |
| `bypass_rules` | `false` | Asks GitHub to bypass the branch rules on a merge. |
| `stack` | `auto` | How chaoshub merges a stack: `auto`, `atomic` or `sequential`. See "Actions" below. |
| `repos."owner/name"` | none | A table with `method`, `bypass_rules` and `stack` for one repo. Each key that it sets overrides the top-level key for that repo. |

chaoshub copies these keys into `[actions.merge]`. If you set a key in
`[actions.merge]`, that value wins over the value in `[merge]`.

### `[actions.<id>]`

Each action reads the table with its id. Four actions use a table:

| Table | Key | Meaning |
|---|---|---|
| `[actions.review]` | `zoom` | Zooms the review pane. The default is the value of `[codeview] zoom`. |
| `[actions.codeview_send]` | `comments_dir` | The directory of the codeview comment files. The default is the value of `[codeview] comments_dir`. |
| `[actions.merge]` | `method`, `bypass_rules`, `stack`, `repos` | The same keys as `[merge]`. |
| `[actions.copy_stack_urls]` | `order` | The order of the URLs that `copy_stack_urls` (`Y`) copies. `"bottom_first"` (the default) is the order of the Stack pane: position 1, the bottom of the stack, first. `"top_first"` is the order of the list: the top of the stack first. Any other value is an error. |

```toml
[actions.copy_stack_urls]
order = "top_first"
```

The other actions ignore their table. A new action can read its own
table (see "How to extend").

### `[agent]`

The pr-agent runs (see "pr-agent" below).

```toml
[agent]
command = ["pr-agent"]
model = "anthropic/claude-sonnet-5-5"
fallback_models = []
timeout_secs = 900
extra_instructions = { review = "", improve = "", describe = "", ask = "" }

[agent.env]
ANTHROPIC__KEY = { command = "op read op://dev/anthropic/key" }
```

| Key | Default | Meaning |
|---|---|---|
| `command` | `["pr-agent"]` | The pr-agent program and its first arguments. A list of strings, or a string. A string is one program name: chaoshub does not split it at the spaces. So write `uvx --from pr-agent pr-agent` as a list. |
| `model` | `"anthropic/claude-sonnet-5-5"` | The LiteLLM name of the model. chaoshub does not read it. It gives it to pr-agent. |
| `fallback_models` | `[]` | The LiteLLM names of the models that pr-agent tries when the model fails. |
| `env` | `{}` | The env of the pr-agent process: a variable name to a value. See below. |
| `timeout_secs` | `900` | After this time, chaoshub stops the run. |
| `extra_instructions` | `""` for each tool | Extra instructions for the model, one text per tool: `review`, `improve`, `describe` and `ask`. |

A value of `env` has one of these forms:

| Form | Value |
|---|---|
| `"text"` | The text as it is. |
| `{ from_env = "NAME" }` | The value of the variable `NAME` in the env of chaoshub. A missing variable stops the run. |
| `{ command = "op read op://dev/anthropic/key" }` | The standard output of the shell command, without the spaces and the line breaks at the two ends. A command that fails stops the run, and the status line shows its error output. |

chaoshub reads each value once per run, when the run starts. chaoshub
writes no value to the log or to the screen. The process also gets the
env of chaoshub.

chaoshub has no provider code. pr-agent reaches every provider through
LiteLLM, so a provider is only a model name and the keys in `env`. For
the names of the keys, see the pr-agent docs:
https://github.com/qodo-ai/pr-agent/blob/main/docs/docs/usage-guide/changing_a_model.md

Anthropic direct
([docs](https://github.com/qodo-ai/pr-agent/blob/main/docs/docs/usage-guide/changing_a_model.md#anthropic)):

```toml
[agent]
model = "anthropic/claude-sonnet-5-5"

[agent.env]
ANTHROPIC__KEY = { from_env = "ANTHROPIC_API_KEY" }
```

OpenAI direct
([docs](https://github.com/qodo-ai/pr-agent/blob/main/docs/docs/usage-guide/changing_a_model.md#changing-a-model-in-pr-agent)):

```toml
[agent]
model = "openai/gpt-6.1-sol"
fallback_models = ["openai/gpt-6-luna"]

[agent.env]
OPENAI__KEY = { command = "op read op://dev/openai/key" }
```

OpenRouter
([docs](https://github.com/qodo-ai/pr-agent/blob/main/docs/docs/usage-guide/changing_a_model.md#openrouter)):

```toml
[agent]
model = "openrouter/anthropic/claude-sonnet-5"

[agent.env]
OPENROUTER__KEY = { from_env = "OPENROUTER_API_KEY" }
```

An OpenAI-compatible gateway, for example a LiteLLM proxy
([pr-agent docs](https://github.com/qodo-ai/pr-agent/blob/main/docs/docs/usage-guide/changing_a_model.md#changing-a-model-in-pr-agent),
[LiteLLM proxy docs](https://docs.litellm.ai/docs/simple_proxy)). Use
the `openai/` prefix and the model id of the gateway. For a model that
pr-agent does not know, also set the input limit:

```toml
[agent]
model = "openai/claude-sonnet"

[agent.env]
OPENAI__API_BASE = "http://localhost:4000"
OPENAI__KEY = { from_env = "LITELLM_PROXY_KEY" }
CONFIG__CUSTOM_MODEL_MAX_TOKENS = "200000"
```

Ollama
([docs](https://github.com/qodo-ai/pr-agent/blob/main/docs/docs/usage-guide/changing_a_model.md#ollama)):

```toml
[agent]
model = "ollama/qwen2.5-coder:32b"

[agent.env]
OLLAMA__API_BASE = "http://localhost:11434"
CONFIG__CUSTOM_MODEL_MAX_TOKENS = "128000"
```

Other forms of `command`:

```toml
# pr-agent from PyPI through uv, with no install.
command = ["uvx", "--from", "pr-agent", "pr-agent"]

# The Docker image. ask does not work with this form (see "pr-agent").
# The container gets only the env names that -e lists, and it must see
# the state directory at the same path for the JSON result.
command = [
  "docker", "run", "--rm",
  "-v", "/Users/me/.local/state/chaoshub:/Users/me/.local/state/chaoshub",
  "-e", "GITHUB__USER_TOKEN", "-e", "CONFIG__MODEL", "-e", "CONFIG__FALLBACK_MODELS",
  "-e", "CONFIG__PUBLISH_OUTPUT", "-e", "CONFIG__PROPAGATE_TOOL_ERRORS",
  "-e", "ANTHROPIC__KEY",
  "pragent/pr-agent:latest",
]
```

### `[theme]`

| Key | Default | Meaning |
|---|---|---|
| `style` | `punk` | The look: `punk`, `grunge` or `plain`. |
| `palette` | `pierre` | The colors: `pierre` or `ansi16`. On a 16-color terminal, `pierre` changes to `ansi16`. |
| `grunge` | `true` | Draws the rough block borders. Applies to the grunge style only. |
| `textures` | `true` | Fills empty areas with noise textures. Applies to the grunge style only. |
| `animations` | `true` | Turns the transitions on (see the list below the table). With `false`, none of them runs. |
| `animation_ms` | `120` | The duration of the popup, output, help and picker slides and of the scroll, in milliseconds. The other transitions have their own fixed durations (see the list below the table). |
| `key_hints` | `true` | Shows the key hint bar above the status bar. |
| `column_headers` | `true` | Shows the header row with the column names above the list. The `H` key turns it on or off for the session and saves the choice in the cache. A saved choice wins over this value. |

The style and the palette are two switches. The style sets the shapes,
and the palette sets the colors. The default style is punk: cut
corners, diagonal accents, inverted headers, slanted tab cards, a
two-layer selection band and drop shadows under popups. Punk keeps the
palette, also on a 16-color terminal.

The other style is grunge: rough block borders, stencil headers and
noise textures. To try it, start chaoshub with `--theme grunge`. The
`grunge` and `textures` keys only affect the grunge style. With
`grunge = false`, or on a 16-color terminal, the grunge style draws the
plain look.

The transitions are short, and a key press during a transition
retargets it, so nothing blocks typing:

- The popup, the output and the help slide in from the right and out
  again (`animation_ms`, 120 ms by default). The confirm and the prompt
  slide in from the bottom in 100 ms, the picker in `animation_ms`. In
  the punk style, the drop shadow slides with the popup.
- A tab switch: the highlight of the active tab glides along the tab
  bar in 120 ms. The list changes at once.
- A pane switch in the pull request detail: the same glide on the pane
  tabs, in 120 ms. The pane changes at once.
- A scroll: the rows move to the new offset (`animation_ms`).
- A cursor move in the list, a pane table or the picker: the selection
  band wipes onto the new row from the left in 60 ms.
- A successful action: the row of the pull request flashes in the
  primary color and fades back in 200 ms.
- A sync or a CI poll that changes the checks or the reviews of a
  visible row: the row glows grey and fades back in 250 ms.
- An expanded folder, file or thread in the pull request detail: the new
  rows unfold from top to bottom in 80 ms. Collapsing is instant.
- A new status message slides in from the left in 80 ms. An expired
  message fades out in 300 ms.

If 5 of the last 10 drawn frames take longer than 12 ms to draw,
chaoshub turns the transitions off until it quits, and writes a line in
the log.

### `[keys]`

Overrides the key bindings. Each key is a binding name, and each value
is one or more key chords, separated by spaces. An override replaces
every default chord of that binding. An empty value removes the binding.

```toml
[keys]
merge = "ctrl-m"
refresh = "ctrl-r F5"
toggle_all = ""
```

A chord is a character (`q`, `A`, `/`), a key name (`enter`, `esc`,
`tab`, `shift-tab`, `backspace`, `delete`, `insert`, `home`, `end`,
`pageup`, `pagedown`, `up`, `down`, `left`, `right`, `space`, `F1` to
`F12`), or one of these with modifiers (`ctrl-d`, `alt-x`,
`ctrl-alt-x`). A single character is case-sensitive: `a` and `A` are
different keys. A chord that does not parse stops chaoshub with an
error.

The binding names are the app keys and the action ids in the keymap
below. A `detail_<id>` binding, for example `detail_close = "c"`,
runs the action `<id>` directly in the list and in the pull request
detail (see "Actions").

### `[terminal]`

The terminal title, status sequences (see "Rex, Ghostty and codeview")
and mouse controls (see "Mouse").

| Key | Default | Meaning |
|---|---|---|
| `title` | detected | Sets the terminal title with the count of ready PRs to review, or the PR overview in the standalone PR view, and the current activity. |
| `program_status` | detected | Writes OSC 7501, the program status. |
| `progress` | detected | Writes OSC 9;4, the progress bar. |
| `mouse` | `true` | Reads the mouse. `false` leaves the mouse to the terminal, so the terminal selects the text. chaoshub selects text itself (see "Mouse"), so `false` is only necessary to select across panes. |

When `title`, `program_status` or `progress` is not set, chaoshub writes
the sequence only inside Rex (`REX_BLOCK` is set) and Ghostty
(`TERM_PROGRAM=ghostty`). `true` forces it on, `false` forces it off.

The list title is `chaoshub · 3 PRs to review`, or
`chaoshub · 1 PR to review` for one PR. The count uses the normal
Review tab: open PRs that request review from you or your teams, are
not drafts, have passed CI, and pass the age limit. Text and author
filters and the `t` and `T` switches do not change it. Until the first
review list sync completes, `? PRs to review` shows that the count is
not known yet.

One short activity can follow the count, such as `merging #42`,
`input` or `error`. While chaoshub syncs or prefetches pull requests, a
spinner follows the count instead, for example
`chaoshub · 3 PRs to review · ⠋`. The spinner moves five times per second. The status bar names the
work and shows the error details.

Rex shows its own spinner on the tab for the OSC 7501 `working` state.
For this reason, the title has no spinner when chaoshub runs in Rex
(`REX_BLOCK` is set) and writes OSC 7501 (`program_status`). While
chaoshub syncs, the title is then only `chaoshub · 3 PRs to review`. The other
activities do not change. Ghostty and other terminals keep the title
spinner.

The standalone PR view (`chaoshub pr`) shows a short overview of its
pull request instead of the count of PRs to review:

```text
#7 Fix the merge · blocked · CI failed · approved
```

The parts are:

1. The PR number and the PR title. A title of more than 30 characters
   is cut and ends with `…`.
2. The verdict of the Description pane in lowercase: `ready`,
   `blocked`, `waiting`, `checking`, `conflicts`, `draft`, `in queue`,
   `merged` or `closed`.
3. The CI state: `CI passed`, `CI failed` or `CI running`. A pull
   request without checks has no CI part.
4. The reviews: `approved`, `changes requested` or `review required`.
   A pull request that needs no review and has no reviews has no review
   part.

A merged or closed pull request shows only the number, the title and
the verdict. Before the pull request loads, the title is `#7 · loading`.
The activity follows the overview, for example
`#7 Fix the merge · ready · CI passed · approved · merging #7`.

The title uses OSC 2. It updates only when its text changes. It clears
when chaoshub exits or hands the terminal to another program, then
updates again when chaoshub resumes. It does not rename Rex sessions or
windows. Set `title = false` in `[terminal]` to turn title updates off.

The PR view that the list opens in a Rex pane also keeps the label of
its Rex block at the overview, without the activity (see "What `enter`
runs inside Rex"). Rex shows this label in the header of the block.

### `[notify]`

The terminal notice of new pull requests that are ready for review (see
"Notice of new pull requests").

| Key | Default | Meaning |
|---|---|---|
| `enabled` | detected | Sends the notice. When it is not set, chaoshub sends the notice only inside Rex and Ghostty, the same as `[terminal] program_status`. |
| `tabs` | `["team", "review"]` | The tabs to watch: `"mine"`, `"review"` or `"team"`. Another value stops chaoshub with an error. |
| `debounce` | `"15s"` | The wait before the notice, for example `"15s"` or `"2m"`. `"0s"` sends the notice at once. |
| `desktop` | `true` | Also sends each notice as a desktop notification with its text (OSC 777). `false` keeps only the program status record, so Rex shows only the tab dot. |

```toml
[notify]
tabs = ["review"]
debounce = "1m"
```

### `[workspaces]`

| Key | Default | Meaning |
|---|---|---|
| `root` | `"~/code/workspaces"` | The directory of the jj workspaces other than `default`: `<root>/<folder>/<workspace name>`, where `<folder>` is the folder name of the repo path. |

## List columns

The list shows the state of each pull request with glyphs and colors.
The glyphs are the same in every theme. Only the colors change: the
names below (success, danger, warning, primary, dim) are the colors of
the palette. Press `?` to see the same legend in the help overlay
("list symbols"), in the colors of your theme.

### CHECKS

The CI state of the pull request, then the passed checks of all checks.
A check passed when its conclusion is success, neutral or skipped.

| Symbol | Color | Meaning |
|---|---|---|
| `·` | dim | No checks. |
| `✓ 5/5` | success | CI is green. 5 of 5 checks passed. |
| `✗ 3/5` | danger | CI failed. 3 of 5 checks passed. |
| `● 2/5` | warning | CI is pending. 2 of 5 checks passed. |

### REV

The reviews. With approvals or change requests, the column shows their
counts. Without counts, it shows the review decision.

| Symbol | Color | Meaning |
|---|---|---|
| `✓2` | success | 2 approvals. |
| `✓2 ✗1` | danger | 2 approvals and 1 change request. A change request gives the cell the danger color. |
| `✓` | success | Approved. |
| `✗` | danger | Changes requested. |
| `○` | dim | A review is required. |
| `·` | dim | No review is needed. |

### FLG

The flags, in this order, in the bold warning color.

| Symbol | Meaning |
|---|---|
| `D` | The pull request is a draft. |
| `C` | The pull request has merge conflicts. |
| `Q` | The pull request is in the merge queue. |
| `◆` | A pane of the pull request is open. |

### CHANGE

The Mine tab only. The local change that matches the pull request. In a
git repo, the change is the branch name.

| Color | Meaning |
|---|---|
| Primary | Synced: the local change has the same commit as the pull request head. |
| Warning | Diverged: the local change has a different commit. The commit column also has the warning color. |

### The stack tree

The Mine tab only. The first column starts with a tree glyph. A stack
starts with a header row, for example `stack #12 -> main · 3 PRs`.

| Symbol | Meaning |
|---|---|
| `─` | A pull request without a stack. |
| `┌` | The top pull request of a stack. |
| `├` | A pull request in the middle of a stack. |
| `└` | The bottom pull request of a stack. |

GitHub does not change the update time of a pull request when it adds
the pull request to a stack. Thus chaoshub finds a new stack in one of
two ways:

- A sync reads a pull request of the stack that changed after GitHub
  made the stack. Then chaoshub also reads the other open entries of
  the stack again.
- The next full sync (`refresh.full_secs`, and at the start) finds the
  new place in the stack of each pull request.

### Ready for review

By default, the Review and the Team tab show only the pull requests that
are ready for review: open, not a draft, and CI passed. `t` shows all open
pull requests, and a second `t` shows only the ready ones again.

### Hidden rows

Four rules hide pull requests from a list:

- the age limit (`[filters] max_age`),
- the ready rule of the Review and the Team tab (`t`),
- the author picker (`a`),
- the filter text (`/`).

Press `T` to show every open pull request of the active tab. The four
rules then do not hide rows. In the Mine tab, only the age limit and the
filter text apply. Press `T` again for the normal view.

| Symbol | Meaning |
|---|---|
| `?` | Before the title: only `T` shows this row. The normal view hides it. In the Mine tab, the `?` comes before the tree glyph. |
| dim row | The same rows are dim. |

While `T` is on, the mode row at the top of the list shows the number of
rows, for example `everything · 12 · T shows normal view`.

Each tab has its own `T` switch. chaoshub does not save it: after a
restart, every tab shows the normal view.

While `T` is on in a tab, the syncs search GitHub without the age limit.
When you set `T` on, chaoshub starts a full sync of the tab at once, so
the pull requests past the limit arrive after a few seconds. Setting `T`
off starts no sync. While `T` is on, a full
sync does not remove the pull requests past the limit from the list.
After you set `T` off in every tab, the next full sync removes them
again.

The rows that `T` shows count as visible rows: chaoshub polls their CI,
loads their details and prefetches their refs, like any other row.
These requests and the searches without the age limit use more of the
GitHub rate budget.

## Filter

Press `/` to type a filter for the active tab. The list shows only the
pull requests that match the filter. Each tab has its own filter.

The filter searches this text of each pull request: the title, the number
with `#`, the author and the head branch, in this order. For example:

```text
fix(store): reopen the connection #99 alice alice/reopen
```

The filter does not search the repo, the base branch or the labels.
`T` also shows the pull requests that the filter hides, as dim rows with
a `?` (see "Hidden rows").

| Key | What it does |
|---|---|
| `enter` | Keeps the filter. chaoshub saves it in the cache. The key hint bar shows `Enter apply`. |
| `esc` | Clears the filter. The key hint bar shows `Esc clear`. |
| `backspace` | Deletes the last character. |
| `down`, `up` | Moves the cursor while you type. |

### Syntax

A space separates two words. A pull request must match every word (AND).
There is no OR.

| Word | Matches | Example | Example matches |
|---|---|---|---|
| `abc` | The letters of `abc` in this order, with other characters between them (fuzzy). | `fx conn` | `fix(store): reopen the connection` |
| `'abc` | The exact text `abc`. | `'reopen` | `reopen`, not `re-open` |
| `^abc` | Text that starts with `abc`. The text starts with the title. | `^fix` | `fix(store): ...`, not `docs: fix ...` |
| `abc$` | Text that ends with `abc`. The text ends with the branch. | `reopen$` | the branch `alice/reopen` |
| `!abc` | Text that does not contain `abc`. | `!draft` | every title without `draft` |
| `\` | Escapes a special character: `\!`, `\^` or `\'` at the start, `\$` at the end, and `\ ` for a space in a word. | `fix\ the` | `fix the` as one word |

Each word has smart case. A word with only lower case letters ignores the
case. A word with an upper case letter matches the case. For example,
`store` matches `Store` and `store`, but `Store` matches only `Store`.

More examples:

| Filter | Shows |
|---|---|
| `#99` | The pull request #99. The match is fuzzy, so other numbers can match too, for example #199. |
| `alice` | The pull requests of `alice`, and other text with these letters. |
| `^feat !wip` | The titles that start with `feat` and do not contain `wip`. |
| `ui 'popup` | The pull requests with `ui` (fuzzy) and the exact text `popup`. |

## Keymap

The default bindings. The name in the first column is the binding name
for `[keys]`.

### App keys

| Binding | Keys | What it does |
|---|---|---|
| `tab_1` to `tab_9` | `1` to `9` | Selects a tab. In the pull request detail, the number keys select a pane. |
| `next_tab` | `tab` | The next tab. In the detail, the next pane. |
| `down`, `up` | `j`, `k` | Moves the cursor. |
| `top`, `bottom` | `g`, `G` | Moves to the first or the last row. |
| `page_down`, `page_up` | `ctrl-d`, `ctrl-u` | Moves half a page. In the pull request detail, moves half the height of the active pane. |
| `scroll_down`, `scroll_up` | `D`, `U` | In the pull request detail, scrolls the active pane down or up by half its height. The cursor stays. When the cursor leaves the pane, it moves to the first visible row. |
| `open` | `enter` | Opens the pull request detail: a Rex pane inside Rex, else the sidebar (see `[detail]`). A second `enter` on the same pull request focuses its pane. In the conversation pane, collapses or expands a thread, and opens the comment menu on a comment (see "Reactions"). |
| `open_url` | `o` | In the checks pane, opens the selected check in the browser. |
| `actions` | `space` | Opens the actions menu: in the list, of the selected row; in the pull request detail, of its pull request and its active pane (see "Actions"). |
| `close_overlay` | `esc`, `q` | Closes the sidebar or the overlay. When nothing is open, these keys quit (see `quit`). In the Rex pane of the detail, they quit the pane. |
| `confirm` | `y` | Confirms in a confirm popup. `enter` also confirms. |
| `cancel` | `n` | Cancels a confirm popup. |
| `filter` | `/` | Types a fuzzy filter for the active tab. |
| `authors` | `a` | Opens the author picker of the Review or the Team tab. Inside Rex, the picker can open in the Rex palette (see "Questions in the Rex palette"). |
| `jump` | `f` | Inside Rex, asks for a pull request of the rows of all tabs in the Rex palette. The pick selects the tab and the row, and opens the pull request as `enter` does. Without the Rex palette, `f` opens the filter, as `/` does (see "Questions in the Rex palette"). |
| `toggle_all` | `t` | On the Review and the Team tab, switches between every open pull request and only the ready ones. The help shows it as "all open / ready only". |
| `show_hidden` | `T` | Switches between the normal view and every open pull request of the active tab, also the rows that the age limit, the ready rule, the author picker and the filter text hide (see "Hidden rows"). The help shows it as "hidden rows / normal view". Only the help overlay shows this key, not the key hint bar. |
| `refresh` | `R` | Syncs both lists now. In the pull request detail, syncs the pull request and its stack, and scans its repo. |
| `column_headers` | `H` | Shows or hides the header row with the column names. chaoshub saves the choice in the cache. |
| `help` | `?` | Shows the help overlay with every binding. |
| `quit` | `q`, `esc`, `ctrl-c` | Quits. When the sidebar or an overlay is open, `esc` and `q` close it first. While you type a filter, `esc` clears the filter. `ctrl-c` quits from everywhere. |

In a prompt popup, `enter` sends the text and `esc` cancels.

### Pull request detail

`enter` on a pull request opens its detail. Where it opens depends on
`[detail] mode`.

Inside Rex, `enter` runs `chaoshub pr OWNER/NAME N` in a Rex pane beside
the list, and the pane takes the focus. The pane shows the panes of the
pull request and nothing else. `q` or `esc` in the pane closes the pane,
and the list then syncs the pull request again and scans its repo.
`enter` on another pull request opens a new pane for that pull
request, and the panes that are open stay open. Each new pane splits
the list block, with the `[detail] split` and `[detail] ratio`. `enter`
on a pull request that has an open pane focuses that pane. A quit of
the list closes all its panes. A quit of a pane leaves a codeview
review from `v` open.

Outside Rex, `enter` opens the sidebar. The sidebar slides in from the
right and covers 80% of the width, over the list rows. The tab bar, the
key hint bar and the status bar stay visible. `esc` or `q` closes the
sidebar.

The detail has five panes. The number keys select a pane, and `tab`
selects the next pane:

| Key | Pane | What it shows |
|---|---|---|
| `1` | Description | The body of the pull request as GitHub renders it, with tables, details and task lists. The text reflows when the pane width changes. The detail opens on this pane. The Description pane starts with a readiness block: the merge verdict and its reasons, the checks, the reviews, the open threads, the size, and the latest activity. When the pull request has codeview comments that did not go to GitHub yet, one dim line below the block counts them, for example `3 unsent codeview comments (1 on an older push) · r to send` (see "Codeview comments" below). Without such comments, the pane shows no line. |
| `2` | Checks | One row per check, with its state and its duration. |
| `3` | Conversation | The comments, the reviews and the review threads, in time order, with the reactions of each comment. `[conversation] order` can put the newest at the top. `r` replies to the comment of the cursor (see "Replies" below). The threads start expanded. `enter` on a thread collapses or expands it. A collapsed thread stays collapsed while the detail is open, also when chaoshub reads the conversation again. `enter` on a comment opens the comment menu: reply, or react (see "Reactions" below). |
| `4` | Files | The changed files as a tree, with the diff of each file inline. See "Diff colors" below. |
| `5` | Stack | The entries of the stack. Only for a stacked pull request. For a pull request without a stack, the tab shows dim, and `5`, `tab` and a click do not select it. The cursor starts on the pull request of the detail. Enter opens the detail of the entry under the cursor on top, and `esc` goes back. `o` opens the page of that entry. |
| `6` | Agent | The pr-agent results: one section per tool. Only for a repo with a `path`. For a repo without a `path`, the tab shows dim, and `6`, `tab` and a click do not select it. |

`j`, `k` and the other motion keys move the cursor of the pane. In the
description pane, they scroll the text. In the conversation pane, the
view follows the cursor by screen rows. When a comment is taller than
the pane, `j` and `k` scroll through it before the cursor moves on.
`D` and `U` scroll the active pane by half its height, apart from the
cursor. Thus you can read a long comment or a long description. A
pane scrolls down to its last full page, not further. When a pane has
more rows than it shows, a thin scrollbar shows at its right edge. The
mouse does not use the scrollbar.

In the detail, only the keys that change nothing on GitHub work
directly: the navigation keys, `R` (refresh), `v` (codeview), `r`
(the Review popup), `m` (the merge confirm popup), `o` (browser), `y`
and `i` (copy). `r` only opens the Review popup. Only a key in the
popup sends the review. `m` only opens the merge confirm popup. Only
the confirm key (`y` or `enter`) in the popup merges. In the
conversation pane, `r` opens the text box of a reply instead. Only
`ctrl-s` in the text box sends the reply to GitHub. `enter` on a
comment opens the comment menu, and only a key of the menu sends a
reaction. The other actions are in the actions menu. Press `space` to
open it on top of the detail, then the key of the action. For example,
`space` then `r` opens the Review popup, also in the conversation pane,
and `space` then `m` opens the merge confirm popup, as `m` does. `esc` closes the
menu, and the detail shows again. The pane keys, the motion keys and
the keys of the active pane come first: in the checks pane, `o` opens
the selected check, not the pull request. The key hint bar shows
`Space actions`.

The actions menu also lists the own actions of the active pane, after
the actions of the pull request: the pr-agent tools of the Agent pane,
and the reply of the conversation pane.
In the menu, an action key wins over
a pane key. Thus in the Agent pane, `space` then `r` opens the Review
popup, and `space` then `R` starts the pr-agent review. The menu of the
Agent pane does not show the `agent` action, because the pane shows
already. To bind a direct key for an action in the detail, see
"Actions".

#### Replies

In the conversation pane, `r` opens a text box for a reply to the
comment of the cursor. The key hint bar shows `r reply`. In the
actions menu, the action key wins: `space` then `r` opens the Review
popup. The menu also shows the reply, without a key. Move the cursor to
it and press `enter` to reply. The place of the reply depends on the
line of the cursor:

| Cursor on | The reply |
|---|---|
| A review thread, or a comment in an expanded thread | Goes into the thread. The text box starts empty. |
| An issue comment or a review | Is a new comment on the pull request. The text box starts with a quote of the author and the first line, for example `> @alice: Looks good.`, then an empty line. A comment without text gives `@alice` instead. |
| Another line, for example `no comments yet` | Is a new comment on the pull request. The text box starts empty. |

The text box has the keys of the text box of the review message:

| Key | What it does |
|---|---|
| `enter` | Adds a new line. |
| `ctrl-e` | Opens the text in `$VISUAL`, else `$EDITOR`, else `nvim`. When the editor quits, its text comes back into the text box. |
| `ctrl-s` | Sends the reply. A text that is empty, or that is still the start text, sends nothing. The status line then says `empty reply, not sent`. |
| `esc` | Closes the text box. The text is lost. |

When the reply went out, the status line says so, for example
`commented on #7` or `replied on src/lib.rs:12`, and chaoshub reads the
conversation again. Then the reply shows in the pane. When GitHub
refuses the reply, the status line shows the error.

#### Reactions

The conversation pane shows the reactions of a comment or a review on
one row under its body, in the order of GitHub, for example
`👍 3  🎉 1`. Your own reactions have the primary color. A comment
without reactions has no such row.

`enter` on an issue comment, a review or a comment in an expanded
thread opens the comment menu. `enter` on a thread header still
collapses or expands the thread. A click on the selected comment also
opens the menu. The `Enter` hint in the key hint bar of the pane
shows what `enter` does on the line of the cursor:

| Line of the cursor | Hint |
|---|---|
| An expanded thread header | `Enter collapse` |
| A collapsed thread header | `Enter expand` |
| An issue comment, a review or a comment in a thread | `Enter menu` |
| A status line, for example `loading the conversation…` or `no comments yet` | No `Enter` hint, because `enter` does nothing there |

The other panes follow the same rule. In the Files pane, the hint is
`Enter collapse` on an open directory or file, and `Enter expand` on a
closed one. In the Stack pane, the hint is `Enter open` on a loaded
entry. The Description, the Checks and the Agent pane show no `Enter`
hint, because `enter` does nothing there.

| Key in the menu | What it does |
|---|---|
| `r` | Opens the text box of a reply, as `r` in the pane (see "Replies"). |
| `1` | 👍 thumbs up |
| `2` | 👎 thumbs down |
| `3` | 😄 laugh |
| `4` | 🎉 hooray |
| `5` | 😕 confused |
| `6` | ❤️ heart |
| `7` | 🚀 rocket |
| `8` | 👀 eyes |
| `c` | Collapses the thread. Only for a comment in a thread. The cursor goes to the thread header. |
| `j`, `k`, `enter` | Move the cursor, and run the item of the cursor. |
| `esc` | Closes the menu. |

A reaction that you already have shows a `✓` in the menu. Its key
removes your reaction again, as on GitHub. Else the key adds your
reaction. The pane shows the change at once, and the reaction pulses
until GitHub shows it (see "Changes on their way to GitHub"). A removed
reaction with no count left shows as `👍 0` while it pulses. When the
reaction went out, the status line says so, for example `reacted 🎉 on
#7` or `removed 👍 on #7`, and chaoshub reads the conversation again.
When GitHub refuses the reaction, the pane shows the old reactions
again, and the status line shows the error, for example `react to a
comment failed: offline`.

#### Diff colors

The Files pane shows the diff of an open file in the style of the
diff view of Pierre ([diffs.com](https://diffs.com)). The colors are
keys of the palette:

| Part | Color | Pierre | ANSI 16 |
|---|---|---|---|
| Open file name | `fg`, bold, on `surface` | `#fafafa` on `#171717` | white, bold |
| Hunk header (`@@ -1,3 +1,4 @@`) | `diff_hunk` on `diff_hunk_bg` | `#7ea6c0` on `#0a1d21` | cyan |
| Added line | `success` on `diff_add_bg` | `#07c480` on `#0a1d16` | green |
| Removed line | `danger` on `diff_del_bg` | `#ff2e3f` on `#230e0f` | red |
| Changed word of an added line | its text color on `diff_add_word_bg` | `#093e2b` | on dark gray |
| Changed word of a removed line | its text color on `diff_del_word_bg` | `#4f1419` | on dark gray |
| Context line | `fg` | `#fafafa` | white |
| Line number | `diff_gutter` | `#737373` | dark gray |

The tint of a line goes from its line number to the right edge of the
pane. The ANSI palette has no light tints, so there the lines keep the
background. Each line starts with its line number: the new number for
an added line and a context line, the old number for a removed line.

A run of removed lines that a run of added lines follows directly is
one change. Its lines pair up by position, as on GitHub: the first
removed line with the first added line, and so on. In each pair, the
words that changed get a stronger tint, as in the Pierre diff view.
The text keeps its color. A word is a run of letters, digits and `_`,
or one other sign. Two changed words that one char divides show as one
mark. A pair gets no marks when one of its lines is longer than 1000
characters, or when less than half of the text of the two lines is
equal. A change gets no marks when one side has more than three times
the lines of the other side. On the cursor line, the selection band
hides the marks.

The code of each line has the syntax colors of its language. chaoshub
finds the language from the file name or the extension, with
[syntect](https://github.com/trishume/syntect) and the syntaxes of
[bat](https://github.com/sharkdp/bat) that
[two-face](https://codeberg.org/CosmicHarper/two-face) bundles. The set
has more than 200 languages, for example Rust, Go, Python, TypeScript
and TSX, JavaScript, TOML, Nix, Dockerfile, YAML, JSON, Markdown and
shell. The `+` and the `-` keep the green and the red of the change. The token
colors are the `syntax` keys of the palette, from the `pierre-dark`
theme:

| Token | Key | Pierre | ANSI 16 |
|---|---|---|---|
| Comment | `syntax.comment` | `#737373` | dark gray |
| String | `syntax.string` | `#5ecc71` | green |
| Number, `true` | `syntax.number` | `#68cdf2` | cyan |
| Other constant | `syntax.constant` | `#ffd452` | yellow |
| Keyword | `syntax.keyword` | `#ff678d` | light red |
| Operator | `syntax.operator` | `#08c0ef` | cyan |
| Function | `syntax.function` | `#9d6afb` | magenta |
| Type | `syntax.type_name` | `#d568ea` | light magenta |
| Namespace | `syntax.namespace` | `#ffab16` | yellow |
| Variable | `syntax.variable` | `#ffa359` | light yellow |
| Parameter | `syntax.parameter` | `#a3a3a3` | gray |
| Punctuation | `syntax.punctuation` | `#636363` | dark gray |

On the ANSI palette, an added or a removed line has no tint. Thus its
code keeps the green or the red of the change, and only the context
lines get the syntax colors.

A background worker gives the syntax colors, so the UI does not wait
for them. When you open a file, its diff shows at once with the diff
colors. The syntax colors come when the worker is done, usually after
some milliseconds. chaoshub keeps the result until the pull request
gets a new head. The worker also finds the changed words. A file gets
no syntax colors when no bundled syntax knows its language. A file
gets no syntax colors and no marks of changed words when its patch is
larger than 128 KiB. A line that is longer than 1000 characters gets
no syntax colors. The diff colors stay.

#### Codeview comments

You write review comments in codeview (`v`). The Review popup lists the
comments that did not go to GitHub yet, and sends the marked comments
as one review. The Review popup is the only way to approve a pull
request, to request changes, and to send codeview comments. chaoshub
reads the comment files of codeview (`[codeview] comments_dir`) and
never writes them.

Press `r` to open the Review popup. This works in the list and in the
pull request detail. In the conversation pane, `r` is the reply, so
there press `space`, then `r`. `space`, then `r`, works in every place. The popup opens at once with the
comments that chaoshub has, and reads the comment files again. When the
read ends, the popup shows the new comments. The popup also opens
without comments, and for a repo without a `path`. When the read
fails, the popup shows the error in the list, and approve still works.

The comment list is on top. Each row shows the cursor (`>`), the mark
(`[x]` marked, `[ ]` not marked), the place (`path:L12-14 (new)`, or
`PR` for a comment on the pull request document), and the first line of
the comment. A `PR` comment goes into the text of the review, after
your message. Every comment starts marked, also a new comment. GitHub
takes no comment on a commit message, so such a row shows dim with
`[-]`, and the cursor skips it.

A comment that you wrote before the last push of the head is on an
older push. Its row comes after the comments of the head, with the tag
`older push`. It starts marked too, and goes out with the review.
codeview posts the comments of each older push first, in a comment
review of their own on the head commit of that push. So each comment
stays on the lines that you wrote it for. GitHub shows it as outdated
when a later push changed these lines.

Below the list, the keys are on the left, and the message is on the
right. The message wraps in its column. The popup shows up to 4 rows
of it, then `… N more lines`.

| Key | What it does |
|---|---|
| `j` / `k` | Moves the cursor down or up. The other motion keys also work, and the list scrolls. |
| `enter` | Marks or unmarks the comment of the cursor. |
| `m` | Opens the text box of the message. |
| `c` | Sends a review that adds comments only. |
| `a` | Sends a review that approves the pull request. |
| `r` | Sends a review that requests changes. |
| `esc` | Closes the popup. The marks and the message stay. |

`c`, `a` and `r` send at once, with no other question. A key that
cannot send now shows dim, with the reason:

- `r` needs a message.
- `c` needs a message when no comment is marked.
- `a` and `r` show `your own PR` on your own pull request, and `not open`
  on a closed or merged pull request. GitHub takes no such review. `c`
  still works.

With no marked comment, the popup still works: the review has only the
message. chaoshub then posts the review itself.

The marks and the message stay for each pull request until chaoshub
quits. They stay when you close the popup or the detail, and the popup
in the list shows the same marks as the popup in the detail. A review
that went out clears the message.

When you approve a pull request that requests your review, the pull
request leaves the Review tab at once. chaoshub does not wait for
GitHub. GitHub then removes the review request, so the pull request
moves to the Team tab, and chaoshub shows it there at once too. The
title count of the Review tab goes down at once. The notification does
not announce the pull request in the Team tab. If the approval fails,
the pull request comes back, and the status bar shows the error. The
change is local only. It ends when GitHub data shows your approval
and no review request of you. A read that comes before GitHub has the
approval does not end it. If GitHub still requests your review 60 s
after the approval, the pull request comes back to the Review tab:
GitHub decides. An approval in the detail pane (`chaoshub pr`) hides
the pull request in the list at once too (see "How the PR view talks
to the list"). The Description pane of the PR view shows your review
at once after the `REVIEWS` label, for example `✓ you`, until GitHub
shows it.

The text box of the message can have more than one line, and its line
breaks go to GitHub:

| Key | What it does |
|---|---|
| `enter` | Adds a new line. |
| `ctrl-e` | Opens the text in `$VISUAL`, else `$EDITOR`, else `nvim`. chaoshub leaves its screen while the editor runs. When the editor quits, its text comes back into the text box. |
| `ctrl-s` / `esc` | Goes back to the Review popup, with the text. |

When a marked comment went out from codeview in the meantime, the send
leaves it out. When every marked comment went out already, the review
goes out with only the message, so an approval does not stay behind. A
comment review without a message then has nothing to post. codeview
also finds a comment that GitHub holds already, for example after a
send that stopped before codeview wrote its sent marks, and it does not
post that comment again. When codeview can post none of the marked comments, for
example because no comment maps to the diff, it sends no review. The
status line then says `review not sent`, and names the
comments that stay local. When the review went out, but some marked
comments stay local, the status line shows a warning that names them
with the reason. When GitHub refuses the review of an older push, for
example after a force push, the review does not go out, and the status
line shows the error. Unmark the comments of that push to send the
review.

### Actions

The actions work on the pull request of the selected row, or on the
pull request of the detail. When an action cannot run, the status line
says why.

In the list, only the action keys that change nothing on GitHub work
directly on the selected row: `v` (codeview), `r` (the Review popup),
`m` (the merge confirm popup), `o` (browser), `y` and `i` (copy). `r`
only opens the Review popup, and only a key in the popup sends the
review. `m` only opens the merge confirm popup, and only the confirm
key in the popup merges. The other action keys do nothing in the list.
Thus a wrong key cannot send a review or merge. A row without a pull request, for example a
stack header, runs no action. The key hint bar shows the direct keys
that the row can run. The `actions` key (`space`) opens the actions
menu of the selected row. The menu shows each action for the row, with
its key. A disabled action shows the reason in parentheses. In the
menu:

- `j` and `k` move the cursor.
- `enter` runs the action of the cursor.
- The key of an action runs that action. For example, `space` then `r`
  opens the Review popup.
- `esc` or `q` closes the menu.

The action keys work in the actions menu. In the pull request detail
(the sidebar and the Rex pane), `space` opens the same menu for the
pull request of the detail. There, as in the list, only `v`, `r`, `m`,
`o`, `y` and `i` also work directly. To run another action directly in the list
and in the detail, bind a key to `detail_<id>` in `[keys]`. For
example, `detail_close = "c"` makes `c` open the close confirm popup of
the selected row or of the detail. These bindings have no default.

| Binding | Key | What it does |
|---|---|---|
| `review` | `v` | Reviews the pull request in codeview. Needs a repo with a `path`. A second press focuses the open pane. |
| `merge` | `m` | Merges the pull request after a confirm popup. `m` opens the popup also directly in the list and in the detail, and only the confirm key in the popup merges. For a stacked pull request, see the stack merge paragraph below. |
| `merge_stack` | `s` | Merges the whole stack of the pull request after a confirm popup, from any entry of the stack. It is the same merge as `merge` on the top entry. Hidden for a pull request without a stack. |
| `rerun_checks` | `x` | Runs the failed checks again. |
| `draft` | `d` | Changes the pull request from draft to ready, or from ready to draft. The actions menu shows "mark ready" for a draft and "convert to draft" for a ready pull request. |
| `close` | `c` | Closes the pull request after a confirm popup. |
| `browser` | `o` | Opens the pull request in the browser. |
| `copy_url` | `y` | Copies the pull request URL. |
| `copy_stack_urls` | `Y` | Copies the URLs of the open pull requests of the stack, one URL on each line. The order is the order of the Stack pane: position 1, the bottom of the stack, first. `[actions.copy_stack_urls] order = "top_first"` puts the top of the stack first, as the list does (see `[actions.<id>]`). It works only in the actions menu (`space` then `Y`). Hidden for a pull request without a stack. When an open entry of the stack is not loaded, the status line shows "stack not loaded, refreshing…". Try again after the refresh. |
| `copy_change_id` | `i` | Copies the jj change id of the local change that matches the pull request. In a git repo, it copies the local branch name, the actions menu shows "copy branch name", and the key hint bar shows "copy branch". Hidden when no local change matches. |
| `agent` | `p` | Opens the detail on the Agent pane. Needs a repo with a `path`. |
| `codeview_send` | `r` | Opens the Review popup, also directly in the list and in the detail. In the conversation pane, `r` is the reply, so use `space` then `r` there. In the popup, you mark the codeview comments, write the message, and comment, approve or request changes (see "Codeview comments"). The hint bar, the help and the actions menu show it as "send review". |
| `reply` | `r` | Only in the conversation pane of the pull request detail: replies to the comment of the cursor (see "Replies"). The actions menu shows it only as an action of the conversation pane, without a key, because `r` there opens the Review popup. |
| `react` | none | Only in the comment menu of the conversation pane: adds or removes your reaction on the comment (see "Reactions"). It has no key of its own, and the actions menu does not show it. |

A merge of a stacked pull request includes every entry up to it. A
merged entry needs no merge, so the merge skips it. GitHub tells
chaoshub which entries merged with each pull request of the stack, so a
merged entry does not have to be in the cache. When an entry is not
loaded, the status line shows "stack not loaded, refreshing…", and
chaoshub reads the stack again. Try the merge again after
the refresh. When the entries below an entry merged, but GitHub did not
move the entry to the base of the stack yet, the merge waits for the
move, up to 2 min. To merge every entry, use `merge_stack` (`s`) from
any entry. The `[merge] stack` key sets how chaoshub sends the merge.
In `auto` mode, chaoshub reads the squash defaults of the repo. When they are the PR
title and the PR body, or when the method is `merge` or `rebase`,
chaoshub sends one atomic request. Else chaoshub merges each entry
itself, bottom first, each with its own title and body. Thus every
squash commit carries its PR body. The confirm popup says which way
chaoshub merges.

### Changes on their way to GitHub

An action that changes GitHub, for example an approval, a merge or a
reaction, waits for GitHub twice: for the result of the request, and
for GitHub data that shows the change. In this time, chaoshub pulses
the item that changes, in the colors of the merge badge: green to purple
and back. The pulse uses the tick of the status bar spinner and moves
only while an item pulses.

| Action | What pulses |
|---|---|
| Approve, request changes, comment review (`r`) | The title of the row in the list, and `REVIEWS` with your review (`✓ you`, `✗ you`, `you commented`) in the Description pane. |
| Review in codeview (`v`) | When the pane ends: the row and `REVIEWS`, until GitHub shows a change of your review or of the review request. |
| Merge (`m`), stack merge (`s`) | The row of each entry that the merge merges, in the list and in the Stack pane, and the badge of the Description pane. |
| Close (`c`), draft (`d`) | The row and the badge of the Description pane. |
| Rerun the checks (`x`) | The row, `CHECKS` in the Description pane, and each failed check in the Checks pane. |
| Reply (`r`) | The row, and the thread header in the Conversation pane, or a `sending your comment…` line at the end. With `[conversation] order = "newest_first"`, the line is at the top. |
| Reaction | The row, and the reaction in the Conversation pane. |

The header of the pull request popup is plain text. It cannot pulse,
so it shows the spinner before the state instead, for example
`⠋ draft`, during a merge, a close or a draft change.

The item stops when the GitHub data shows the change: for an approval,
your latest review is an approval and GitHub requests no review of you;
for a merge, the pull request is merged, closed or in the merge queue;
for a reaction, you have it or not. Data that does not show the change
yet keeps the pulse. After the result, chaoshub reads the pull request
again after 2 s, 7 s, 22 s, 37 s and 52 s, until the change shows and
GitHub knows the merge state. If the change does not show 60 s after
the result, the pulse stops, and the GitHub data decides. If no result
comes in 30 min, the pulse stops too.

When the action fails, the pulse stops at once, the item shows the
GitHub data again, and the status line shows the error.

GitHub computes the merge state again after many changes. While it does,
the badge in the Description pane keeps the last known state, pulses
for up to 60 s and says `computing the merge state`. The badge shows
CHECKING only when no state is known.

### Mouse

chaoshub reads the mouse. Set `[terminal] mouse = false` to give the
mouse back to the terminal, for example to select text across panes.
While a command runs in the foreground (for example codeview without
Rex), chaoshub does not read the mouse.

The wheel moves three rows per step:

- On the list, it moves the cursor.
- In the sidebar, it scrolls the description pane, and moves the
  cursor of the other panes.
- In the help overlay and the output popup, it scrolls the text.
- In the Review popup, it moves the cursor of the comment list.

A click acts on what it hits:

| Click on | What it does |
|---|---|
| A tab in the tab bar | Selects the tab. |
| A list row | Selects the row. A click on the selected row opens the sidebar. |
| A key hint | Presses the key. For example, `Space actions` in the list or in the sidebar opens the actions menu. |
| A pane tab in the sidebar | Selects the pane. |
| A row in a pane | Moves the cursor of the pane to the row. In the conversation pane, a click on the selected thread expands it or collapses it, and a click on the selected comment opens the comment menu. In the stack pane, a click on the selected entry opens its detail. |
| An author in the author picker | Picks the author. |
| An action in the actions menu | Runs the action. |
| A comment in the Review popup | Marks or unmarks the comment. |
| A cell outside of the sidebar or the overlay | Closes it. For a confirm popup, this cancels the action. |

A drag with the left button selects text. The text areas are the body
of the sidebar and the detail (all panes), the body of the help overlay
and the body of the output popup:

1. Press the left button in a text area. The press is also a click.
2. Drag. The selection goes in reading order, as in a terminal: from
   the start cell to the end of its row, then full rows, then to the end
   cell. The marked cells show reversed.
3. Release the button. chaoshub copies the marked text to the
   clipboard, and the status line says `copied N lines`.

The mark stays on the screen until the next press, key, wheel step or
resize. A press and a release without a drag is a click only, and
copies nothing.

The selection has these limits:

- It stays inside the text area where it started. A drag out of the
  area stops at the edge of the area.
- It does not scroll the area. To copy more text than the area shows,
  copy it in parts.
- The copy comes from the drawn cells. Thus it has no borders and no
  text of the panes beside it. A row that the pane wrapped stays as two
  lines. In the conversation pane, the tree marks and the indent are in
  the copy.

With `[terminal] mouse = false`, chaoshub does not select text. The
terminal selects the text.

## pr-agent

chaoshub runs [pr-agent](https://github.com/qodo-ai/pr-agent) on a pull
request and keeps the result on your disk. codeview shows the result in
the diff. pr-agent runs with `publish_output=false`, so it writes nothing
to GitHub. A result goes to GitHub only when you take it as your own: in
codeview, you adopt a suggestion as a local comment and submit it, or in
chaoshub, you apply a describe result with `space`, `p` and `y`.

pr-agent must be installed, or `[agent] command` must start it (for
example with `uvx`). pr-agent runs in the local checkout, so the repo
needs a `path`.

### The Agent pane

`space` then `p` on a pull request opens the detail on the Agent pane
(`6`). The pane has one section per tool. Each section shows the state (`not run`,
`running` with a spinner, `done 5m ago`, or `failed` with the error), the
model and the head commit of the result. When the head of the pull
request moved after the run, the section says
`outdated: ran on 3a1f2c0, head is now 9bd2e11`. chaoshub does not run
pr-agent again by itself.

The pane has no direct keys. Its tools are in the actions menu: press
`space`, then the key of the tool.

| Key | Tool | What it shows |
|---|---|---|
| `R` | review | The fields of the review and the key issues, with file and lines. |
| `I` | improve | One line per suggestion: `label · file:line · summary`. |
| `D` | describe | The new title and the new body. |
| `a` | ask | Asks for a question first. Inside Rex, the question can open in the Rex palette (see "Questions in the Rex palette"). Shows the questions and the answers, newest first. |
| `p` | | Opens a confirm popup with the line diff of the old and the new title and body. `y` sets the title and the body of the pull request on GitHub. |

One run per tool per pull request runs at a time. A second `space`
then `R` while the review runs only says `pr-agent review already running`. Different tools, and
different pull requests, run at the same time.

In the menu, the action keys win over the pane keys. Thus a tool uses
an uppercase key when an action uses the lowercase letter: `r` opens
the Review popup, `i` copies the change id, and `d` changes the draft
state. `p` applies the describe result, because the menu of the Agent
pane does not show the `agent` action.

While a run goes, the status bar shows the spinner and one item per run,
for example `pr-agent review #7 · pr-agent ask #7`. The last output line of
pr-agent shows in the status line. At the end, the status line shows
`pr-agent review done (12k tokens)`, or the error.

### What chaoshub runs

For `review`, `improve` and `describe`, chaoshub runs this command in
the repo path:

```sh
<command> --pr_url https://github.com/<owner>/<repo>/pull/<n> --json-output <tmp file> <tool>
```

pr-agent reads its settings from env variables in the `SECTION__KEY`
form. chaoshub sets these, after the `[agent] env` table:

| Variable | Value |
|---|---|
| `GITHUB__USER_TOKEN` | The output of `gh auth token`. |
| `CONFIG__MODEL` | `[agent] model`. |
| `CONFIG__FALLBACK_MODELS` | `[agent] fallback_models`. |
| `CONFIG__PUBLISH_OUTPUT` | `false`: pr-agent writes nothing to the pull request. |
| `CONFIG__PROPAGATE_TOOL_ERRORS` | `true`: a failed tool exits with an error code. |
| `PR_REVIEWER__EXTRA_INSTRUCTIONS`, `PR_CODE_SUGGESTIONS__EXTRA_INSTRUCTIONS`, `PR_DESCRIPTION__EXTRA_INSTRUCTIONS`, `PR_QUESTIONS__EXTRA_INSTRUCTIONS` | `[agent] extra_instructions` of the tool, when it is not empty. |

pr-agent reads env values as TOML, so chaoshub writes the texts in
quotes and the model list as a TOML list. A name that chaoshub sets
replaces the same name in `[agent] env`. pr-agent does not accept
`config.fallback_models` as a command argument, so the env carries all
settings.

`--json-output` does not work for `ask`, and with
`publish_output=false`, pr-agent prints the answer nowhere. So for `ask`,
chaoshub writes the Python script
`~/.local/state/chaoshub/chaoshub-agent-ask` and runs it with the Python
of the pr-agent command:

```sh
<python> ~/.local/state/chaoshub/chaoshub-agent-ask https://github.com/<owner>/<repo>/pull/<n> "<question>"
```

The script runs the ask tool in its own settings scope, as the pr-agent
CLI does, and prints the answer. chaoshub finds the Python this way:

- `command` has one word, for example `["pr-agent"]` from pip or pipx:
  chaoshub finds the program in `PATH` and reads its first line. A
  `#!/path/to/python` line gives that Python, and
  `#!/usr/bin/env python3` gives `/usr/bin/env python3`. A `#!/bin/sh`
  line with an `'''exec' /path/to/python` second line (the pip form for
  long paths) gives that Python. Any other first line is an error.
- `command` has more than one word, for example
  `["uvx", "--from", "pr-agent", "pr-agent"]`: chaoshub puts `python` in
  place of the last word. So `uvx --from pr-agent python <script>` runs
  the script in the env of the tool.
- `docker run ...` does not work for `ask`: the last word is the image,
  and the container cannot see the script. `review`, `improve` and
  `describe` work with Docker when the container gets the env names
  with `-e` and the state directory at the same path (see `[agent]`).

At `[agent] timeout_secs`, chaoshub stops the process. A run that fails
keeps the old result file.

### The result files

The results are in `~/.local/state/chaoshub/agent/<owner>/<repo>/<n>/`
(`$XDG_STATE_HOME/chaoshub/agent/...`), one file per tool:
`review.json`, `improve.json`, `describe.json` and `ask.json`. chaoshub
writes a file to a temporary name first and then renames it, so a
reader never sees a part of a file. codeview reads the same files.

Each file holds one JSON object:

| Field | Meaning |
|---|---|
| `version` | `1`. A reader refuses other versions. |
| `tool` | `review`, `improve`, `describe` or `ask`. |
| `pr` | The number of the pull request. |
| `head_sha` | The head commit of the pull request when the run started. |
| `model` | The LiteLLM name of the model. |
| `started_at`, `finished_at` | The times of the run, in RFC 3339. |
| `usage` | The token counts of pr-agent (`prompt_tokens`, `completion_tokens`, `total_tokens`), or `{}`. |
| `data` | The JSON of pr-agent. For `ask`, a list of `{question, answer, head_sha, at}`, newest first. |

The `data` of pr-agent:

- review: `{review: {key_issues_to_review: [{relevant_file, issue_header, issue_content, start_line, end_line}], ...}, usage, coverage}`.
- improve: `{code_suggestions: [{relevant_file, relevant_lines_start, relevant_lines_end, existing_code, improved_code, suggestion_content, one_sentence_summary, label, score}], usage}`.
- describe: `{title, type, description, changes_diagram, pr_files: [{filename, changes_title, changes_summary, label}], usage}`.

The JSON has no markdown. chaoshub makes the text of the review from the
fields. The body that `p` applies is the `description`, then a `Files`
list with one line per entry of `pr_files`.

`v` gives codeview the root of the files in `CHAOSHUB_AGENT_ROOT`.

## Rex, Ghostty and codeview

### What `v` runs

The review action runs nvim in the local path of the repo:

```sh
nvim -c "autocmd User CodeViewSessionClosed ++once qall" -c "Codeview pr <N>"
```

`Codeview pr <N>` opens the pull request in codeview. When you close the
codeview session, codeview sends `User CodeViewSessionClosed`, and nvim
quits. nvim gets `CHAOSHUB_AGENT_ROOT=<state dir>/agent`, so codeview
finds the pr-agent results.

nvim also gets `CODEVIEW_PR`: the pull request data that chaoshub has,
as the JSON of `gh pr view --json` (number, title, state, draft, url,
author, branches, head and base commits, head repo and body). codeview
then shows the review without a wait for `gh pr view`, and checks the
data with gh in the background. When the head moved, codeview reads the
review again and shows a short message. An older codeview ignores the
variable. chaoshub leaves the variable out when it does not know the
base commit yet, or when the data is longer than 100 KiB.

Inside Rex, chaoshub opens a pane beside its own block with
`rex split -b $REX_BLOCK --split=<split> [--ratio <ratio>] -c <repo path>
--label "review #<N> <title>" --json -- nvim ...`. Rex shows the label
in the header of the block. A title of more than 30 chars is cut and
ends with `…`. With `zoom`, it runs `rex zoom -b <block> --on`. It waits
for the pane with `rex wait <block>`. The Rex binary is
`$REX_BIN_DIR/rex` when that file exists, else `rex` on your `PATH`.

Outside Rex, or with `launcher = "inline"`, chaoshub leaves its screen,
runs nvim in the same terminal, and comes back when nvim quits. No
transition runs before or after nvim.

When the review ends, chaoshub syncs the pull request again and scans
the repo again.

### What the Review popup runs

The send of codeview comments runs codeview in a headless nvim, in the
local path of the repo:

```sh
nvim --headless -c "lua require('codeview.bridge').submit_file('<request>')"
```

First chaoshub reads the comment files again, and leaves out each
comment that went out from codeview in the meantime. When no comment is
left, it runs nothing, and the status line says that the comments went
out from codeview already.

`<request>` is a JSON file in `<state dir>/codeview/`:

```json
{ "version": 2, "repo": "owner/name", "number": 14,
  "pushes": [{ "head": "<sha>", "ids": ["..."] }],
  "event": "comment", "body": "...", "out": "<result>" }
```

`pushes` holds the marked comments per push of the head. `event` is
`comment`, `approve` or `request-changes`. codeview opens the pull
request, maps the comments to the lines of the diff of their push,
posts one comment review for each older push and then the review with
the event, marks the comments as sent in its comment file, and writes
the result to `<result>`. A codeview that knows only version 1 refuses
the request. chaoshub then sends the comments of one push again with
version 1 (`"head"` and `"ids"` in place of `"pushes"`). For comments
of more than one push, the send then fails, and the status line asks
you to update codeview. nvim gets `CHAOSHUB_AGENT_ROOT=<state dir>/agent`,
like the review of `v`. chaoshub stops nvim after 10 minutes. codeview
gives the post of a review 2 minutes, so a stop during the post is
rare.

chaoshub never posts these comments to GitHub itself. Only codeview
posts, so its lock file and its sent marks stop a second send of the
same comment, also from an open codeview. The status line then says
`approved #N with 2 comments` (or `commented on #N …`, `requested
changes on #N …`), with the count of the comments
on an older push, and names the comments that stay local and the
comments that went out already. After the send,
chaoshub reads the comment files again. See `:help codeview-bridge` in
nvim for the request and the result.

### What `enter` runs inside Rex

The pull request detail runs chaoshub again, in a Rex pane beside the
list:

```sh
rex split -b $REX_BLOCK --split=<[detail] split> [--ratio <[detail] ratio>] \
  -c <repo path> --label "<PR overview>" --json -- \
  env CHAOSHUB_REX_LABEL=1 CHAOSHUB_LIST_SOCKET=<socket path> \
  <chaoshub> --config <config path> --no-prefetch [--theme <style>] \
  [--palette <name>] [--repo <owner/name> ...] pr <owner/name> <N>
```

The label is the PR overview of the terminal title of the PR view (see
`[terminal]`), for example `#7 Fix the merge · ready · CI passed ·
approved`. Rex shows it in the header of the block. With
`CHAOSHUB_REX_LABEL=1`, the PR view keeps the label current: when the
overview changes, it runs `rex block rename [-s <session>] --
$REX_BLOCK "<PR overview>"`. It runs one rename at a time, always with
the newest overview. Before the pull request loads, the label of the
launch stays. A PR view that you start yourself does not rename its
block.

Because the header of the block shows the overview, the PR view with
`CHAOSHUB_REX_LABEL=1` has no title row. Its first row is the author
and the branches, and the panes get the row. The label cuts a title of
more than 30 chars. For such a title, the Description pane starts with
the full title. A PR view outside Rex, or one that you start yourself,
keeps its title row.

The pane gets the configuration file and the `--theme`, `--palette` and
`--repo` options of the list. It always gets `--no-prefetch`, because
the list already prefetches the pull request of its cursor row. A repo
without a `path` runs in the directory of the list. chaoshub does not
zoom the pane. It waits for the pane with `rex wait <block>`. The
`CHAOSHUB_LIST_SOCKET` env is there only when the list listens on its
socket.

### Questions in the Rex palette

Inside Rex, chaoshub can ask a question in the command palette of Rex
in place of its own popup. It does this for these questions:

- The author picker (`a`). Each author has the `person` icon, and
  `all authors` comes first. The current choice shows `current` at the
  end of its row. The submit row says `Filter`.
- The one-line prompts: the question to pr-agent (`a` in the Agent
  pane), and the text prompt of an action. The palette shows a text
  field with the title and the placeholder of the prompt. Your text
  goes to the same place as the text of the prompt popup. The submit
  row says `Ask` for pr-agent, and `Send` for an action.
- The jump to a pull request (`f` in the list). The palette lists the
  pull requests that the tabs show now, in tab order: the filter, the
  author picker and the ready rule of each tab apply. A pull request in
  two tabs shows once, with its first tab. Each row has `#N <title>`,
  the repo and the author, and the status, as in `chaoshub: open PR…`.
  The icon shows the tab: `branch` for Mine, `person` for Review and
  `code` for Team. The list holds at most 2,000 pull requests, the
  limit of `rex.ui.pick`. The submit row says `Open`. When you pick a
  pull request, chaoshub selects its tab and its row, and opens it as
  `enter` does: inside Rex, in the detail pane. When Rex cannot ask,
  chaoshub opens the filter of the active tab (`/`).

Rex shows the question in each Rex app that is connected to the Rex
server, also on other devices. The first answer counts.

`rex.ui` works only in an action of the `init.lua` of the Rex server.
An action of a `rex do` script gets `unsupported`. For this reason,
[`contrib/rex/chaoshub.lua`](contrib/rex/chaoshub.lua) registers the
action `chaoshub_ask`. Load the file from your `init.lua` (see "Open a
pull request from the Rex palette"). Without the action, chaoshub opens
its own popups.

chaoshub uses the palette only when all these conditions are true:

- chaoshub runs inside Rex (`REX_BLOCK` is set).
- At start, `rex api call server.version` gives `api_revision` 17 or
  more. Revision 17 added `rex.ui`.
- At start, `rex actions --json` has the action `chaoshub_ask`.
- At start, `rex client ls --json` shows a connected client of the
  kind `app`.

To ask, chaoshub runs the action:

```sh
rex --timeout 330s do chaoshub_ask --args - --json
```

The question goes to the action as JSON on stdin, for example
`{"ask": {"kind": "pick", "title": "author", "confirm": "Filter",
"items": [...]}, "timeout": 300}`. The fields of `ask` are the fields
of `rex.ui.input` and `rex.ui.pick`: `title`, `placeholder`, `value`
and `confirm`, and for a pick `items` and `allow_other`. The action
gives `{"answer": "..."}`, `{"other": "..."}` (a typed text of a pick
with `allow_other`) or `{"reason": "..."}`. No question of chaoshub
sets `allow_other` now. Rex shows the question for 5 minutes. The rex
binary is the same one as for the panes. After a change of `init.lua`,
restart chaoshub, because it checks only at start.

While the question waits, the status bar shows
`waiting for Rex`. chaoshub then ignores the mouse and
its keys, so no second question can start. `esc` or `q` stops the
wait, and `ctrl-c` quits.

- When you answer, chaoshub does what its popup does with that answer.
- When you close the question (`cancelled`), or when you do not answer
  in time (`timeout`), chaoshub does nothing.
- When Rex cannot ask (`no_client`, `unsupported`, `busy` or `failed`),
  or when the rex command fails, chaoshub opens its own popup.

### How the PR view talks to the list

The PR view in the detail pane is a separate chaoshub process. It
tells the list process about its changes through a local socket, so
the list does not wait for the next sync.

- At start, the list opens a Unix datagram socket, `list-<pid>.sock`.
  The socket is in `$XDG_RUNTIME_DIR/chaoshub`, or in
  `~/.local/state/chaoshub/ipc` when `XDG_RUNTIME_DIR` is not set (for
  example on macOS).
- The list gives the path to each PR view that it starts, in the env
  `CHAOSHUB_LIST_SOCKET`.
- For each event, the PR view sends one small JSON message, for
  example `{"v":2,"event":"pr_changed","pr":{...}}`. A message of
  another version is dropped.
- The list deletes its socket when it quits. At start, it also deletes
  the sockets of list processes that do not run now.

The PR view sends these events:

| Event | When | What the list does |
|---|---|---|
| `pending_started` | The PR view starts an action that changes GitHub, for example an approval, a merge or a reaction. The message holds the action id and its parts (see "Changes on their way to GitHub"). | The list keeps the parts with the same rules as its own: the rows pulse, and an approval hides the pull request from the Review tab at once. |
| `pending_done` | That action ends. `ok` is false when it failed or was cancelled. | A failure ends the parts at once, so an approved pull request comes back. A success keeps the parts until GitHub shows the change. |
| `pr_changed` | A review, a reply, a merge, a close, a draft change or a rerun of the checks succeeds, or a codeview review pane ends. | The list reads the pull request from GitHub again. |

If the pane of the PR view ends before the end of an action comes, the
action ends as a success at that time, and the GitHub data or the time
limit decides.

The channel is for the local user only:

- The socket directory has mode 0700, and the socket has mode 0600.
  Other users cannot open it.
- The socket is a local Unix socket. It uses no network.
- The PR view never waits for the list. When no list listens, the
  message goes nowhere, and the PR view works as before.
- A message that does not decode is dropped.

Each list process has its own socket, so a PR view talks only to the
list that started it. A PR view that you start yourself
(`chaoshub pr OWNER/NAME N` in a shell) has no list socket. Its changes
show in a list after the next sync.

### Open a pull request from the Rex palette

[`contrib/rex/chaoshub.lua`](contrib/rex/chaoshub.lua) adds the Rex
action `chaoshub: open PR…` to the command palette. It works from any
block, also when no chaoshub list runs. The file also adds the action
`chaoshub_ask` (`chaoshub: TUI question (internal)`). The chaoshub
TUI uses it to ask its questions in the palette (see "Questions in the
Rex palette"). Started from the palette, it has no question and does
nothing.

1. The action runs `chaoshub list --json` (see "List the pull
   requests"). It reads only the cache, so the list is as old as the
   last sync of the TUI.
2. It shows the pull requests with `rex.ui.pick`. Each row has
   `#N <title>`, the repo and the author, and the status. The icon shows
   the tab: `branch` for Mine, `person` for Review and `code` for Team.
3. When you pick a pull request, the action opens a new Rex window
   with the label `<name>#N`. The window runs
   `env CHAOSHUB_REX_LABEL=1 chaoshub [--config <path>] pr <owner/name> <N>`
   in the `path` of the repo, or in the home directory for a repo
   without a path. The block label is the PR overview, and the PR view
   keeps it current.

If you press `esc`, or nobody answers before the time limit, the action
stops and opens nothing. If the cache has no pull requests, the list
shows one row that opens nothing.

To install the action, load the file from `~/.config/rex/init.lua`:

```lua
dofile("/path/to/chaoshub/contrib/rex/chaoshub.lua").setup()
```

`setup` takes a table with two optional fields:

| Field | What it does |
|---|---|
| `bin` | The chaoshub program. The default is `chaoshub`. The Rex server runs `chaoshub list` without a shell, so the program must be on the `PATH` of the server, or `bin` must be a full path. |
| `config` | The configuration file. The action gives it to `chaoshub list` and `chaoshub pr` as `--config`. |

The action name is `chaoshub_open_pr`. To bind a key to it, add a line
such as `rex.bind("cmd+shift+o", "chaoshub_open_pr")`. Then run
`rex config check` and `rex config reload`.

### Terminal status: OSC 7501 and OSC 9;4

chaoshub tells the terminal what it does, so a Rex or Ghostty tab shows
the state without a look at the pane. OSC 7501 is the program status,
and OSC 9;4 is the progress bar. Every OSC 7501 record has
`app=chaoshub`.

| Situation | OSC 7501 | OSC 9;4 |
|---|---|---|
| Idle | `state=idle` | off |
| First sync or `R` sync | `state=working`, `syncing <n> repos` (`syncing 1 repo`) | indeterminate |
| Prefetch of more than one ref | `state=working`, `prefetching` | percent done |
| Confirm or prompt popup open | `state=blocked`, `kind=question`, the popup title | unchanged |
| Merge pending | `state=working`, `merging #N` | indeterminate |
| Merge in the merge queue | `state=working`, `#N in merge queue` | indeterminate |
| Merged | `state=done`, `merged #N` | 100 for 2 s, then off |
| Merge failed, sync failed, worker died | `state=error`, the reason | error for 5 s, then off |
| Rate limited | `state=error`, `rate limited until HH:MM` | paused |
| Review pane open | `state=idle` | unchanged |
| Review in the same terminal while a notice shows | `state=idle` | off |
| Quit, review in the same terminal, crash | `state=clear` | off |

The `done` and `error` states clear on the next key press or the next
successful sync.

### Notice of new pull requests

chaoshub tells you when new pull requests are ready for review. The
terminal shows a notice, for example `3 team PRs ready for review`.
Rex shows the notice as a dot and a message on the tab, and as a
desktop notification with the text.

A pull request is new when it becomes ready in a watched tab (`[notify]
tabs`) for the first time. Ready means open, not a draft, CI passed, and
in the `max_age` limit. These changes make a pull request new:

- A colleague opens it.
- It leaves draft.
- Its CI turns green.

The author filter, the `/` filter and the `t` and `T` keys do not change
which pull requests are new. When chaoshub starts, the cache and the
first sync only give the start state, so they send no notice.
chaoshub keeps the announced pull requests in the cache. A restart
does not announce them again.

Check runs do not change the update time of a pull request, so the
list sync does not see CI that turns green. For this reason, the CI poll
also reads the CI of the watched pull requests that are not green yet,
at most 20 in each poll (`[refresh] ci_secs`).

The notice waits for `[notify] debounce` (15 s by default) after a new
pull request. Each new pull request starts the wait again, so one
notice tells you about all of them. A steady stream gets a notice at
the latest 5 debounce times after the first new pull request. The text
of the notice starts with the reason for the notice. A desktop
notification can cut a long text, but the reason stays. For one pull
request, the text is one of these:

- `Your PR ready for review: #123 <title>` for the Mine tab.
- `Ready for your review: #123 <title> (<author>)` for the Review tab.
- `Team PR ready for review: #123 <title> (<author>)` for the Team tab.

For more pull requests, the text gives the count of each tab, in tab
bar order:

- `2 of your PRs ready for review` for the Mine tab.
- `3 PRs ready for your review` for the Review tab.
- `3 team PRs ready for review` for the Team tab.
- `2 PRs ready for your review · 1 team PR ready for review` for pull
  requests of more tabs.

The status bar shows the same text. The notice stays until the next key
press in chaoshub. A quit removes it too.

chaoshub sends the notice with OSC 7501 as its own record, so the
program status stays: `state=done`, `id=new-prs`, `app=chaoshub`, the
title `chaoshub` and the text as `msg`. When a new notice comes while
the last one shows, chaoshub first removes the old record
(`state=clear`, `id=new-prs`), so the terminal sees a new record.
Ghostty shows OSC 7501 too.

With `[notify] desktop` (on by default), chaoshub also sends each new
notice as an OSC 777 desktop notification:
`ESC ] 777 ; notify ; chaoshub ; <text> ESC \`. A program status record
gives only a tab dot in Rex, and a text notification only while Rex is
in the background. OSC 777 gives the text also while Rex is in front.
In Rex, the setting "Terminal notifications" (Settings > Notifications
> System Notifications) must be on. Ghostty shows OSC 777 too. chaoshub
does not send OSC 9, because OSC 9 has the same number as the OSC 9;4
progress bar. chaoshub does not send OSC 99.

While Rex is in the background, Rex can also send a macOS notification
for the program status record, so you can get two notifications for one
notice. To get only the one from OSC 777, turn off the system
notifications of the Done state in Rex (Settings > Notifications >
Program Activity > Done). The tab dot stays. To get only the tab dot,
set `desktop = false`.

chaoshub does not support tmux for the notice yet. tmux does not pass
OSC 7501 and OSC 777 to the terminal, and chaoshub does not wrap the
sequences for tmux. Inside tmux, the notice does not show.

### Status bar

The status bar keeps three states until they end, also after a key
press:

- `rate limited until HH:MM`, in the warning color, until the reset
  time.
- `offline`, in the danger color, until the next successful request.
- `<name> worker restarting in Ns`, in the warning color, until the
  worker restarts.

Other messages show for 5 or 10 seconds and then fade out.

The right part of the status bar shows the work that runs, after a
spinner. For example, `⠹ syncing team · prefetching 2` tells you that the
sync of the pull requests of your colleagues runs, and that the prefetch
has 2 pull requests left. The right part shows these items:

- `syncing mine`, `syncing team`: a sync of the pull requests, also a
  timed sync in the background. `syncing team` is the sync of the pull
  requests of your colleagues. It feeds the Review and the Team tab.
- `prefetching N`: the prefetch of pull request refs.
- `loading N conversations`: the comments, reviews and threads of N
  pull requests load from GitHub.
- `pr-agent review #7`: a pr-agent run of one tool on one pull request.
  Each run has its own item.
- `waiting for Rex`: a question waits in the command
  palette of Rex (see "Questions in the Rex palette").

When no work runs, the spinner and the items go away. A sync first
reads a light list of the pull requests in pages. Then it reads the full
data of only the new and the changed pull requests, in batches. The rows
of each batch show in the list when the batch arrives.

## How to extend

The code has three registries. A new action, tab or pane is one new
module plus one line in its registry. The names and the keys must be
unique; a test in each registry checks this.

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
`[actions.<id>]` table. After the action is done, chaoshub syncs the pull
request again and scans the repo again, for every action.

An action that changes GitHub also implements `pending`. It returns the
parts of the pull requests that it changes (`bus::PendingPart`) and the
change that GitHub must show for each part (`bus::Expect`). chaoshub then
pulses the parts and ends them by the same rules for every action (see
"Changes on their way to GitHub"). An action without `pending` changes
nothing on GitHub.

### Add a tab

1. Make a module in `src/app/tabs/`, for example `src/app/tabs/starred.rs`.
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

1. Make a module in `src/app/panes/`, for example `src/app/panes/files.rs`.
2. Implement the `Pane` trait of `src/app/panes/mod.rs`: `id`, `title`,
   `key` (the character that selects the pane in the sidebar), `applies`,
   `init_state`, `model` (a `PaneModel`: a table, lines or a tree) and
   `handle_key`.
3. In `src/app/panes/mod.rs`, add `mod files;` and add one line to
   `PANES`:

   ```rust
   Box::new(files::Files),
   ```

The sidebar shows the panes whose `applies` is true, in the order of
`PANES`.

## Development

jj is recommended, never required: every feature must work in a git-only
repo (see [AGENTS.md](AGENTS.md)).

Every change must pass the required checks of CI. Before each push, run
them with one command:

```sh
scripts/check.sh
```

The script runs `cargo build`, `cargo nextest run`, `cargo clippy` and
`cargo fmt --check` with the same flags and in the same order as CI.
First, it makes sure that rustc has the version of `rust-toolchain.toml`.
[AGENTS.md](AGENTS.md) gives the details.

### Benchmark of the review open

`bench/bench.sh` times the open of a pull request in codeview, from the
start of nvim (or of the Rex split) to the first file on the screen and
to the review comments. It runs in scratch clones under `$BENCH_DIR`,
because codeview writes refs there. The Rex case starts a private Rex
server, so the panes of the benchmark never reach your own Rex session.

```sh
export BENCH_DIR=/tmp/chaoshub-bench
bench/bench.sh setup     # clone the repo: a warm clone and a stale base template
bench/bench.sh headless  # nvim without a terminal
BENCH_CASE=stale bench/bench.sh headless
bench/bench.sh rex       # rex split and rex zoom, as chaoshub runs them
bench/bench.sh network   # the GitHub calls alone
bench/bench.sh clean
```

Each case runs 5 times (`BENCH_RUNS`) and prints the median of each stage
in milliseconds. `bench/bench.sh` lists all options.

## License

chaoshub is licensed under either of these licenses, at your option:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or
  <https://opensource.org/licenses/MIT>)

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the
Apache-2.0 license, shall be dual licensed as above, without any
additional terms or conditions.
