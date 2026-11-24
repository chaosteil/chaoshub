# The PR view

The PR view shows one pull request (PR) in six panes: the description,
the checks, the conversation, the files, the stack and the agent
results.

- [Where the PR view opens](#where-the-pr-view-opens)
- [The panes](#the-panes)
- [Keys in the PR view](#keys-in-the-pr-view)
- [Replies](#replies)
- [Reactions](#reactions)
- [Diff colors](#diff-colors)

## Where the PR view opens

`enter` on a PR in the list opens its PR view. Where it opens depends on
`[detail] mode` (see [Configuration](configuration.md#detail)).

Inside Rex, `enter` runs `chaoshub pr OWNER/NAME N` in a Rex pane beside
the list, and the pane takes the focus. The pane shows the panes of the
PR and nothing else. `q` or `esc` in the pane closes the pane, and the
list then syncs the PR again and scans its repo. `enter` on another PR
opens a new pane for that PR, and the panes that are open stay open.
Each new pane splits the list block, with the `[detail] split` and
`[detail] ratio`. `enter` on a PR that has an open pane focuses that
pane. A quit of the list closes all its panes. A quit of a pane leaves a
codeview review from `v` open. For the command that chaoshub runs, see
[What `enter` runs inside Rex](rex.md#what-enter-runs-inside-rex).

Outside Rex, `enter` opens the sidebar. The sidebar slides in from the
right and covers 80% of the width, over the list rows. The tab bar, the
key hint bar and the status bar stay visible. `esc` or `q` closes the
sidebar.

You can also start the PR view yourself with
`chaoshub pr OWNER/NAME NUMBER`, or with `chaoshub pr URL` for a GitHub
PR URL (see [Start](usage.md#start)).

The header of the PR view shows the number and the title of the PR, then
the author, the branches and the state. When the PR has reactions, the
Description pane starts with one line that shows them, directly under
the title, as the Conversation pane shows the reactions of a comment,
for example `👍 3 (alice, bob, …)  🎉 1 (carol)` (see
[Reactions](#reactions)). Your own reactions have the primary color. A
PR without reactions has no such line. The line comes when the
conversation loads. When the line is wider than the pane, it wraps.

## The panes

The number keys select a pane, and `tab` selects the next pane:

| Key | Pane | What it shows |
|---|---|---|
| `1` | Description | The body of the PR as GitHub renders it, with tables, details and task lists. The text reflows when the pane width changes. The PR view opens on this pane, unless the end of a PR URL selects another pane (see [Start](usage.md#start)). The pane starts with a readiness block: the merge verdict and its reasons, the checks, the reviews, the open threads, the size, and the latest activity. When the PR has codeview comments that did not go to GitHub yet, one dim line below the block counts them, for example `3 unsent codeview comments (1 on an older push) · r to send` (see [The Review popup](actions.md#the-review-popup)). Without such comments, the pane shows no line. |
| `2` | Checks | One row per check, with its state and its duration. `o` opens the selected check in the browser. |
| `3` | Conversation | The comments, the reviews and the review threads, in time order, with the reactions of each comment. `[conversation] order` can put the newest at the top. `r` replies to the comment of the cursor (see [Replies](#replies)). The threads start expanded. `enter` on a thread collapses or expands it. A collapsed thread stays collapsed while the PR view is open, also when chaoshub reads the conversation again. `enter` on a comment opens the comment menu: reply, or react (see [Reactions](#reactions)). |
| `4` | Files | The changed files as a tree, with the diff of each file inline (see [Diff colors](#diff-colors)). |
| `5` | Stack | The entries of the stack. Only for a stacked PR. For a PR without a stack, the tab shows dim, and `5`, `tab` and a click do not select it. The cursor starts on the PR of the PR view. `enter` opens the PR view of the entry under the cursor on top, and `esc` goes back. `o` opens the page of that entry. |
| `6` | Agent | The agent results: one section per tool (see [The Agent pane](review-agents.md#the-agent-pane)). Only for a repo with a `path`. For a repo without a `path`, the tab shows dim, and `6`, `tab` and a click do not select it. |

`j`, `k` and the other motion keys move the cursor of the pane. In the
Description pane, they scroll the text. In the Conversation pane, the
view follows the cursor by screen rows. When a comment is taller than
the pane, `j` and `k` scroll through it before the cursor moves on.
`D` and `U` scroll the active pane by half its height, apart from the
cursor. Thus you can read a long comment or a long description. A pane
scrolls down to its last full page, not further. When a pane has more
rows than it shows, a thin scrollbar shows at its right edge. The mouse
does not use the scrollbar.

The `Enter` hint in the key hint bar shows what `enter` does on the
line of the cursor:

| Pane | Line of the cursor | Hint |
|---|---|---|
| Conversation | An expanded thread header | `Enter collapse` |
| Conversation | A collapsed thread header | `Enter expand` |
| Conversation | An issue comment, a review or a comment in a thread | `Enter menu` |
| Conversation | A status line, for example `loading the conversation…` or `no comments yet` | No hint, because `enter` does nothing there |
| Files | An open directory or file | `Enter collapse` |
| Files | A closed directory or file | `Enter expand` |
| Stack | A loaded entry | `Enter open` |
| Description, Checks, Agent | Any line | No hint, because `enter` does nothing there |

## Keys in the PR view

In the PR view, only the keys that change nothing on GitHub work
directly: the navigation keys, `R` (refresh), `v` (codeview), `r` (the
Review popup), `m` (the merge confirm popup), `o` (browser), `y` and
`i` (copy). `r` and `m` only open a popup. Only a key in the popup
sends the review or merges.

The other actions are in the actions menu. Press `space` to open it on
top of the PR view, then press the key of the action. `esc` closes the
menu, and the PR view shows again. The key hint bar shows
`Space actions`. For the list of actions, see
[Actions](actions.md#actions).

These rules give the order of the keys:

- Outside the menu, the pane keys, the motion keys and the keys of the
  active pane come first. For example, in the Checks pane, `o` opens
  the selected check, not the PR. In the Conversation pane, `r` opens
  the text box of a reply, not the Review popup.
- In the menu, an action key wins over a pane key. Thus `space` then
  `r` opens the Review popup in every pane, also in the Conversation
  pane.

The actions menu also lists the own actions of the active pane, after
the actions of the PR: the agent tools of the Agent pane, and the reply
of the Conversation pane. In the Agent pane, `space` then `r` opens the
Review popup, and `space` then `R` starts the agent review. The menu of
the Agent pane does not show the `agent` action, because the pane shows
already. To bind a direct key for an action in the PR view, see
[Actions](actions.md#actions).

## Replies

In the Conversation pane, `r` opens a text box for a reply to the
comment of the cursor. The key hint bar shows `r reply`. The actions
menu also shows the reply, without a key, because `space` then `r`
opens the Review popup. Move the cursor to it and press `enter` to
reply. The place of the reply depends on the line of the cursor:

| Cursor on | The reply |
|---|---|
| A review thread, or a comment in an expanded thread | Goes into the thread. The text box starts empty. |
| An issue comment or a review | Is a new comment on the PR. The text box starts with a quote of the author and the first line, for example `> @alice: Looks good.`, then an empty line. A comment without text gives `@alice` instead. |
| Another line, for example `no comments yet` | Is a new comment on the PR. The text box starts empty. |

The text box has the keys of the text box of the review message:

| Key | What it does |
|---|---|
| `enter` | Adds a new line. |
| `ctrl-e` | Opens the text in `$VISUAL`, else `$EDITOR`, else `nvim`. When the editor quits, its text comes back into the text box. |
| `ctrl-s` | Sends the reply. A text that is empty, or that is still the start text, sends nothing. The status line then says `empty reply to #7, not sent`. |
| `esc` | Closes the text box. The text is lost. |

When the reply went out, the status line says so, for example
`commented on #7` or `replied on #7 at src/lib.rs:12`, and chaoshub
reads the conversation again. Then the reply shows in the pane. When
GitHub refuses the reply, the status line shows the error.

## Reactions

The Conversation pane shows the reactions of a comment or a review on
one row under its body, in the order of GitHub, for example
`👍 3 (alice, bob, …)  🎉 1 (carol)`. Your own reactions have the
primary color. A comment without reactions has no such row.

Each reaction shows the number of people who reacted and the logins of
the first two of them, in the order of GitHub. With one or two people,
the reaction names all of them. With three or more, a `…` comes after
the two logins. The logins come in the same GitHub request as the
counts.

A comment in a review thread shows only the counts, for example
`👍 3  🎉 1`. The PR view reads up to 100 threads of 50 comments each.
Logins for all these comments raise the cost of the request from 3 to
53 GraphQL points of the rate limit. When a row is wider than the pane,
the pane cuts it at the right edge.

`enter` on an issue comment, a review or a comment in an expanded
thread opens the comment menu. `enter` on a thread header still
collapses or expands the thread. A click on the selected comment also
opens the menu.

| Key in the menu | What it does |
|---|---|
| `r` | Opens the text box of a reply, as `r` in the pane (see [Replies](#replies)). |
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
until GitHub shows it (see
[Changes on their way to GitHub](actions.md#changes-on-their-way-to-github)).
A removed reaction with no count left shows as `👍 0` while it pulses.
When the reaction went out, the status line says so, for example
`reacted 🎉 on #7` or `removed 👍 on #7`, and chaoshub reads the
conversation again. When GitHub refuses the reaction, the pane shows
the old reactions again, and the status line shows the error, for
example `react failed on #7: offline`.

## Diff colors

The Files pane shows the diff of an open file in the style of the diff
view of Pierre ([diffs.com](https://diffs.com)). The colors are keys of
the palette:

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

### Changed words

A run of removed lines that a run of added lines follows directly is
one change. Its lines pair up by position, as on GitHub: the first
removed line with the first added line, and so on. In each pair, the
words that changed get a stronger tint, as in the Pierre diff view. The
text keeps its color. A word is a run of letters, digits and `_`, or
one other sign. Two changed words that one character divides show as
one mark.

A pair gets no marks in these cases:

- One of its lines is longer than 1000 characters.
- Less than half of the text of the two lines is equal.
- One side of its change has more than three times the lines of the
  other side. Then no pair of the change gets marks.

On the cursor line, the selection band hides the marks.

### Syntax colors

The code of each line has the syntax colors of its language. chaoshub
finds the language from the file name or the extension, with
[syntect](https://github.com/trishume/syntect) and the syntaxes of
[bat](https://github.com/sharkdp/bat) that
[two-face](https://codeberg.org/CosmicHarper/two-face) bundles. The set
has more than 200 languages, for example Rust, Go, Python, TypeScript
and TSX, JavaScript, TOML, Nix, Dockerfile, YAML, JSON, Markdown and
shell. The `+` and the `-` keep the green and the red of the change.
The token colors are the `syntax` keys of the palette, from the
`pierre-dark` theme:

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
some milliseconds. chaoshub keeps the result until the PR gets a new
head. The worker also finds the changed words.

These limits apply. In each case, the diff colors stay:

- A file gets no syntax colors when no bundled syntax knows its
  language.
- A file gets no syntax colors and no marks of changed words when its
  patch is larger than 128 KiB.
- A line that is longer than 1000 characters gets no syntax colors.
