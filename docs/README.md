# chaoshub documentation

chaoshub is a terminal app that tracks your GitHub pull requests (PRs).
For a short overview, see the [README](../README.md). These pages
describe every feature.

| Page | What it tells |
|---|---|
| [Install](install.md) | The requirements, jj and git, and each way to install chaoshub. |
| [Usage](usage.md) | The start, the command line, the tabs, the list columns, the hidden rows, the filter, the app keys, the mouse, the status bar, the toast, `chaoshub list` and the files of chaoshub. |
| [The PR view](pr-view.md) | Where the PR view opens, its panes and keys, replies, reactions and the diff colors. |
| [Actions and reviews](actions.md) | The actions and their keys, the stack merge, the Review popup for codeview comments, and the pulse of changes on their way to GitHub. |
| [Configuration](configuration.md) | Every key of `config.toml`. |
| [Review agents](review-agents.md) | The AI review with Open Code Review or pr-agent: the setup, the models, the Agent pane and the result files. |
| [Rex, Ghostty and the terminal](rex.md) | What chaoshub runs for codeview and Rex, the Rex palette, the terminal title, the terminal status and the notice of new PRs. |
| [Development](development.md) | The checks, how to add an action, a tab or a pane, the benchmark and the screenshots. |

The words of these pages:

- **The list**: the tabs Mine, Review and Team, with one row for each
  PR.
- **The PR view**: the panes of one PR. Inside Rex, it opens in a Rex
  pane beside the list. Outside Rex, it opens in the sidebar.
  `chaoshub pr OWNER/NAME NUMBER` or `chaoshub pr URL` starts it
  alone.
- **The actions menu**: the menu that `space` opens, with every action
  for a PR.
- **The Review popup**: the popup that `r` opens. It sends a review to
  GitHub, with your codeview comments.
