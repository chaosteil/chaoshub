# Configuration

The configuration file is `~/.config/chaoshub/config.toml`
(`$XDG_CONFIG_HOME/chaoshub/config.toml`), or the file of `--config`.
The file is TOML. Every key has a default, so an empty file or no file
is a valid configuration. A file that does not parse stops chaoshub with
an error that names the file. In paths, a leading `~` is your home
directory. Restart chaoshub after a configuration change.

- [Top-level keys](#top-level-keys)
- [`[[repo]]`](#repo)
- [`[filters]`](#filters)
- [`[refresh]`](#refresh)
- [`[prefetch]`](#prefetch)
- [`[codeview]`](#codeview)
- [`[detail]`](#detail)
- [`[conversation]`](#conversation)
- [`[merge]`](#merge)
- [`[actions.<id>]`](#actionsid)
- [`[agent]`](#agent)
- [`[theme]`](#theme)
- [`[keys]`](#keys)
- [`[terminal]`](#terminal)
- [`[notify]`](#notify)
- [`[workspaces]`](#workspaces)

## Top-level keys

```toml
login = "your-github-login"
teams = ["my-org/my-team"]
launcher = "auto"
```

| Key | Default | Meaning |
|---|---|---|
| `login` | none | Your GitHub login. When it is not set, chaoshub asks GitHub for the login of the token. |
| `teams` | `[]` | Teams in the `org/team` form. A review request to one of these teams counts as a review request to you. |
| `launcher` | `"auto"` | Where the review pane and the PR view pane open. `auto` and `rex` use a Rex pane when chaoshub runs inside Rex (`REX_BLOCK` is set). `inline` always runs the review in the same terminal. Any other value is an error. |

## `[[repo]]`

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
| `path` | none | The local checkout, a jj repo or a git repo. Without a path, chaoshub shows no local data for the repo, and the review action and the agent are disabled. |

## `[filters]`

Set the maximum age of pull requests (PRs) in the Mine, Review and Team
lists:

```toml
[filters]
max_age = "2 weeks"
```

`max_age` measures the time since a PR was created. For example,
`2 weeks` keeps PRs created within the last 14 days. The window moves
with the current time. A later update does not reset the age of a PR.

Use a positive duration such as `14 days`, `2 weeks` or `48 hours`. The
default has no age limit. Invalid durations and unknown filter keys stop
configuration loading.

The limit applies to GitHub searches and cached lists, including the
author picker and the `t` switch. Old cache rows without creation times
stay hidden until a sync reads that field. No cache reset is needed.

The limit changes list membership. It does not change a stack on GitHub
or prevent `chaoshub pr OWNER/NAME NUMBER` from opening a specific PR.
The existing author and text filters apply within the included PRs.

The `T` switch skips the limit. While `T` is on in a tab, the syncs
search GitHub without the limit (see
[Hidden rows](usage.md#hidden-rows)).

## `[refresh]`

The poll intervals, in seconds.

| Key | Default | Meaning |
|---|---|---|
| `mine_secs` | `60` | The sync of your own PRs (only the changes since the last sync). |
| `team_secs` | `90` | The sync of the PRs of your colleagues (only the changes). |
| `full_secs` | `900` | The full sync of both lists. The prefetch also fetches each base branch once in this period, and again when a sync reports a new base commit of a PR. |
| `ci_secs` | `45` | The CI poll of the visible PRs, and of the watched PRs of `[notify]` (see [Notice of new PRs](rex.md#notice-of-new-prs)). |
| `jj_secs` | `30` | The local scan with jj or git, while the active tab shows local data. |

When the GitHub rate budget runs low, the sync intervals double.

The PR view (`chaoshub pr`) refreshes its PR every `mine_secs`. GitHub
computes the merge state of a PR on request, for example after its base
changes. When a refresh gives an unknown merge state, the PR view
refreshes again after 3 s, at most 10 times in a row. The `refresh` key
starts this count again.

## `[prefetch]`

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `true` | Fetches the git refs of the selected and the visible PRs into the local repo, so a review in codeview starts without a fetch. The refs go under `refs/chaoshub/`, which jj does not import. |

## `[codeview]`

The Rex pane of the review action, and the codeview comment files.

| Key | Default | Meaning |
|---|---|---|
| `zoom` | `true` | Zooms the pane, so nvim fills the window. |
| `split` | `"right"` | The direction of the new pane: `right`, `left`, `above` or `below`. |
| `ratio` | none | The percent of the split that the pane gets. Without it, Rex uses its default. |
| `comments_dir` | see "Meaning" | The directory of the codeview comment files. `~` expands to your home directory. The default is the default of codeview: `$XDG_DATA_HOME/<app>/codeview/review`, else `~/.local/share/<app>/codeview/review`. `<app>` is `$NVIM_APPNAME`, else `nvim`. Set it when you set the `comments.dir` option of codeview. |

## `[detail]`

Where `enter` opens the PR view (see
[Where the PR view opens](pr-view.md#where-the-pr-view-opens)).

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

## `[conversation]`

The Conversation pane of the PR view (see [The panes](pr-view.md#the-panes)).

```toml
[conversation]
order = "newest_first"
```

| Key | Default | Meaning |
|---|---|---|
| `order` | `"oldest_first"` | The order of the comments, the reviews and the threads, and of the comments in each thread. `oldest_first` shows them in time order, with the oldest at the top. `newest_first` shows the newest at the top. A thread header stays above its comments. Any other value is an error. |

## `[merge]`

The configuration of the merge action (`m`). The `merge_stack` action
(`s`) uses the same configuration.

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
| `stack` | `auto` | How chaoshub merges a stack: `auto`, `atomic` or `sequential`. See [Merge a stack](actions.md#merge-a-stack). |
| `repos."owner/name"` | none | A table with `method`, `bypass_rules` and `stack` for one repo. Each key that it sets overrides the top-level key for that repo. |

chaoshub copies these keys into `[actions.merge]`. If you set a key in
`[actions.merge]`, that value wins over the value in `[merge]`.

## `[actions.<id>]`

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
table (see [Add an action](development.md#add-an-action)).

## `[agent]`

The AI review agent: the backend, its program, its model and its env.

```toml
[agent]
backend = "open_code_review"   # or "pr_agent"
timeout_secs = 900
```

The keys are `backend`, `env` and `timeout_secs`, and the tables
`[agent.open_code_review]` and `[agent.pr_agent]`. For all the keys and
for examples of each model provider, see
[Review agents](review-agents.md#configuration).

## `[theme]`

| Key | Default | Meaning |
|---|---|---|
| `style` | `punk` | The look: `punk`, `grunge` or `plain`. |
| `palette` | `pierre` | The colors: `pierre` or `ansi16`. On a 16-color terminal, `pierre` changes to `ansi16`. |
| `grunge` | `true` | Draws the rough block borders. Applies to the grunge style only. |
| `textures` | `true` | Fills empty areas with noise textures. Applies to the grunge style only. |
| `animations` | `true` | Turns the transitions on (see [Transitions](#transitions)). With `false`, none of them runs. |
| `animation_ms` | `120` | The duration of the popup, output, help and picker slides and of the scroll, in milliseconds. The other transitions have their own fixed durations. |
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

### Transitions

The transitions are short, and a key press during a transition
retargets it, so nothing blocks typing:

- The popup, the output and the help slide in from the right and out
  again (`animation_ms`, 120 ms by default). The confirm and the prompt
  slide in from the bottom in 100 ms, the picker in `animation_ms`. In
  the punk style, the drop shadow slides with the popup.
- A tab switch: the highlight of the active tab glides along the tab
  bar in 120 ms. The list changes at once.
- A pane switch in the PR view: the same glide on the pane tabs, in
  120 ms. The pane changes at once.
- A scroll: the rows move to the new offset (`animation_ms`).
- A cursor move in the list, a pane table or the picker: the selection
  band wipes onto the new row from the left in 60 ms.
- A successful action: the row of the PR flashes in the primary color
  and fades back in 200 ms.
- A sync or a CI poll that changes the checks or the reviews of a
  visible row: the row glows grey and fades back in 250 ms.
- An expanded folder, file or thread in the PR view: the new rows
  unfold from top to bottom in 80 ms. Collapsing is instant.
- A new status message slides in from the left in 80 ms. An expired
  message fades out in 300 ms.
- A new toast slides in from the right edge of the screen
  (`animation_ms`). A toast that replaces a toast during the slide takes
  over the slide. A toast that replaces a toast at rest shows at once.
  An expired toast fades out in 300 ms (see [Toast](usage.md#toast)).

If 5 of the last 10 drawn frames take longer than 12 ms to draw,
chaoshub turns the transitions off until it quits, and writes a line in
the log.

## `[keys]`

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

The binding names are the names of the [App keys](usage.md#app-keys)
and the action ids of [Actions](actions.md#actions). A `detail_<id>`
binding, for example `detail_close = "c"`, runs the action `<id>`
directly in the list and in the PR view (see
[Direct keys and the actions menu](actions.md#direct-keys-and-the-actions-menu)).

## `[terminal]`

The terminal title, the status sequences and the mouse.

| Key | Default | Meaning |
|---|---|---|
| `title` | detected | Sets the terminal title with the count of ready PRs to review, or the PR overview in the standalone PR view, and the current activity (see [Terminal title](rex.md#terminal-title)). |
| `program_status` | detected | Writes OSC 7501, the program status (see [Terminal status](rex.md#terminal-status-osc-7501-and-osc-94)). |
| `progress` | detected | Writes OSC 9;4, the progress bar. |
| `mouse` | `true` | Reads the mouse. `false` leaves the mouse to the terminal, so the terminal selects the text. chaoshub selects text itself (see [Mouse](usage.md#mouse)), so `false` is only necessary to select across panes. |

When `title`, `program_status` or `progress` is not set, chaoshub writes
the sequence only inside Rex (`REX_BLOCK` is set) and Ghostty
(`TERM_PROGRAM=ghostty`). `true` forces it on, `false` forces it off.

## `[notify]`

The terminal notice of new PRs that are ready for review (see
[Notice of new PRs](rex.md#notice-of-new-prs)).

```toml
[notify]
tabs = ["review"]
debounce = "1m"
```

| Key | Default | Meaning |
|---|---|---|
| `enabled` | detected | Sends the notice. When it is not set, chaoshub sends the notice only inside Rex and Ghostty, the same as `[terminal] program_status`. |
| `tabs` | `["team", "review"]` | The tabs to watch: `"mine"`, `"review"` or `"team"`. Another value stops chaoshub with an error. |
| `debounce` | `"15s"` | The wait before the notice, for example `"15s"` or `"2m"`. `"0s"` sends the notice at once. |
| `desktop` | `true` | Also sends each notice as a desktop notification with its text (OSC 777). `false` keeps only the program status record, so Rex shows only the tab dot. |

## `[workspaces]`

| Key | Default | Meaning |
|---|---|---|
| `root` | `"~/code/workspaces"` | The directory of the jj workspaces other than `default`: `<root>/<folder>/<workspace name>`, where `<folder>` is the folder name of the repo path. |
