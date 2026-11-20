# Rex, Ghostty and the terminal

This page tells what chaoshub runs for codeview and Rex, and what it
tells the terminal. Rex is optional. Without Rex, the review runs in the
same terminal, and the PR view opens in the sidebar.

- [What `v` runs](#what-v-runs)
- [What the Review popup runs](#what-the-review-popup-runs)
- [What `enter` runs inside Rex](#what-enter-runs-inside-rex)
- [Questions in the Rex palette](#questions-in-the-rex-palette)
- [How the PR view talks to the list](#how-the-pr-view-talks-to-the-list)
- [Open a PR from the Rex palette](#open-a-pr-from-the-rex-palette)
- [Terminal title](#terminal-title)
- [Terminal status: OSC 7501 and OSC 9;4](#terminal-status-osc-7501-and-osc-94)
- [Notice of new PRs](#notice-of-new-prs)

## What `v` runs

The review action runs nvim in the local path of the repo:

```sh
nvim -c "autocmd User CodeViewSessionClosed ++once qall" -c "Codeview pr <N>"
```

`Codeview pr <N>` opens the pull request (PR) in codeview. When you
close the codeview session, codeview sends `User CodeViewSessionClosed`,
and nvim quits. nvim gets `CHAOSHUB_AGENT_ROOT=<state dir>/agent`, so
codeview finds the agent results (see
[The result files](review-agents.md#the-result-files)).

nvim also gets `CODEVIEW_PR`: the PR data that chaoshub has, as the
JSON of `gh pr view --json` (number, title, state, draft, url, author,
branches, head and base commits, head repo and body). codeview then
shows the review without a wait for `gh pr view`, and checks the data
with gh in the background. When the head moved, codeview reads the
review again and shows a short message. An older codeview ignores the
variable. chaoshub leaves the variable out when it does not know the
base commit yet, or when the data is longer than 100 KiB.

Inside Rex, chaoshub opens a pane beside its own block with
`rex split -b $REX_BLOCK --split=<split> [--ratio <ratio>] -c <repo path>
--label "review #<N> <title>" --json -- nvim ...`. Rex shows the label
in the header of the block. A title of more than 30 characters is cut
and ends with `…`. With `zoom`, it runs `rex zoom -b <block> --on`. It
waits for the pane with `rex wait <block>`. The Rex binary is
`$REX_BIN_DIR/rex` when that file exists, else `rex` on your `PATH`.
The `[codeview]` table sets the pane (see
[Configuration](configuration.md#codeview)).

Outside Rex, or with `launcher = "inline"`, chaoshub leaves its screen,
runs nvim in the same terminal, and comes back when nvim quits. No
transition runs before or after nvim.

When the review ends, chaoshub syncs the PR again and scans the repo
again.

## What the Review popup runs

The send of codeview comments from the
[Review popup](actions.md#the-review-popup) runs codeview in a headless
nvim, in the local path of the repo:

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
`comment`, `approve` or `request-changes`. codeview opens the PR, maps
the comments to the lines of the diff of their push, posts one comment
review for each older push and then the review with the event, marks
the comments as sent in its comment file, and writes the result to
`<result>`.

A codeview that knows only version 1 refuses the request. chaoshub then
sends the comments of one push again with version 1 (`"head"` and
`"ids"` in place of `"pushes"`). For comments of more than one push,
the send then fails, and the status line asks you to update codeview.

nvim gets `CHAOSHUB_AGENT_ROOT=<state dir>/agent`, like the review of
`v`. chaoshub stops nvim after 10 minutes. codeview gives the post of a
review 2 minutes, so a stop during the post is rare.

chaoshub never posts these comments to GitHub itself. Only codeview
posts, so its lock file and its sent marks stop a second send of the
same comment, also from an open codeview. The status line then says
`posted N comments on #<number> as a <event> review`, with the count of
the comments on an older push, and names the comments that stay local
and the comments that went out already. After the send, chaoshub reads
the comment files again. See `:help codeview-bridge` in nvim for the
request and the result.

## What `enter` runs inside Rex

The PR view runs chaoshub again, in a Rex pane beside the list (see
[Where the PR view opens](pr-view.md#where-the-pr-view-opens)):

```sh
rex split -b $REX_BLOCK --split=<[detail] split> [--ratio <[detail] ratio>] \
  -c <repo path> --label "<PR overview>" --json -- \
  env CHAOSHUB_REX_LABEL=1 CHAOSHUB_LIST_SOCKET=<socket path> \
  <chaoshub> --config <config path> --no-prefetch [--theme <style>] \
  [--palette <name>] [--repo <owner/name> ...] pr <owner/name> <N>
```

The label is the PR overview of the terminal title of the PR view (see
[Terminal title](#terminal-title)), for example
`#7 Fix the merge · ready · CI passed · approved`. Rex shows it in the
header of the block. With `CHAOSHUB_REX_LABEL=1`, the PR view keeps the
label current: when the overview changes, it runs
`rex block rename [-s <session>] -- $REX_BLOCK "<PR overview>"`. It runs
one rename at a time, always with the newest overview. The label has no
activity. Before the PR loads, the label of the launch stays. A PR view
that you start yourself does not rename its block.

Because the header of the block shows the overview, the PR view with
`CHAOSHUB_REX_LABEL=1` has no title row. Its first row is the author
and the branches, and the panes get the row. The label cuts a title of
more than 30 characters. For such a title, the Description pane starts
with the full title. A PR view outside Rex, or one that you start
yourself, keeps its title row.

The pane gets the configuration file and the `--theme`, `--palette` and
`--repo` options of the list. It always gets `--no-prefetch`, because
the list already prefetches the PR of its cursor row. A repo without a
`path` runs in the directory of the list. chaoshub does not zoom the
pane. It waits for the pane with `rex wait <block>`. The
`CHAOSHUB_LIST_SOCKET` env is there only when the list listens on its
socket (see [How the PR view talks to the list](#how-the-pr-view-talks-to-the-list)).

For `open url` (`space` then `u`), the pane runs `pr <url>` in place of
`pr <owner/name> <N>`, so the end of the URL selects the first pane (see
[Open a PR from a URL](actions.md#open-a-pr-from-a-url)). For a PR that
the list does not have, the label is `<name>#N` until the PR loads, and
a repo that chaoshub does not know runs in the directory of the list.

## Questions in the Rex palette

Inside Rex, chaoshub can ask a question in the command palette of Rex
in place of its own popup. It does this for these questions:

- The author picker (`a`). Each author has the `person` icon, and
  `all authors` comes first. The current choice shows `current` at the
  end of its row. The submit row says `Filter`.
- The one-line prompts: the question to pr-agent (`a` in the Agent
  pane), the text prompt of an action, and the PR URL of `open url`
  (`space` then `u`, see
  [Open a PR from a URL](actions.md#open-a-pr-from-a-url)). The palette
  shows a text field with the title and the placeholder of the prompt,
  and the start text of the prompt, for example a PR URL from the
  clipboard. Your text goes to the same place as the text of the prompt
  popup. The submit row says `Ask` for pr-agent, `Send` for an action,
  and `Open` for the PR URL.
- The jump to a PR (`f` in the list). The palette lists the PRs that
  the tabs show now, in tab order: the filter, the author picker and
  the ready rule of each tab apply. A PR in two tabs shows once, with
  its first tab. Each row has `#N <title>`, the repo and the author,
  and the status, as in `chaoshub: open PR…`. The icon shows the tab:
  `branch` for Mine, `person` for Review and `code` for Team. The list
  holds at most 2,000 PRs, the limit of `rex.ui.pick`. The submit row
  says `Open`. When you pick a PR, chaoshub selects its tab and its
  row, and opens it as `enter` does: inside Rex, in the PR view pane.
  When Rex cannot ask, chaoshub opens the filter of the active tab
  (`/`).

Rex shows the question in each Rex app that is connected to the Rex
server, also on other devices. The first answer counts.

`rex.ui` works only in an action of the `init.lua` of the Rex server.
An action of a `rex do` script gets `unsupported`. For this reason,
[`contrib/rex/chaoshub.lua`](../contrib/rex/chaoshub.lua) registers the
action `chaoshub_ask`. Load the file from your `init.lua` (see
[Open a PR from the Rex palette](#open-a-pr-from-the-rex-palette)).
Without the action, chaoshub opens its own popups.

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

While the question waits, the status bar shows `waiting for Rex`.
chaoshub then ignores the mouse and its keys, so no second question can
start. `esc` or `q` stops the wait, and `ctrl-c` quits.

- When you answer, chaoshub does what its popup does with that answer.
- When you close the question (`cancelled`), or when you do not answer
  in time (`timeout`), chaoshub does nothing.
- When Rex cannot ask (`no_client`, `unsupported`, `busy` or `failed`),
  or when the rex command fails, chaoshub opens its own popup.

## How the PR view talks to the list

The PR view in the Rex pane is a separate chaoshub process. It tells
the list process about its changes through a local socket, so the list
does not wait for the next sync.

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
| `pending_started` | The PR view starts an action that changes GitHub, for example an approval, a merge or a reaction. The message holds the action id and its parts (see [Changes on their way to GitHub](actions.md#changes-on-their-way-to-github)). | The list keeps the parts with the same rules as its own: the rows pulse, and an approval hides the PR from the Review tab at once. |
| `pending_done` | That action ends. `ok` is false when it failed or was cancelled. | A failure ends the parts at once, so an approved PR comes back. A success keeps the parts until GitHub shows the change. |
| `pr_changed` | A review, a reply, a merge, a close, a draft change or a rerun of the checks succeeds, or a codeview review pane ends. | The list reads the PR from GitHub again. |

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

## Open a PR from the Rex palette

[`contrib/rex/chaoshub.lua`](../contrib/rex/chaoshub.lua) adds the Rex
action `chaoshub: open PR…` to the command palette. It works from any
block, also when no chaoshub list runs. The file also adds the action
`chaoshub_ask` (`chaoshub: TUI question (internal)`). The chaoshub TUI
uses it to ask its questions in the palette (see
[Questions in the Rex palette](#questions-in-the-rex-palette)). Started
from the palette, it has no question and does nothing.

1. The action runs `chaoshub list --json` (see
   [List the PRs from a script](usage.md#list-the-prs-from-a-script)).
   It reads only the cache, so the list is as old as the last sync of
   the TUI.
2. It shows the PRs with `rex.ui.pick`. Each row has `#N <title>`, the
   repo and the author, and the status. The icon shows the tab:
   `branch` for Mine, `person` for Review and `code` for Team.
3. When you pick a PR, the action opens a new Rex window with the label
   `<name>#N`. The window runs
   `env CHAOSHUB_REX_LABEL=1 chaoshub [--config <path>] pr <owner/name> <N>`
   in the `path` of the repo, or in the home directory for a repo
   without a path. The block label is the PR overview, and the PR view
   keeps it current.

If you press `esc`, or nobody answers before the time limit, the action
stops and opens nothing. If the cache has no PRs, the list shows one
row that opens nothing.

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

## Terminal title

chaoshub sets the terminal title with OSC 2. `[terminal] title` turns it
on or off (see [Configuration](configuration.md#terminal)). By default,
chaoshub sets the title only inside Rex and Ghostty.

The list title is `chaoshub · 3 PRs to review`, or
`chaoshub · 1 PR to review` for one PR. The count uses the normal
Review tab: open PRs that request review from you or your teams, are
not drafts, have passed CI, and pass the age limit. Text and author
filters and the `t` and `T` switches do not change it. Until the first
review list sync completes, `? PRs to review` shows that the count is
not known yet.

One short activity can follow the count, such as `merging #42`,
`input` or `error`. While chaoshub syncs or prefetches PRs, a spinner
follows the count instead, for example
`chaoshub · 3 PRs to review · ⠋`. The spinner moves five times per
second. The status bar names the work and shows the error details.

Rex shows its own spinner on the tab for the OSC 7501 `working` state.
For this reason, the title has no spinner when chaoshub runs in Rex
(`REX_BLOCK` is set) and writes OSC 7501 (`program_status`). While
chaoshub syncs, the title is then only `chaoshub · 3 PRs to review`.
The other activities do not change. Ghostty and other terminals keep
the title spinner.

The standalone PR view (`chaoshub pr`) shows a short overview of its PR
instead of the count of PRs to review:

```text
#7 Fix the merge · blocked · CI failed · approved
```

The parts are:

1. The PR number and the PR title. A title of more than 30 characters
   is cut and ends with `…`.
2. The verdict of the Description pane in lowercase: `ready`,
   `blocked`, `waiting`, `checking`, `conflicts`, `draft`, `in queue`,
   `merged` or `closed`.
3. The CI state: `CI passed`, `CI failed` or `CI running`. A PR without
   checks has no CI part.
4. The reviews: `approved`, `changes requested` or `review required`.
   A PR that needs no review and has no reviews has no review part.

A merged or closed PR shows only the number, the title and the verdict.
Before the PR loads, the title is `#7 · loading`. The activity follows
the overview, for example
`#7 Fix the merge · ready · CI passed · approved · merging #7`.

The title updates only when its text changes. It clears when chaoshub
exits or hands the terminal to another program, then updates again
when chaoshub resumes. It does not rename Rex sessions or windows. The
PR view pane in Rex also keeps the label of its Rex block at the
overview (see [What `enter` runs inside Rex](#what-enter-runs-inside-rex)).

## Terminal status: OSC 7501 and OSC 9;4

chaoshub tells the terminal what it does, so a Rex or Ghostty tab shows
the state without a look at the pane. OSC 7501 is the program status
(`[terminal] program_status`), and OSC 9;4 is the progress bar
(`[terminal] progress`). Every OSC 7501 record has `app=chaoshub`.

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

## Notice of new PRs

chaoshub tells you when new PRs are ready for review. The terminal
shows a notice, for example `3 team PRs ready for review`. Rex shows the
notice as a dot and a message on the tab, and as a desktop notification
with the text. The `[notify]` table sets the notice (see
[Configuration](configuration.md#notify)).

A PR is new when it becomes ready in a watched tab (`[notify] tabs`)
for the first time. Ready means open, not a draft, CI passed, and in
the `max_age` limit. These changes make a PR new:

- A colleague opens it.
- It leaves draft.
- Its CI turns green.

The author filter, the `/` filter and the `t` and `T` keys do not
change which PRs are new. When chaoshub starts, the cache and the first
sync only give the start state, so they send no notice. chaoshub keeps
the announced PRs in the cache. A restart does not announce them again.

Check runs do not change the update time of a PR, so the list sync does
not see CI that turns green. For this reason, the CI poll also reads
the CI of the watched PRs that are not green yet, at most 20 in each
poll (`[refresh] ci_secs`).

The notice waits for `[notify] debounce` (15 s by default) after a new
PR. Each new PR starts the wait again, so one notice tells you about
all of them. A steady stream gets a notice at the latest 5 debounce
times after the first new PR. The text of the notice starts with the
reason for the notice. A desktop notification can cut a long text, but
the reason stays. For one PR, the text is one of these:

- `Your PR ready for review: #123 <title>` for the Mine tab.
- `Ready for your review: #123 <title> (<author>)` for the Review tab.
- `Team PR ready for review: #123 <title> (<author>)` for the Team tab.

For more PRs, the text gives the count of each tab, in tab bar order:

- `2 of your PRs ready for review` for the Mine tab.
- `3 PRs ready for your review` for the Review tab.
- `3 team PRs ready for review` for the Team tab.
- `2 PRs ready for your review · 1 team PR ready for review` for PRs
  of more tabs.

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
In Rex, the setting "Terminal notifications"
(Settings > Notifications > System Notifications) must be on. Ghostty
shows OSC 777 too. chaoshub
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
