# Actions and reviews

The actions work on the pull request (PR) of the selected row, or on
the PR of the PR view. When an action cannot run, the status line says
why.

- [Direct keys and the actions menu](#direct-keys-and-the-actions-menu)
- [Actions](#actions)
- [Merge a stack](#merge-a-stack)
- [The Review popup](#the-review-popup)
- [Changes on their way to GitHub](#changes-on-their-way-to-github)

## Direct keys and the actions menu

In the list and in the PR view (the sidebar and the Rex pane), only the
action keys that change nothing on GitHub work directly: `v`
(codeview), `r` (the Review popup), `m` (the merge confirm popup), `o`
(browser), `y` and `i` (copy). `r` only opens the Review popup, and
only a key in the popup sends the review. `m` only opens the merge
confirm popup, and only the confirm key (`y` or `enter`) in the popup
merges. The other action keys do nothing outside the menu. Thus a wrong
key cannot send a review or merge.

A row without a PR, for example a stack header, runs no action. The key
hint bar shows the direct keys that the row can run.

The `actions` key (`space`) opens the actions menu of the selected row,
or of the PR of the PR view. The menu shows each action for the PR, with
its key. A disabled action shows the reason in parentheses. In the menu:

- `j` and `k` move the cursor.
- `enter` runs the action of the cursor.
- The key of an action runs that action. For example, `space` then `r`
  opens the Review popup.
- `esc` or `q` closes the menu.

To run another action directly in the list and in the PR view, bind a
key to `detail_<id>` in `[keys]` (see
[Configuration](configuration.md#keys)). For example,
`detail_close = "c"` makes `c` open the close confirm popup of the
selected row or of the PR view. These bindings have no default.

For the order of the keys in the panes of the PR view, see
[Keys in the PR view](pr-view.md#keys-in-the-pr-view).

## Actions

The binding is the action id. It is also the binding name for `[keys]`
and the table name for `[actions.<id>]`.

| Binding | Key | What it does |
|---|---|---|
| `review` | `v` | Reviews the PR in codeview. Needs a repo with a `path`. A second press focuses the open pane. See [What `v` runs](rex.md#what-v-runs). |
| `merge` | `m` | Merges the PR after a confirm popup. For a stacked PR, see [Merge a stack](#merge-a-stack). |
| `merge_stack` | `s` | Merges the whole stack of the PR after a confirm popup, from any entry of the stack. It is the same merge as `merge` on the top entry. Hidden for a PR without a stack. |
| `rerun_checks` | `x` | Runs the failed checks again. |
| `draft` | `d` | Changes the PR from draft to ready, or from ready to draft. The actions menu shows "mark ready" for a draft and "convert to draft" for a ready PR. |
| `close` | `c` | Closes the PR after a confirm popup. |
| `browser` | `o` | Opens the PR in the browser. |
| `copy_url` | `y` | Copies the PR URL. |
| `copy_stack_urls` | `Y` | Copies the URLs of the open PRs of the stack, one URL on each line. The order is the order of the Stack pane: position 1, the bottom of the stack, first. `[actions.copy_stack_urls] order = "top_first"` puts the top of the stack first, as the list does (see [`[actions.<id>]`](configuration.md#actionsid)). It works only in the actions menu (`space` then `Y`). Hidden for a PR without a stack. When an open entry of the stack is not loaded, the status line shows "stack not loaded, refreshing…". Try again after the refresh. |
| `copy_change_id` | `i` | Copies the jj change id of the local change that matches the PR. In a git repo, it copies the local branch name, the actions menu shows "copy branch name", and the key hint bar shows "copy branch". Hidden when no local change matches. |
| `agent` | `p` | Opens the PR view on the Agent pane. Needs a repo with a `path`. See [Review agents](review-agents.md). |
| `codeview_send` | `r` | Opens the Review popup. In the Conversation pane, `r` is the reply, so use `space` then `r` there. In the popup, you mark the codeview comments, write the message, and comment, approve or request changes (see [The Review popup](#the-review-popup)). The hint bar, the help and the actions menu show it as "send review". |
| `reply` | `r` | Only in the Conversation pane of the PR view: replies to the comment of the cursor (see [Replies](pr-view.md#replies)). The actions menu shows it only as an action of the Conversation pane, without a key, because `r` there opens the Review popup. |
| `react` | none | Only in the comment menu of the Conversation pane: adds or removes your reaction on the comment (see [Reactions](pr-view.md#reactions)). It has no key of its own, and the actions menu does not show it. |

After an action is done, chaoshub syncs the PR again and scans the repo
again.

## Merge a stack

A merge of a stacked PR includes every entry up to it. A merged entry
needs no merge, so the merge skips it. GitHub tells chaoshub which
entries merged with each PR of the stack, so a merged entry does not
have to be in the cache. When an entry is not loaded, the status line
shows "stack not loaded, refreshing…", and chaoshub reads the stack
again. Try the merge again after the refresh. When the entries below an
entry merged, but GitHub did not move the entry to the base of the
stack yet, the merge waits for the move, up to 2 min. To merge every
entry, use `merge_stack` (`s`) from any entry.

The `[merge] stack` key sets how chaoshub sends the merge (see
[`[merge]`](configuration.md#merge)). In `auto` mode, chaoshub reads
the squash defaults of the repo:

- When they are the PR title and the PR body, or when the method is
  `merge` or `rebase`, chaoshub sends one atomic request.
- Else chaoshub merges each entry itself, bottom first, each with its
  own title and body. Thus every squash commit carries its PR body.

The confirm popup says which way chaoshub merges.

## The Review popup

You write review comments in codeview (`v`). The Review popup lists the
comments that did not go to GitHub yet, and sends the marked comments
as one review. The Review popup is the only way to approve a PR, to
request changes, and to send codeview comments. chaoshub reads the
comment files of codeview (`[codeview] comments_dir`) and never writes
them.

Press `r` to open the Review popup. This works in the list and in the
PR view. In the Conversation pane, `r` is the reply, so there press
`space`, then `r`. `space`, then `r`, works in every place. The popup
opens at once with the comments that chaoshub has, and reads the
comment files again. When the read ends, the popup shows the new
comments. The popup also opens without comments, and for a repo without
a `path`. When the read fails, the popup shows the error in the list,
and approve still works.

### The comment list

The comment list is on top. Each row shows the cursor (`>`), the mark
(`[x]` marked, `[ ]` not marked), the place (`path:L12-14 (new)`, or
`PR` for a comment on the PR document), and the first line of the
comment. A `PR` comment goes into the text of the review, after your
message. Every comment starts marked, also a new comment. GitHub takes
no comment on a commit message, so such a row shows dim with `[-]`, and
the cursor skips it.

A comment that you wrote before the last push of the head is on an
older push. Its row comes after the comments of the head, with the tag
`older push`. It starts marked too, and goes out with the review.
codeview posts the comments of each older push first, in a comment
review of their own on the head commit of that push. So each comment
stays on the lines that you wrote it for. GitHub shows it as outdated
when a later push changed these lines.

Below the list, the keys are on the left, and the message is on the
right. The message wraps in its column. The popup shows up to 4 rows of
it, then `… N more lines`.

| Key | What it does |
|---|---|
| `j` / `k` | Moves the cursor down or up. The other motion keys also work, and the list scrolls. |
| `enter` | Marks or unmarks the comment of the cursor. |
| `m` | Opens the text box of the message. |
| `c` | Sends a review that adds comments only. |
| `a` | Sends a review that approves the PR. |
| `r` | Sends a review that requests changes. |
| `esc` | Closes the popup. The marks and the message stay. |

`c`, `a` and `r` send at once, with no other question. A key that
cannot send now shows dim, with the reason:

- `r` needs a message.
- `c` needs a message when no comment is marked.
- `a` and `r` show `your own PR` on your own PR, and `not open` on a
  closed or merged PR. GitHub takes no such review. `c` still works.

With no marked comment, the popup still works: the review has only the
message. chaoshub then posts the review itself.

The marks and the message stay for each PR until chaoshub quits. They
stay when you close the popup or the PR view, and the popup in the list
shows the same marks as the popup in the PR view. A review that went
out clears the message.

### The message

The text box of the message can have more than one line, and its line
breaks go to GitHub:

| Key | What it does |
|---|---|
| `enter` | Adds a new line. |
| `ctrl-e` | Opens the text in `$VISUAL`, else `$EDITOR`, else `nvim`. chaoshub leaves its screen while the editor runs. When the editor quits, its text comes back into the text box. |
| `ctrl-s` / `esc` | Goes back to the Review popup, with the text. |

### The send

When a marked comment went out from codeview in the meantime, the send
leaves it out. When every marked comment went out already, the review
goes out with only the message, so an approval does not stay behind. A
comment review without a message then has nothing to post. codeview
also finds a comment that GitHub holds already, for example after a
send that stopped before codeview wrote its sent marks, and it does not
post that comment again.

When codeview can post none of the marked comments, for example because
no comment maps to the diff, it sends no review. The status line names
the comments that stay local, with the reason, for example
`send review failed on #7 · not sent: src/a.rs:L3 (new): no line`. When
the review went out, but some marked comments stay local, the status
line shows a warning that names them with the reason.

When GitHub refuses the review of an older push, for example after a
force push, the review does not go out, and the status line shows the
error. Unmark the comments of that push to send the review.

For the command that sends the comments, see
[What the Review popup runs](rex.md#what-the-review-popup-runs).

### After an approval

When you approve a PR that requests your review, the PR leaves the
Review tab at once. chaoshub does not wait for GitHub. GitHub then
removes the review request, so the PR moves to the Team tab, and
chaoshub shows it there at once too. The title count of the Review tab
goes down at once. The notice of new PRs does not announce the PR in the
Team tab. If the approval fails, the PR comes back, and the status bar
shows the error.

The change is local only. It ends when GitHub data shows your approval
and no review request of you. A read that comes before GitHub has the
approval does not end it. If GitHub still requests your review 60 s
after the approval, the PR comes back to the Review tab: GitHub
decides.

An approval in the PR view pane (`chaoshub pr`) hides the PR in the list
at once too (see
[How the PR view talks to the list](rex.md#how-the-pr-view-talks-to-the-list)).
The Description pane of the PR view shows your review at once after the
`REVIEWS` label, for example `✓ you`, until GitHub shows it.

## Changes on their way to GitHub

An action that changes GitHub, for example an approval, a merge or a
reaction, waits for GitHub twice: for the result of the request, and
for GitHub data that shows the change. In this time, chaoshub pulses
the item that changes, in the colors of the merge badge: green to
purple and back. The pulse uses the tick of the status bar spinner and
moves only while an item pulses.

| Action | What pulses |
|---|---|
| Approve, request changes, comment review (`r`) | The title of the row in the list, and `REVIEWS` with your review (`✓ you`, `✗ you`, `you commented`) in the Description pane. |
| Review in codeview (`v`) | When the pane ends: the row and `REVIEWS`, until GitHub shows a change of your review or of the review request. |
| Merge (`m`), stack merge (`s`) | The row of each entry that the merge merges, in the list and in the Stack pane, and the badge of the Description pane. |
| Close (`c`), draft (`d`) | The row and the badge of the Description pane. |
| Rerun the checks (`x`) | The row, `CHECKS` in the Description pane, and each failed check in the Checks pane. |
| Reply (`r`) | The row, and the thread header in the Conversation pane, or a `sending your comment…` line at the end. With `[conversation] order = "newest_first"`, the line is at the top. |
| Reaction | The row, and the reaction in the Conversation pane. |

The header of the PR popup is plain text. It cannot pulse, so it shows
the spinner before the state instead, for example `⠋ draft`, during a
merge, a close or a draft change.

The item stops when the GitHub data shows the change:

- For an approval, your latest review is an approval and GitHub
  requests no review of you.
- For a merge, the PR is merged, closed or in the merge queue.
- For a reaction, you have it or not.

Data that does not show the change yet keeps the pulse. After the
result, chaoshub reads the PR again after 2 s, 7 s, 22 s, 37 s and
52 s, until the change shows and GitHub knows the merge state. If the
change does not show 60 s after the result, the pulse stops, and the
GitHub data decides. If no result comes in 30 min, the pulse stops too.

When the action fails, the pulse stops at once, the item shows the
GitHub data again, and the status line shows the error.

GitHub computes the merge state again after many changes. While it
does, the badge in the Description pane keeps the last known state,
pulses for up to 60 s and says `computing the merge state`. The badge
shows CHECKING only when no state is known.
