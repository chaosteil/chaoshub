# chaoshub

chaoshub is a terminal app that tracks your GitHub pull requests. The
Mine tab lists your own pull requests as stacks, and only pull requests.
A local jj change without a pull request gets no row. The change id,
commit and workspace columns of each pull request come from the jj scan
of your local checkout. chaoshub also shows the pull requests of your
colleagues that are ready for review. From the list, you can review a
pull request in codeview, merge it, approve it, and do other actions.

chaoshub keeps a local cache. It shows the cached lists at once when it
starts, and then syncs with GitHub in the background.

## Requirements

- **jj** (Jujutsu) on your `PATH`. chaoshub reads your local changes and
  workspaces with `jj`.
- **git** on your `PATH`. chaoshub finds the repo of the current
  directory with `git remote get-url origin`, and prefetches pull request
  refs with `git fetch`.
- **gh**, the GitHub command line tool, logged in. Run `gh auth login`
  once. chaoshub gets its token from `gh auth token`.
- **nvim** with the **codeview** plugin, for the review action (`v`).
- **Rex** (optional). Inside Rex, the review and the pull request
  detail open in Rex panes beside chaoshub. Without Rex, the review runs
  in the same terminal, and chaoshub comes back when nvim quits.

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
| `--theme STYLE` | Uses `grunge`, `punk` or `plain` for this session, in place of `[theme] style`. |
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

chaoshub uses these XDG paths on macOS too, not `~/Library`.

The cache holds the last lists, the jj scans and the filters of the
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
| `path` | none | The local jj checkout. Without a path, chaoshub shows no jj data for the repo, and the review action is disabled. |

### `[refresh]`

The poll intervals, in seconds.

| Key | Default | Meaning |
|---|---|---|
| `mine_secs` | `60` | The sync of your own pull requests (only the changes since the last sync). |
| `team_secs` | `90` | The sync of the pull requests of your colleagues (only the changes). |
| `full_secs` | `900` | The full sync of both lists. The prefetch also fetches each base branch once in this period. |
| `ci_secs` | `45` | The CI poll of the visible pull requests. |
| `jj_secs` | `30` | The jj scan, while the active tab shows jj data. |

When the GitHub rate budget runs low, the sync intervals double.

### `[prefetch]`

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `true` | Fetches the git refs of the selected and the visible pull requests into the local repo, so a review in codeview starts without a fetch. The refs go under `refs/chaoshub/`, which jj does not import. |

### `[codeview]`

The Rex pane of the review action.

| Key | Default | Meaning |
|---|---|---|
| `zoom` | `true` | Zooms the pane, so nvim fills the window. |
| `split` | `"right"` | The direction of the new pane: `right`, `left`, `above` or `below`. |
| `ratio` | none | The percent of the split that the pane gets. Without it, Rex uses its default. |

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

The settings of the merge action (`m`).

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

Each action reads the table with its id. Two actions use a table:

| Table | Key | Meaning |
|---|---|---|
| `[actions.review]` | `zoom` | Zooms the review pane. The default is the value of `[codeview] zoom`. |
| `[actions.merge]` | `method`, `bypass_rules`, `stack`, `repos` | The same keys as `[merge]`. |

The other actions ignore their table. A new action can read its own
table (see "How to extend").

### `[theme]`

| Key | Default | Meaning |
|---|---|---|
| `style` | `grunge` | The look: `grunge`, `punk` or `plain`. |
| `palette` | `pierre` | The colors: `pierre` or `ansi16`. On a 16-color terminal, `pierre` changes to `ansi16`. |
| `grunge` | `true` | Draws the rough block borders. Applies to the grunge style only. |
| `textures` | `true` | Fills empty areas with noise textures. Applies to the grunge style only. |
| `animations` | `true` | Turns the transitions on: tab slides, popup slides, scrolls and the status message fade. |
| `animation_ms` | `160` | The duration of one transition, in milliseconds. The confirm and prompt popups always take 100 ms, and the status fade takes 300 ms. |
| `key_hints` | `true` | Shows the key hint bar above the status bar. |

The style and the palette are two switches. The style sets the shapes,
and the palette sets the colors. The punk style keeps the palette and
changes the shapes: cut corners, diagonal accents, inverted headers and
a selection band. To try it, start chaoshub with `--theme punk`.

If two frames in a row take longer than 12 ms to draw, chaoshub turns
the transitions off until it quits, and writes a line in the log.

### `[keys]`

Overrides the key bindings. Each key is a binding name, and each value
is one or more key chords, separated by spaces. An override replaces
every default chord of that binding. An empty value removes the binding.

```toml
[keys]
merge = "M"
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
below.

### `[terminal]`

The terminal status sequences (see "Rex, Ghostty and codeview") and the
mouse (see "Mouse").

| Key | Default | Meaning |
|---|---|---|
| `program_status` | detected | Writes OSC 7501, the program status. |
| `progress` | detected | Writes OSC 9;4, the progress bar. |
| `mouse` | `true` | Reads the mouse. `false` leaves the mouse to the terminal, so a drag selects text again. |

When `program_status` or `progress` is not set, chaoshub writes the
sequence only inside Rex (`REX_BLOCK` is set) and Ghostty
(`TERM_PROGRAM=ghostty`). `true` forces it on, `false` forces it off.

### `[workspaces]`

| Key | Default | Meaning |
|---|---|---|
| `root` | `"~/code/workspaces"` | The directory of the jj workspaces other than `default`: `<root>/<folder>/<workspace name>`, where `<folder>` is the folder name of the repo path. |

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
| `page_down`, `page_up` | `ctrl-d`, `ctrl-u` | Moves one page. |
| `open` | `enter` | Opens the pull request detail: a Rex pane inside Rex, else the sidebar (see `[detail]`). A second `enter` on the same pull request focuses its pane. In the conversation pane, expands a thread. |
| `open_url` | `o` | In the checks pane, opens the selected check in the browser. |
| `close_overlay` | `esc`, `q` | Closes the sidebar or the overlay. When nothing is open, these keys quit (see `quit`). In the Rex pane of the detail, they quit the pane. |
| `confirm` | `y` | Confirms in a confirm popup. `enter` also confirms. |
| `cancel` | `n` | Cancels a confirm popup. |
| `filter` | `/` | Types a fuzzy filter for the active tab. |
| `authors` | `a` | Opens the author picker of the Team tab. |
| `toggle_all` | `t` | On the Team tab, shows every open pull request, not only the ready ones. |
| `refresh` | `r` | Syncs both lists now. In the pull request detail, syncs the pull request and its stack, and scans its repo. |
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

The detail has four panes. The number keys select a pane, and `tab`
selects the next pane:

| Key | Pane | What it shows |
|---|---|---|
| `1` | Description | The body of the pull request as markdown. The detail opens on this pane. |
| `2` | Checks | One row per check, with its state and its duration. |
| `3` | Conversation | The comments, the reviews and the review threads, in time order. |
| `4` | Stack | The entries of the stack. Only for a stacked pull request. |

`j`, `k` and the other motion keys move the cursor of the pane. In the
description pane and the stack pane, they scroll the text.

The action keys also work in the detail, on the pull request of the
detail. For example, `v` opens codeview, and `m` opens the merge
confirm popup on top of the detail. The pane keys, the motion keys and
the keys of the active pane come first: in the checks pane, `o` opens
the selected check, not the pull request. The key hint bar shows the
action keys that work for the pull request.

### Actions

The actions work on the pull request of the selected row, or on the
pull request of the sidebar. When an action cannot run, the status line
says why.

| Binding | Key | What it does |
|---|---|---|
| `review` | `v` | Reviews the pull request in codeview. Needs a repo with a `path`. A second press focuses the open pane. |
| `merge` | `m` | Merges the pull request after a confirm popup. For a stacked pull request, see the stack merge paragraph below. |
| `approve` | `A` | Approves the pull request. Not for your own pull request. |
| `request_changes` | `R` | Asks for a message, then requests changes. Not for your own pull request. The message must not be empty. |
| `rerun_checks` | `x` | Runs the failed checks again. |
| `draft` | `d` | Changes the pull request from draft to ready, or from ready to draft. |
| `close` | `X` | Closes the pull request after a confirm popup. |
| `browser` | `o` | Opens the pull request in the browser. |
| `copy_url` | `y` | Copies the pull request URL. |
| `copy_change_id` | `Y` | Copies the jj change id of the local change that matches the pull request. Hidden when no local change matches. |

A merge of a stacked pull request includes every entry up to it. The
`[merge] stack` key sets how chaoshub sends the merge. In `auto` mode,
chaoshub reads the squash defaults of the repo. When they are the PR
title and the PR body, or when the method is `merge` or `rebase`,
chaoshub sends one atomic request. Else chaoshub merges each entry
itself, bottom first, each with its own title and body. Thus every
squash commit carries its PR body. The confirm popup says which way
chaoshub merges.

### Mouse

chaoshub reads the mouse. Set `[terminal] mouse = false` to give the
mouse back to the terminal, for example to select text. While a command
runs in the foreground (for example codeview without Rex), chaoshub
does not read the mouse.

The wheel moves three rows per step:

- On the list, it moves the cursor.
- In the sidebar, it scrolls the description pane and the stack pane,
  and moves the cursor of the checks pane and the conversation pane.
- In the help overlay and the output popup, it scrolls the text.

A click acts on what it hits:

| Click on | What it does |
|---|---|
| A tab in the tab bar | Selects the tab. |
| A list row | Selects the row. A click on the selected row opens the sidebar. |
| A key hint | Presses the key. Every action with a hint is clickable, for example `v review`. |
| A pane tab in the sidebar | Selects the pane. |
| A row in a pane | Moves the cursor of the pane to the row. In the conversation pane, a click on the selected thread expands it or collapses it. |
| An author in the author picker | Picks the author. |
| A cell outside of the sidebar or the overlay | Closes it. For a confirm popup, this cancels the action. |

## Rex, Ghostty and codeview

### What `v` runs

The review action runs nvim in the local path of the repo:

```sh
nvim -c "autocmd User CodeViewSessionClosed ++once qall" -c "Codeview pr <N>"
```

`Codeview pr <N>` opens the pull request in codeview. When you close the
codeview session, codeview sends `User CodeViewSessionClosed`, and nvim
quits.

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

A spinner turns at the end of the left part while one of these runs:

- The first sync after the start.
- A sync that the `refresh` key started.
- A prefetch of pull request refs.

While the sync of the active tab runs, the left part shows `syncing` in
place of the age of the data. A sync reads the pull requests in pages,
and the rows of each page show in the list when the page arrives.

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

1. Make a module in `src/app/tabs/`, for example `src/app/tabs/review.rs`.
2. Implement the `Tab` trait of `src/app/tabs/mod.rs`: `id`, `title`,
   `scope` (the sync scope that feeds the tab), `wants_local`,
   `init_state`, `model` (the `ListModel` of the tab), `interest`,
   `rescore` (the fuzzy filter), `authors` (the author picker),
   `handle_key` and `help`.
3. In `src/app/tabs/mod.rs`, add `mod review;` and add the tab to
   `TABS`:

   ```rust
   LazyLock::new(|| vec![Box::new(mine::Mine), Box::new(team::Team), Box::new(review::Review)])
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

Every change must pass these checks:

```sh
cargo build
cargo nextest run
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```
