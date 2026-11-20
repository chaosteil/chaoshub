# Usage

This page tells how to start chaoshub and how to use the list of pull
requests (PRs). For the panes of one PR, see [The PR view](pr-view.md).
For the actions, see [Actions and reviews](actions.md).

- [Start](#start)
- [The tabs](#the-tabs)
- [List columns](#list-columns)
- [Hidden rows](#hidden-rows)
- [Filter](#filter)
- [App keys](#app-keys)
- [Mouse](#mouse)
- [Status bar](#status-bar)
- [List the PRs from a script](#list-the-prs-from-a-script)
- [Files](#files)

## Start

```sh
chaoshub
```

chaoshub tracks the repos of the `[[repo]]` entries of the
configuration (see [Configuration](configuration.md#repo)). It also adds
the repo of the current directory, when its `origin` remote is a GitHub
repo.

chaoshub keeps a local cache. It shows the cached lists at once when it
starts, and then syncs with GitHub in the background.

To show one PR and nothing else, for example #7 of
`chaosteil/chaoshub-test`, run:

```sh
chaoshub pr chaosteil/chaoshub-test 7
```

You can also give the URL of the PR, for example a link from a chat.
Thus a link handler can open the PR in chaoshub:

```sh
chaoshub pr https://github.com/chaosteil/chaoshub-test/pull/7/files
```

The URL can have no `https://` and no `www.`, and it can be in `<...>`.
A query (`?w=1`) and an anchor (`#...`) are permitted. The end of the
URL selects the first pane:

| End of the URL | First pane |
|---|---|
| `/files`, `/changes` | Files |
| `/checks` | Checks |
| `#discussion_r…`, `#issuecomment-…`, `#pullrequestreview-…` | Conversation |
| Other, for example no end or `/commits` | Description |

Any other text, for example the URL of an issue, gives the error
`` `<text>` is not a PR url, and not `OWNER/NAME NUMBER` ``.

Command line options:

| Option | What it does |
|---|---|
| `--repo OWNER/NAME` | Adds a repo for this session. Repeat the option for more repos. A repo from this option has no local path. |
| `--config PATH` | Reads the configuration from `PATH` instead of `~/.config/chaoshub/config.toml`. |
| `--no-prefetch` | Turns off the prefetch of PR refs for this session. |
| `--theme STYLE` | Uses `punk`, `grunge` or `plain` for this session, in place of `[theme] style`. |
| `--palette NAME` | Uses `pierre` or `ansi16` for this session, in place of `[theme] palette`. |
| `cache clear` | Deletes the cache file. The next start syncs from GitHub again. |
| `list [--json] [--all]` | Prints the PRs of the tabs from the cache, and makes no GitHub call. See [List the PRs from a script](#list-the-prs-from-a-script). |
| `pr OWNER/NAME NUMBER` | Shows one PR and nothing else: the panes of the PR view. A PR that is not in the cache loads from GitHub. `q` or `esc` quits. The PR view pane inside Rex runs this command. |
| `pr URL` | The same as `pr OWNER/NAME NUMBER`, for the PR of a GitHub PR URL. The end of the URL selects the first pane (see above). |

## The tabs

chaoshub has three tabs:

- **Mine** lists your own PRs as stacks, and only PRs. A local change
  without a PR gets no row. The change, commit and workspace columns of
  each PR come from the scan of your local checkout, with jj or with git
  (see [jj and git](install.md#jj-and-git)).
- **Review** shows the PRs of your colleagues that request your review,
  or the review of one of your `teams`.
- **Team** shows the other PRs of your colleagues.

By default, the Review and the Team tab show only the PRs that are
ready for review: open, not a draft, and CI passed. `t` shows all open
PRs, and a second `t` shows only the ready ones again.

A header row above the list names the columns. `H` shows or hides it.
From the list, you can review a PR in codeview, merge it, send a review,
and do other actions (see [Actions and reviews](actions.md)).

## List columns

The list shows the state of each PR with glyphs and colors. The glyphs
are the same in every theme. Only the colors change: the names on this
page (success, danger, warning, primary, dim) are the colors of the
palette. Press `?` to see the same legend in the help overlay ("list
symbols"), in the colors of your theme.

### CHECKS

The CI state of the PR, then the passed checks of all checks. A check
passed when its conclusion is success, neutral or skipped.

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
| `D` | The PR is a draft. |
| `C` | The PR has merge conflicts. |
| `Q` | The PR is in the merge queue. |
| `◆` | A pane of the PR is open. |

### CHANGE

The Mine tab only. The local change that matches the PR. In a git repo,
the change is the branch name.

| Color | Meaning |
|---|---|
| Primary | Synced: the local change has the same commit as the PR head. |
| Warning | Diverged: the local change has a different commit. The commit column also has the warning color. |

### The stack tree

The Mine tab only. The first column starts with a tree glyph. A stack
starts with a header row, for example `stack #12 -> main · 3 PRs`.

| Symbol | Meaning |
|---|---|
| `─` | A PR without a stack. |
| `┌` | The top PR of a stack. |
| `├` | A PR in the middle of a stack. |
| `└` | The bottom PR of a stack. |

GitHub does not change the update time of a PR when it adds the PR to a
stack. For this reason, chaoshub finds a new stack in one of two ways:

- A sync reads a PR of the stack that changed after GitHub made the
  stack. Then chaoshub also reads the other open entries of the stack
  again.
- The next full sync (`refresh.full_secs`, and at the start) finds the
  new place in the stack of each PR.

## Hidden rows

Four rules hide PRs from a list:

- The age limit (`[filters] max_age`, see
  [Configuration](configuration.md#filters)).
- The ready rule of the Review and the Team tab (`t`).
- The author picker (`a`).
- The filter text (`/`).

Press `T` to show every open PR of the active tab. The four rules then
do not hide rows. In the Mine tab, only the age limit and the filter
text apply. Press `T` again for the normal view.

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
the PRs past the limit arrive after a few seconds. Setting `T` off
starts no sync. While `T` is on, a full sync does not remove the PRs
past the limit from the list. After you set `T` off in every tab, the
next full sync removes them again.

The rows that `T` shows count as visible rows: chaoshub polls their CI,
loads their details and prefetches their refs, like any other row.
These requests and the searches without the age limit use more of the
GitHub rate budget.

## Filter

Press `/` to type a filter for the active tab. The list shows only the
PRs that match the filter. Each tab has its own filter.

The filter searches this text of each PR: the title, the number with
`#`, the author and the head branch, in this order. For example:

```text
fix(store): reopen the connection #99 alice alice/reopen
```

The filter does not search the repo, the base branch or the labels.
`T` also shows the PRs that the filter hides, as dim rows with a `?`
(see [Hidden rows](#hidden-rows)).

| Key | What it does |
|---|---|
| `enter` | Keeps the filter. chaoshub saves it in the cache. The key hint bar shows `Enter apply`. |
| `esc` | Clears the filter. The key hint bar shows `Esc clear`. |
| `backspace` | Deletes the last character. |
| `down`, `up` | Moves the cursor while you type. |

### Syntax

A space separates two words. A PR must match every word (AND). There is
no OR.

| Word | Matches | Example | Example matches |
|---|---|---|---|
| `abc` | The letters of `abc` in this order, with other characters between them (fuzzy). | `fx conn` | `fix(store): reopen the connection` |
| `'abc` | The exact text `abc`. | `'reopen` | `reopen`, not `re-open` |
| `^abc` | Text that starts with `abc`. The text starts with the title. | `^fix` | `fix(store): ...`, not `docs: fix ...` |
| `abc$` | Text that ends with `abc`. The text ends with the branch. | `reopen$` | the branch `alice/reopen` |
| `!abc` | Text that does not contain `abc`. | `!draft` | every title without `draft` |
| `\` | Escapes a special character: `\!`, `\^` or `\'` at the start, `\$` at the end, and `\ ` for a space in a word. | `fix\ the` | `fix the` as one word |

Each word has smart case. A word with only lower case letters ignores
the case. A word with an upper case letter matches the case. For
example, `store` matches `Store` and `store`, but `Store` matches only
`Store`.

More examples:

| Filter | Shows |
|---|---|
| `#99` | The PR #99. The match is fuzzy, so other numbers can match too, for example #199. |
| `alice` | The PRs of `alice`, and other text with these letters. |
| `^feat !wip` | The titles that start with `feat` and do not contain `wip`. |
| `ui 'popup` | The PRs with `ui` (fuzzy) and the exact text `popup`. |

## App keys

The default bindings of the app. The name in the first column is the
binding name for `[keys]` (see [Configuration](configuration.md#keys)).
The keys of the actions are in [Actions and reviews](actions.md#actions),
and the keys of the panes are in [The PR view](pr-view.md).

| Binding | Keys | What it does |
|---|---|---|
| `tab_1` to `tab_9` | `1` to `9` | Selects a tab. In the PR view, the number keys select a pane. |
| `next_tab` | `tab` | The next tab. In the PR view, the next pane. |
| `down`, `up` | `j`, `k` | Moves the cursor. |
| `top`, `bottom` | `g`, `G` | Moves to the first or the last row. |
| `page_down`, `page_up` | `ctrl-d`, `ctrl-u` | Moves half a page. In the PR view, moves half the height of the active pane. |
| `scroll_down`, `scroll_up` | `D`, `U` | In the PR view, scrolls the active pane down or up by half its height. The cursor stays. When the cursor leaves the pane, it moves to the first visible row. |
| `open` | `enter` | Opens the PR view: a Rex pane inside Rex, else the sidebar (see `[detail]`). A second `enter` on the same PR focuses its pane. In the Conversation pane, collapses or expands a thread, and opens the comment menu on a comment (see [Reactions](pr-view.md#reactions)). |
| `open_url` | `o` | In the Checks pane, opens the selected check in the browser. |
| `actions` | `space` | Opens the actions menu: in the list, of the selected row. In the PR view, of its PR and its active pane (see [Actions](actions.md#actions)). On a row without a PR, or in an empty tab, the menu has only `open url`. |
| `close_overlay` | `esc`, `q` | Closes the sidebar or the overlay. When nothing is open, these keys quit (see `quit`). In the Rex pane of the PR view, they quit the pane. |
| `confirm` | `y` | Confirms in a confirm popup. `enter` also confirms. |
| `cancel` | `n` | Cancels a confirm popup. |
| `filter` | `/` | Types a fuzzy filter for the active tab (see [Filter](#filter)). |
| `authors` | `a` | Opens the author picker of the Review or the Team tab. Inside Rex, the picker can open in the Rex palette (see [Questions in the Rex palette](rex.md#questions-in-the-rex-palette)). |
| `jump` | `f` | Inside Rex, asks for a PR of the rows of all tabs in the Rex palette. The pick selects the tab and the row, and opens the PR as `enter` does. Without the Rex palette, `f` opens the filter, as `/` does. |
| `open_pr_url` | `u` | Only in the actions menu (`space` then `u`): asks for a GitHub PR URL and opens its PR view (see [Open a PR from a URL](actions.md#open-a-pr-from-a-url)). |
| `toggle_all` | `t` | On the Review and the Team tab, switches between every open PR and only the ready ones. The help shows it as "all open / ready only". |
| `show_hidden` | `T` | Switches between the normal view and every open PR of the active tab, also the rows that the age limit, the ready rule, the author picker and the filter text hide (see [Hidden rows](#hidden-rows)). The help shows it as "hidden rows / normal". Only the help overlay shows this key, not the key hint bar. |
| `refresh` | `R` | Syncs both lists now. In the PR view, syncs the PR and its stack, and scans its repo. |
| `column_headers` | `H` | Shows or hides the header row with the column names. chaoshub saves the choice in the cache. |
| `help` | `?` | Shows the help overlay with every binding. |
| `quit` | `q`, `esc`, `ctrl-c` | Quits. When the sidebar or an overlay is open, `esc` and `q` close it first. While you type a filter, `esc` clears the filter. `ctrl-c` quits from everywhere. |

In a prompt popup, `enter` sends the text and `esc` cancels.

## Mouse

chaoshub reads the mouse. Set `[terminal] mouse = false` to give the
mouse back to the terminal, for example to select text across panes.
While a command runs in the foreground (for example codeview without
Rex), chaoshub does not read the mouse.

The wheel moves three rows per step:

- On the list, it moves the cursor.
- In the sidebar, it scrolls the Description pane, and moves the cursor
  of the other panes.
- In the help overlay and the output popup, it scrolls the text.
- In the Review popup, it moves the cursor of the comment list.

A click acts on what it hits:

| Click on | What it does |
|---|---|
| A tab in the tab bar | Selects the tab. |
| A list row | Selects the row. A click on the selected row opens the sidebar. |
| A key hint | Presses the key. For example, `Space actions` in the list or in the sidebar opens the actions menu. |
| A pane tab in the sidebar | Selects the pane. |
| A row in a pane | Moves the cursor of the pane to the row. In the Conversation pane, a click on the selected thread expands it or collapses it, and a click on the selected comment opens the comment menu. In the Stack pane, a click on the selected entry opens its PR view. |
| An author in the author picker | Picks the author. |
| An action in the actions menu | Runs the action. |
| A comment in the Review popup | Marks or unmarks the comment. |
| A cell outside of the sidebar or the overlay | Closes it. For a confirm popup, this cancels the action. |

A drag with the left button selects text. The text areas are the body
of the sidebar and the PR view (all panes), the body of the help
overlay and the body of the output popup:

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
  lines. In the Conversation pane, the tree marks and the indent are in
  the copy.

With `[terminal] mouse = false`, chaoshub does not select text. The
terminal selects the text.

## Status bar

The status bar keeps three states until they end, also after a key
press:

- `rate limited until HH:MM`, in the warning color, until the reset
  time.
- `offline`, in the danger color, until the next successful request.
- `<name> worker restarting in Ns`, in the warning color, until the
  worker restarts.

Other messages show for 5 or 10 seconds and then fade out.

The right part of the status bar shows the work that runs, after a
spinner. For example, `⠹ syncing team · prefetching 2` tells you that
the sync of the PRs of your colleagues runs, and that the prefetch has
2 PRs left. The right part shows these items:

- `syncing mine`, `syncing team`: a sync of the PRs, also a timed sync
  in the background. `syncing team` is the sync of the PRs of your
  colleagues. It feeds the Review and the Team tab.
- `prefetching N`: the prefetch of PR refs.
- `loading N conversations`: the comments, reviews and threads of N PRs
  load from GitHub.
- `ocr review #7` or `pr-agent review #7`: an agent run of one tool on
  one PR. Each run has its own item (see
  [Review agents](review-agents.md)).
- `waiting for Rex`: a question waits in the command palette of Rex
  (see [Questions in the Rex palette](rex.md#questions-in-the-rex-palette)).

When no work runs, the spinner and the items go away. A sync first
reads a light list of the PRs in pages. Then it reads the full data of
only the new and the changed PRs, in batches. The rows of each batch
show in the list when the batch arrives.

The terminal title and the tab status of Rex and Ghostty also show what
chaoshub does (see [Rex, Ghostty and the terminal](rex.md#terminal-title)).

## List the PRs from a script

`chaoshub list` prints the open PRs of the cache that the tabs show:
first the Mine tab, then the Review tab, then the Team tab, in the order
of each tab. It is for scripts, for example the Rex action of
[Open a PR from the Rex palette](rex.md#open-a-pr-from-the-rex-palette).

- The command opens the cache read-only. It makes no GitHub call, so
  the list is as old as the last sync of the TUI. If the cache does not
  exist, the list is empty.
- The rules of the normal view apply: the `[filters]` age limit for all
  tabs, and the ready rule for the Review tab and the Team tab. With
  `--all`, the command prints every open PR of the cache, as the
  `show_hidden` key (`T`) does.
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
| `synced_at` | The newest sync time of the cached PRs, or `null` when the cache holds none. |
| `repo`, `number` | The PR. `chaoshub pr <repo> <number>` shows it. |
| `title`, `author`, `url` | The title, the login of the author and the GitHub URL. |
| `tab` | `mine`, `review` or `team`. |
| `state` | `open`. The list holds only open PRs. |
| `draft` | True for a draft. |
| `review_requested` | True when GitHub requests the review of you or of one of your teams. |
| `status` | The verdict, the CI and the reviews, as in the PR overview (see [Terminal title](rex.md#terminal-title)). |
| `label` | The PR overview, for example `#7 Fix the merge · ready · CI passed · approved`. The PR view pane inside Rex uses it as the block label. |
| `path` | The `path` of the `[[repo]]` entry of the repo, or `null`. |

## Files

| File | Location |
|---|---|
| Configuration | `~/.config/chaoshub/config.toml` (`$XDG_CONFIG_HOME/chaoshub/config.toml`) |
| Cache (SQLite) | `~/.cache/chaoshub/chaoshub.db` (`$XDG_CACHE_HOME/chaoshub/chaoshub.db`) |
| Log | `~/.local/state/chaoshub/chaoshub.log` (`$XDG_STATE_HOME/chaoshub/chaoshub.log`) |
| Agent results | `~/.local/state/chaoshub/agent/<owner>/<repo>/<n>/` (see [The result files](review-agents.md#the-result-files)) |
| List socket | `$XDG_RUNTIME_DIR/chaoshub/list-<pid>.sock`, else `~/.local/state/chaoshub/ipc/list-<pid>.sock` (see [How the PR view talks to the list](rex.md#how-the-pr-view-talks-to-the-list)) |

chaoshub uses these XDG paths on macOS too, not `~/Library`.

The cache holds the last lists, the local scans and the filters of the
tabs. If the cache does not open, chaoshub runs with an empty cache in
memory. To delete the cache, run:

```sh
chaoshub cache clear
```

The log gets messages at the info level and above. The log file grows
until you delete it.
