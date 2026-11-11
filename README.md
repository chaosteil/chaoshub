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

```sh
cargo install --path .
```

The repo pins the Rust toolchain in `rust-toolchain.toml`.

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
| `pr OWNER/NAME NUMBER` | Shows one pull request and nothing else: the panes of the pull request detail. A pull request that is not in the cache loads from GitHub. `q` or `esc` quits. The detail pane inside Rex runs this command. |

To show one pull request, for example #7 of `chaosteil/chaoshub-test`,
run:

```sh
chaoshub pr chaosteil/chaoshub-test 7
```

## Files

| File | Location |
|---|---|
| Configuration | `~/.config/chaoshub/config.toml` (`$XDG_CONFIG_HOME/chaoshub/config.toml`) |
| Cache (SQLite) | `~/.cache/chaoshub/chaoshub.db` (`$XDG_CACHE_HOME/chaoshub/chaoshub.db`) |
| Log | `~/.local/state/chaoshub/chaoshub.log` (`$XDG_STATE_HOME/chaoshub/chaoshub.log`) |
| pr-agent results | `~/.local/state/chaoshub/agent/<owner>/<repo>/<n>/` (see "pr-agent") |

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

Set a creation date cutoff for the Mine, Review and Team lists:

```toml
[filters]
created_since = "2026-09-01"
```

`created_since` includes pull requests created on or after midnight UTC
on that date. Use a quoted `YYYY-MM-DD` date. The default has no cutoff.
A later update does not bring an older pull request back into the lists.

The cutoff applies to GitHub searches and cached lists, including the
author picker and the `t` switch. Restart chaoshub after a change. An old
cache without creation dates hides those rows until the next sync reads
their creation dates. No cache reset is needed.

The cutoff changes list membership. It does not change a stack on GitHub
or prevent `chaoshub pr OWNER/NAME NUMBER` from opening a specific pull
request. The existing author and text filters still apply within the
included pull requests.

### `[refresh]`

The poll intervals, in seconds.

| Key | Default | Meaning |
|---|---|---|
| `mine_secs` | `60` | The sync of your own pull requests (only the changes since the last sync). |
| `team_secs` | `90` | The sync of the pull requests of your colleagues (only the changes). |
| `full_secs` | `900` | The full sync of both lists. The prefetch also fetches each base branch once in this period. |
| `ci_secs` | `45` | The CI poll of the visible pull requests. |
| `jj_secs` | `30` | The local scan with jj or git, while the active tab shows local data. |

When the GitHub rate budget runs low, the sync intervals double.

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

Each action reads the table with its id. Three actions use a table:

| Table | Key | Meaning |
|---|---|---|
| `[actions.review]` | `zoom` | Zooms the review pane. The default is the value of `[codeview] zoom`. |
| `[actions.codeview_send]` | `comments_dir` | The directory of the codeview comment files. The default is the value of `[codeview] comments_dir`. |
| `[actions.merge]` | `method`, `bypass_rules`, `stack`, `repos` | The same keys as `[merge]`. |

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
below. A `detail_<id>` binding, for example `detail_merge = "m"`,
runs the action `<id>` directly in the list and in the pull request
detail (see "Actions").

### `[terminal]`

The terminal status sequences (see "Rex, Ghostty and codeview") and the
mouse (see "Mouse").

| Key | Default | Meaning |
|---|---|---|
| `program_status` | detected | Writes OSC 7501, the program status. |
| `progress` | detected | Writes OSC 9;4, the progress bar. |
| `mouse` | `true` | Reads the mouse. `false` leaves the mouse to the terminal, so the terminal selects the text. chaoshub selects text itself (see "Mouse"), so `false` is only necessary to select across panes. |

When `program_status` or `progress` is not set, chaoshub writes the
sequence only inside Rex (`REX_BLOCK` is set) and Ghostty
(`TERM_PROGRAM=ghostty`). `true` forces it on, `false` forces it off.

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

### Ready for review

By default, the Review and the Team tab show only the pull requests that
are ready for review: open, not a draft, and CI green. `t` shows all open
pull requests.

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
| `open` | `enter` | Opens the pull request detail: a Rex pane inside Rex, else the sidebar (see `[detail]`). A second `enter` on the same pull request focuses its pane. In the conversation pane, expands a thread. |
| `open_url` | `o` | In the checks pane, opens the selected check in the browser. |
| `actions` | `space` | Opens the actions menu: in the list, of the selected row; in the pull request detail, of its pull request and its active pane (see "Actions"). |
| `close_overlay` | `esc`, `q` | Closes the sidebar or the overlay. When nothing is open, these keys quit (see `quit`). In the Rex pane of the detail, they quit the pane. |
| `confirm` | `y` | Confirms in a confirm popup. `enter` also confirms. |
| `cancel` | `n` | Cancels a confirm popup. |
| `filter` | `/` | Types a fuzzy filter for the active tab. |
| `authors` | `a` | Opens the author picker of the Review or the Team tab. |
| `toggle_all` | `t` | On the Review and the Team tab, shows every open pull request, not only the ready ones. |
| `refresh` | `r` | Syncs both lists now. In the pull request detail, syncs the pull request and its stack, and scans its repo. |
| `column_headers` | `H` | Shows or hides the header row with the column names. chaoshub saves the choice in the cache. |
| `help` | `?` | Shows the help overlay with every binding. |
| `quit` | `q`, `esc`, `ctrl-c` | Quits. When the sidebar or an overlay is open, `esc` and `q` close it first. While you type a filter, `esc` clears the filter. `ctrl-c` quits from everywhere. |

In a prompt popup, `enter` submits the text and `esc` cancels.

### Pull request detail

`enter` on a pull request opens its detail. Where it opens depends on
`[detail] mode`.

Inside Rex, `enter` runs `chaoshub pr OWNER/NAME N` in a Rex pane beside
the list, and the pane takes the focus. The pane shows the panes of the
pull request and nothing else. `q` or `esc` in the pane closes the pane,
and the list then syncs the pull request again and scans its repo.
`enter` on another pull request replaces the pane. `enter` on the same
pull request focuses the pane. A quit of the list closes the pane. A
quit of the pane leaves a codeview review from `v` open.

Outside Rex, `enter` opens the sidebar. The sidebar slides in from the
right and covers 80% of the width, over the list rows. The tab bar, the
key hint bar and the status bar stay visible. `esc` or `q` closes the
sidebar.

The detail has five panes. The number keys select a pane, and `tab`
selects the next pane:

| Key | Pane | What it shows |
|---|---|---|
| `1` | Description | The body of the pull request as GitHub renders it, with tables, details and task lists. The text reflows when the pane width changes. The detail opens on this pane. The Description pane starts with a readiness block: the merge verdict and its reasons, the checks, the reviews, the open threads, the size, and the latest activity. When the head has codeview comments that did not go to GitHub yet, one dim line below the block counts them, for example `3 unsent codeview comments · space r to review` (see "Codeview comments" below). Without such comments, the pane shows no line. |
| `2` | Checks | One row per check, with its state and its duration. |
| `3` | Conversation | The comments, the reviews and the review threads, in time order. |
| `4` | Stack | The entries of the stack. Only for a stacked pull request. For a pull request without a stack, the tab shows dim, and `4`, `tab` and a click do not select it. The cursor starts on the pull request of the detail. Enter opens the detail of the entry under the cursor on top, and `esc` goes back. `o` opens the page of that entry. |
| `5` | Files | The changed files as a tree, with the diff of each file inline. |
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
directly: the navigation keys, `r` (refresh), `v` (codeview), `o`
(browser), `y` and `i` (copy). The other actions are in the actions
menu. Press `space` to open it on top of the detail, then the key of
the action. For example, `space` then `r` opens the Review popup,
and `space` then `m` opens the merge confirm popup. `esc` closes the
menu, and the detail shows again. The pane keys, the motion keys and
the keys of the active pane come first: in the checks pane, `o` opens
the selected check, not the pull request. The key hint bar shows
`Space actions`.

The actions menu also lists the own actions of the active pane, after
the actions of the pull request: the pr-agent tools of the Agent pane.
In the menu, an action key wins over
a pane key. Thus in the Agent pane, `space` then `r` opens the Review
popup, and `space` then `R` starts the pr-agent review. The menu of the
Agent pane does not show the `agent` action, because the pane shows
already. To bind a direct key for an action in the detail, see
"Actions".

#### Codeview comments

You write review comments in codeview (`v`). The Review popup lists the
comments that did not go to GitHub yet, and sends the marked comments
as one review. The Review popup is the only way to approve a pull
request, to request changes, and to send codeview comments. chaoshub
reads the comment files of codeview (`[codeview] comments_dir`) and
never writes them.

Press `space`, then `r`, to open the Review popup. This works in the
list and in the pull request detail. The popup opens at once with the
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
`[-]`, and the cursor skips it. A dim line counts the comments of an
older push of the head: open codeview (`v`) to see them.

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

The text box of the message can have more than one line, and its line
breaks go to GitHub:

| Key | What it does |
|---|---|
| `enter` | Adds a new line. |
| `ctrl-e` | Opens the text in `$VISUAL`, else `$EDITOR`, else `nvim`. chaoshub leaves its screen while the editor runs. When the editor quits, its text comes back into the text box. |
| `ctrl-s` / `esc` | Goes back to the Review popup, with the text. |

When a marked comment went out from codeview in the meantime, the send
leaves it out. When codeview can post none of the marked comments, for
example because no comment maps to the diff, it sends no review. The
status line then says `the review did not go out`, and names the
comments that stay local.

### Actions

The actions work on the pull request of the selected row, or on the
pull request of the detail. When an action cannot run, the status line
says why.

In the list, only the action keys that change nothing on GitHub work
directly on the selected row: `v` (codeview), `o` (browser), `y` and
`i` (copy). The other action keys do nothing in the list. Thus a wrong
key cannot send a review. A row without a pull request, for example a
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
pull request of the detail. There, as in the list, only `v`, `o`, `y`
and `i` also work directly. To run another action directly in the list
and in the detail, bind a key to `detail_<id>` in `[keys]`. For
example, `detail_merge = "m"` makes `m` open the merge confirm popup of
the selected row or of the detail. These bindings have no default.

| Binding | Key | What it does |
|---|---|---|
| `review` | `v` | Reviews the pull request in codeview. Needs a repo with a `path`. A second press focuses the open pane. |
| `merge` | `m` | Merges the pull request after a confirm popup. For a stacked pull request, see the stack merge paragraph below. |
| `merge_stack` | `s` | Merges the whole stack of the pull request after a confirm popup, from any entry of the stack. It is the same merge as `merge` on the top entry. Hidden for a pull request without a stack. |
| `rerun_checks` | `x` | Runs the failed checks again. |
| `draft` | `d` | Changes the pull request from draft to ready, or from ready to draft. |
| `close` | `c` | Closes the pull request after a confirm popup. |
| `browser` | `o` | Opens the pull request in the browser. |
| `copy_url` | `y` | Copies the pull request URL. |
| `copy_change_id` | `i` | Copies the jj change id of the local change that matches the pull request. In a git repo, it copies the local branch name, and the actions menu shows "copy the branch name". Hidden when no local change matches. |
| `agent` | `p` | Opens the detail on the Agent pane. Needs a repo with a `path`. |
| `codeview_send` | `r` | Opens the Review popup. There you mark the codeview comments, write the message, and comment, approve or request changes (see "Codeview comments"). |

A merge of a stacked pull request includes every entry up to it. To
merge every entry, use `merge_stack` (`s`) from any entry. The
`[merge] stack` key sets how chaoshub sends the merge. In `auto` mode,
chaoshub reads the squash defaults of the repo. When they are the PR
title and the PR body, or when the method is `merge` or `rebase`,
chaoshub sends one atomic request. Else chaoshub merges each entry
itself, bottom first, each with its own title and body. Thus every
squash commit carries its PR body. The confirm popup says which way
chaoshub merges.

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
| A row in a pane | Moves the cursor of the pane to the row. In the conversation pane, a click on the selected thread expands it or collapses it. In the stack pane, a click on the selected entry opens its detail. |
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
(`6`). The pane has one section per tool. Each section shows the state (`never`,
`running` with a spinner, `done 5m ago`, or `failed` with the error), the
model and the head commit of the result. When the head of the pull
request moved after the run, the section says
`outdated: for 3a1f2c0, the head is now 9bd2e11`. chaoshub does not run
pr-agent again by itself.

The pane has no direct keys. Its tools are in the actions menu: press
`space`, then the key of the tool.

| Key | Tool | What it shows |
|---|---|---|
| `R` | review | The fields of the review and the key issues, with file and lines. |
| `I` | improve | One line per suggestion: `label · file:line · summary`. |
| `D` | describe | The new title and the new body. |
| `a` | ask | Asks for a question first. Shows the questions and the answers, newest first. |
| `p` | | Opens a confirm popup with the line diff of the old and the new title and body. `y` sets the title and the body of the pull request on GitHub. |

One run per tool per pull request runs at a time. A second `space`
then `R` while the review runs only says `review runs already`. Different tools, and
different pull requests, run at the same time.

In the menu, the action keys win over the pane keys. Thus a tool uses
an uppercase key when an action uses the lowercase letter: `r` opens
the Review popup, `i` copies the change id, and `d` changes the draft
state. `p` applies the describe result, because the menu of the Agent
pane does not show the `agent` action.

While a run goes, the status bar shows the spinner and one item per run,
for example `agent review #7 · agent ask #7`. The last output line of
pr-agent shows in the status line. At the end, the status line shows
`review done (12k tokens)`, or the error.

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

Inside Rex, chaoshub opens a pane beside its own block with
`rex split -b $REX_BLOCK --split=<split> [--ratio <ratio>] -c <repo path>
--label <label> --json -- nvim ...`. With `zoom`, it runs
`rex zoom -b <block> --on`. It waits for the pane with `rex wait`. The
Rex binary is `$REX_BIN_DIR/rex` when that file exists, else `rex` on
your `PATH`.

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
{ "version": 1, "repo": "owner/name", "number": 14, "head": "<sha>",
  "ids": ["..."], "event": "comment", "body": "...", "out": "<result>" }
```

`event` is `comment`, `approve` or `request-changes`. codeview opens the
pull request, maps the comments to the lines of the diff, posts one
review, marks the comments as sent in its comment file, and writes the
result to `<result>`. nvim gets `CHAOSHUB_AGENT_ROOT=<state dir>/agent`,
like the review of `v`. chaoshub stops nvim after 120 seconds.

chaoshub never posts these comments to GitHub itself. Only codeview
posts, so its lock file and its sent marks stop a second send of the
same comment, also from an open codeview. The status line then says
`posted N comments as a <event> review`, and names the comments that
stay local and the comments that went out already. After the send,
chaoshub reads the comment files again. See `:help codeview-bridge` in
nvim for the request and the result.

### What `enter` runs inside Rex

The pull request detail runs chaoshub again, in a Rex pane beside the
list:

```sh
rex split -b $REX_BLOCK --split=<[detail] split> [--ratio <[detail] ratio>] \
  -c <repo path> --label chaoshub-detail-<owner>-<name>-<N>-<id> --json -- \
  <chaoshub> --config <config path> --no-prefetch [--theme <style>] \
  [--palette <name>] [--repo <owner/name> ...] pr <owner/name> <N>
```

The pane gets the configuration file and the `--theme`, `--palette` and
`--repo` options of the list. It always gets `--no-prefetch`, because
the list already prefetches the pull request of its cursor row. A repo
without a `path` runs in the directory of the list. chaoshub does not
zoom the pane. It waits for the pane with `rex wait`.

### Terminal status: OSC 7501 and OSC 9;4

chaoshub tells the terminal what it does, so a Rex or Ghostty tab shows
the state without a look at the pane. OSC 7501 is the program status,
and OSC 9;4 is the progress bar. Every OSC 7501 record has
`app=chaoshub`.

| Situation | OSC 7501 | OSC 9;4 |
|---|---|---|
| Idle | `state=idle` | off |
| First sync or `r` sync | `state=working`, `syncing <n> repos` | indeterminate |
| Prefetch of more than one ref | `state=working`, `prefetching` | percent done |
| Confirm or prompt popup open | `state=blocked`, `kind=question`, the popup title | unchanged |
| Merge pending | `state=working`, `merging #N` | indeterminate |
| Merge in the merge queue | `state=working`, `#N in merge queue` | indeterminate |
| Merged | `state=done`, `merged #N` | 100 for 2 s, then off |
| Merge failed, sync failed, worker died | `state=error`, the reason | error for 5 s, then off |
| Rate limited | `state=error`, `rate limited until HH:MM` | paused |
| Review pane open | `state=idle` | unchanged |
| Quit, review in the same terminal, crash | `state=clear` | off |

The `done` and `error` states clear on the next key press or the next
successful sync.

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
spinner. For example, `⠹ syncing team · prefetch 2` tells you that the
sync of the pull requests of your colleagues runs, and that the prefetch
has 2 pull requests left. The right part shows these items:

- `syncing mine`, `syncing team`: a sync of the pull requests, also a
  timed sync in the background. `syncing team` is the sync of the pull
  requests of your colleagues. It feeds the Review and the Team tab.
- `prefetch N`: the prefetch of pull request refs.
- `loading N conversations`: the comments, reviews and threads of N
  pull requests load from GitHub.
- `agent review #7`: a pr-agent run of one tool on one pull request.
  Each run has its own item.

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

Every change must pass these checks:

```sh
cargo build
cargo nextest run
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```
