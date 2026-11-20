# Review agents

chaoshub runs an AI review tool on a pull request (PR) and keeps the
result on your disk. The Agent pane of the PR view and codeview show the
result. The agent writes nothing to GitHub. A result goes to GitHub only
when you take it as your own:

- In codeview, you adopt a suggestion as a local comment and submit it.
- In chaoshub, you apply a describe result with `space`, `p` and `y`.

`[agent] backend` selects the tool:

| Backend | Tools | Setup |
|---|---|---|
| `open_code_review` (default) | review | [Open Code Review](https://github.com/alibaba/open-code-review), the `ocr` CLI. |
| `pr_agent` | review, improve, describe, ask | [pr-agent](https://github.com/qodo-ai/pr-agent). |

The agent runs in the local checkout, so the repo needs a `path` (see
[`[[repo]]`](configuration.md#repo)).

- [Set up Open Code Review](#set-up-open-code-review)
- [Set up pr-agent](#set-up-pr-agent)
- [Configuration](#configuration)
- [Models for Open Code Review](#models-for-open-code-review)
- [Models for pr-agent](#models-for-pr-agent)
- [The Agent pane](#the-agent-pane)
- [What chaoshub runs for Open Code Review](#what-chaoshub-runs-for-open-code-review)
- [What chaoshub runs for pr-agent](#what-chaoshub-runs-for-pr-agent)
- [The result files](#the-result-files)

## Set up Open Code Review

1. Install ocr: `npm install -g @alibaba-group/open-code-review`. For
   the other ways, see the
   [installation docs](https://open-codereview.ai/docs/installation).
2. Configure a model: run `ocr config provider`, then
   `ocr config model`. Or set the env variables in `[agent.env]` (see
   [Models for Open Code Review](#models-for-open-code-review)).
3. Test the model with `ocr llm test`.

ocr needs git 2.41 or later. ocr reads the review rules of the repo in
`.opencodereview/rule.json` (see
[Review Rules](https://open-codereview.ai/docs/review-rules)). ocr
keeps a log of each review in `~/.opencodereview/sessions/`. To see
these logs, run `ocr viewer`.

## Set up pr-agent

1. Install pr-agent, or set `[agent.pr_agent] command` to start it,
   for example with `uvx`.
2. Set `[agent] backend = "pr_agent"`.
3. Set the model and its key (see [Models for pr-agent](#models-for-pr-agent)).

## Configuration

`backend` selects the backend. The keys at the top of `[agent]` apply
to the two backends. Each backend has its own table:
`[agent.open_code_review]` and `[agent.pr_agent]`.

```toml
[agent]
backend = "open_code_review"   # or "pr_agent"
timeout_secs = 900

[agent.env]
ANTHROPIC_API_KEY = { command = "op read op://dev/anthropic/key" }

[agent.open_code_review]
command = ["ocr"]
provider = ""
model = ""
args = []
extra_instructions = ""

[agent.pr_agent]
command = ["pr-agent"]
model = "anthropic/claude-sonnet-5-5"
fallback_models = []
extra_instructions = { review = "", improve = "", describe = "", ask = "" }
```

The keys at the top of `[agent]`:

| Key | Default | Meaning |
|---|---|---|
| `backend` | `"open_code_review"` | The backend that runs the tools: `"open_code_review"` or `"pr_agent"`. Another value is a configuration error. |
| `env` | `{}` | The env of the agent process: a variable name to a value. See [Env values](#env-values). |
| `timeout_secs` | `900` | After this time, chaoshub stops the run. |

A backend table can also set `env` and `timeout_secs`. Its `env` adds
to the `env` at the top. When the two have the same name, the value of
the backend table wins. Its `timeout_secs` replaces the value at the
top.

The keys of `[agent.open_code_review]`:

| Key | Default | Meaning |
|---|---|---|
| `command` | `["ocr"]` | The ocr program and its first arguments. A list of strings, or a string. A string is one program name: chaoshub does not split it at the spaces. |
| `provider` | `""` | The `--provider` of `ocr review`. It must name a provider that `ocr config provider` saved. Empty: ocr uses its own configuration or its env variables. |
| `model` | `""` | The `--model` of `ocr review`, for example `"claude-opus-4-6"`. Empty: the model of the ocr configuration. |
| `args` | `[]` | More arguments for `ocr review`, for example `["--effort", "high"]`. See the [CLI reference](https://open-codereview.ai/docs/cli-reference). |
| `extra_instructions` | `""` | Extra instructions for the review. chaoshub adds them to the review background, after the title and the body of the PR. |

The keys of `[agent.pr_agent]`:

| Key | Default | Meaning |
|---|---|---|
| `command` | `["pr-agent"]` | The pr-agent program and its first arguments. A list of strings, or a string. A string is one program name: chaoshub does not split it at the spaces. So write `uvx --from pr-agent pr-agent` as a list. |
| `model` | `"anthropic/claude-sonnet-5-5"` | The LiteLLM name of the model. chaoshub does not read it. It gives it to pr-agent. |
| `fallback_models` | `[]` | The LiteLLM names of the models that pr-agent tries when the model fails. |
| `extra_instructions` | `""` for each tool | Extra instructions for the model, one text per tool: `review`, `improve`, `describe` and `ask`. |

Before the backend tables, the pr-agent keys `command`, `model`,
`fallback_models` and `extra_instructions` were at the top of
`[agent]`. A configuration with these keys at the top still works: the
keys go to pr-agent. They do not select pr-agent. To keep pr-agent, set
`backend = "pr_agent"`. A key at the top and in `[agent.pr_agent]` is a
configuration error.

### Env values

A value of `env` has one of these forms:

| Form | Value |
|---|---|
| `"text"` | The text as it is. |
| `{ from_env = "NAME" }` | The value of the variable `NAME` in the env of chaoshub. A missing variable stops the run. |
| `{ command = "op read op://dev/anthropic/key" }` | The standard output of the shell command, without the spaces and the line breaks at the two ends. A command that fails stops the run, and the status line shows its error output. |

chaoshub reads each value once per run, when the run starts. chaoshub
writes no value to the log or to the screen. The process also gets the
env of chaoshub.

## Models for Open Code Review

ocr has its own provider code. It reads the model from its
configuration file, `~/.opencodereview/config.json`, or from env
variables. ocr uses the first complete source in this order: the
configuration file, the `OCR_LLM_*` variables, then the `ANTHROPIC_*`
variables of Claude Code. With `provider`, ocr reads only the
configuration file. For all the providers and keys, see the
[ocr configuration docs](https://open-codereview.ai/docs/configuration).
The examples that follow come from the same page.

A provider that `ocr config provider` saved:

```toml
[agent.open_code_review]
provider = "anthropic"
model = "claude-opus-4-6"
```

Anthropic with a key from the env of chaoshub, with no ocr
configuration file. ocr reads the `ANTHROPIC_*` variables of Claude
Code:

```toml
[agent.open_code_review.env]
ANTHROPIC_BASE_URL = "https://api.anthropic.com"
ANTHROPIC_AUTH_TOKEN = { from_env = "ANTHROPIC_API_KEY" }
ANTHROPIC_MODEL = "claude-opus-4-6"
```

An OpenAI-compatible endpoint, for example a LiteLLM proxy or Ollama,
through the `OCR_LLM_*` variables:

```toml
[agent.open_code_review.env]
OCR_LLM_URL = "http://localhost:4000/v1"
OCR_LLM_TOKEN = { from_env = "LITELLM_PROXY_KEY" }
OCR_LLM_MODEL = "claude-sonnet"
OCR_LLM_PROTOCOL = "openai"
```

ocr from npm through npx, with no install:

```toml
[agent.open_code_review]
command = ["npx", "-y", "@alibaba-group/open-code-review"]
```

## Models for pr-agent

chaoshub has no provider code for pr-agent. pr-agent reaches every
provider through LiteLLM, so a provider is only a model name and the
keys in `env`. For the names of the keys, see the pr-agent docs:
[Changing a model](https://github.com/qodo-ai/pr-agent/blob/main/docs/docs/usage-guide/changing_a_model.md).

Anthropic direct
([docs](https://github.com/qodo-ai/pr-agent/blob/main/docs/docs/usage-guide/changing_a_model.md#anthropic)):

```toml
[agent]
backend = "pr_agent"

[agent.pr_agent]
model = "anthropic/claude-sonnet-5-5"

[agent.pr_agent.env]
ANTHROPIC__KEY = { from_env = "ANTHROPIC_API_KEY" }
```

OpenAI direct
([docs](https://github.com/qodo-ai/pr-agent/blob/main/docs/docs/usage-guide/changing_a_model.md#changing-a-model-in-pr-agent)):

```toml
[agent.pr_agent]
model = "openai/gpt-6.1-sol"
fallback_models = ["openai/gpt-6-luna"]

[agent.pr_agent.env]
OPENAI__KEY = { command = "op read op://dev/openai/key" }
```

OpenRouter
([docs](https://github.com/qodo-ai/pr-agent/blob/main/docs/docs/usage-guide/changing_a_model.md#openrouter)):

```toml
[agent.pr_agent]
model = "openrouter/anthropic/claude-sonnet-5"

[agent.pr_agent.env]
OPENROUTER__KEY = { from_env = "OPENROUTER_API_KEY" }
```

An OpenAI-compatible gateway, for example a LiteLLM proxy
([pr-agent docs](https://github.com/qodo-ai/pr-agent/blob/main/docs/docs/usage-guide/changing_a_model.md#changing-a-model-in-pr-agent),
[LiteLLM proxy docs](https://docs.litellm.ai/docs/simple_proxy)). Use
the `openai/` prefix and the model id of the gateway. For a model that
pr-agent does not know, also set the input limit:

```toml
[agent.pr_agent]
model = "openai/claude-sonnet"

[agent.pr_agent.env]
OPENAI__API_BASE = "http://localhost:4000"
OPENAI__KEY = { from_env = "LITELLM_PROXY_KEY" }
CONFIG__CUSTOM_MODEL_MAX_TOKENS = "200000"
```

Ollama
([docs](https://github.com/qodo-ai/pr-agent/blob/main/docs/docs/usage-guide/changing_a_model.md#ollama)):

```toml
[agent.pr_agent]
model = "ollama/qwen2.5-coder:32b"

[agent.pr_agent.env]
OLLAMA__API_BASE = "http://localhost:11434"
CONFIG__CUSTOM_MODEL_MAX_TOKENS = "128000"
```

Other forms of the pr-agent `command`:

```toml
[agent.pr_agent]
# pr-agent from PyPI through uv, with no install.
command = ["uvx", "--from", "pr-agent", "pr-agent"]

# The Docker image. ask does not work with this form (see "What chaoshub
# runs for pr-agent"). The container gets only the env names that -e
# lists, and it must see the state directory at the same path for the
# JSON result.
command = [
  "docker", "run", "--rm",
  "-v", "/Users/me/.local/state/chaoshub:/Users/me/.local/state/chaoshub",
  "-e", "GITHUB__USER_TOKEN", "-e", "CONFIG__MODEL", "-e", "CONFIG__FALLBACK_MODELS",
  "-e", "CONFIG__PUBLISH_OUTPUT", "-e", "CONFIG__PROPAGATE_TOOL_ERRORS",
  "-e", "ANTHROPIC__KEY",
  "pragent/pr-agent:latest",
]
```

## The Agent pane

`space` then `p` on a PR opens the PR view on the Agent pane (`6`). The
pane has one section per tool. Each section shows the state (`not run`,
`running` with a spinner, `done 5m ago`, or `failed` with the error),
the model and the head commit of the result. When the head of the PR
moved after the run, the section says
`outdated: ran on 3a1f2c0, head is now 9bd2e11`. chaoshub does not run
the agent again by itself. A result of the other backend also shows the
name of that backend, for example `done 5m ago · pr-agent · ...`.

The pane has no direct keys. Its tools are in the actions menu: press
`space`, then the key of the tool.

| Key | Tool | What it shows |
|---|---|---|
| `R` | review | pr-agent: the fields of the review and the key issues, with file and lines. ocr: the number of reviewed files and one key issue per finding, the most severe first, for example `High bug src/ui/list.rs:40-44`. |
| `I` | improve | One line per suggestion: `label · file:line · summary`. pr-agent only. |
| `D` | describe | The new title and the new body. pr-agent only. |
| `a` | ask | Asks for a question first. Inside Rex, the question can open in the Rex palette (see [Questions in the Rex palette](rex.md#questions-in-the-rex-palette)). Shows the questions and the answers, newest first. pr-agent only. |
| `p` | | Applies the describe result: opens a confirm popup with the line diff of the old and the new title and body. `y` sets the title and the body of the PR on GitHub. |

With ocr, the tools that ocr does not have stay in the menu, disabled:
`I  improve (needs pr-agent)`. Their sections say `needs pr-agent`. A
press of the key shows `#7: needs pr-agent`.

One run per tool per PR runs at a time. A second `space` then `R` while
the review runs only says `ocr review #7 already running`. Different
tools, and different PRs, run at the same time.

In the menu, the action keys win over the pane keys. Thus a tool uses
an uppercase key when an action uses the lowercase letter: `r` opens
the Review popup, `i` copies the change id, and `d` changes the draft
state. `p` applies the describe result, because the menu of the Agent
pane does not show the `agent` action.

While a run goes, the status bar shows the spinner and one item per run,
for example `ocr review #7 · ocr review #9`. The last output line of
the agent shows in the status line. At the end, the status line shows
`ocr review #7 done (21k tokens)`, or the error.

## What chaoshub runs for Open Code Review

ocr reviews a diff of the local repo. It does not read the PR on
GitHub. So chaoshub first fetches the base branch and the head of the
PR from origin, to the same refs as the prefetch:
`refs/chaoshub/base/<base>` and `refs/chaoshub/pr/<n>/head`. jj does
not import these refs. Then chaoshub runs this command in the repo
path:

```sh
<command> review --repo <repo> --from <base commit> --to <head commit> \
  --format json --audience human --color never --output <tmp file> \
  --background-file <tmp file> [--provider <provider>] [--model <model>] [<args>]
```

ocr reviews the diff from the merge base of the two commits to the
head. This is the same diff as the PR on GitHub. The background file
holds the title and the body of the PR, then `extra_instructions`. ocr
uses it as the context of the review. The process gets the `env` of
`[agent]` and of `[agent.open_code_review]`.

The `[ocr]` lines on stderr are the progress. When ocr exits with an
error code, the run fails, also when ocr found some issues: ocr exits
with 1 when a part of the review failed. At `timeout_secs`, chaoshub
stops the process. A run that fails keeps the old result file.

## What chaoshub runs for pr-agent

For `review`, `improve` and `describe`, chaoshub runs this command in
the repo path:

```sh
<command> --pr_url https://github.com/<owner>/<repo>/pull/<n> --json-output <tmp file> <tool>
```

pr-agent reads its configuration from env variables in the
`SECTION__KEY` form. chaoshub sets these, after the `env` of `[agent]`
and of `[agent.pr_agent]`:

| Variable | Value |
|---|---|
| `GITHUB__USER_TOKEN` | The output of `gh auth token`. |
| `CONFIG__MODEL` | `[agent.pr_agent] model`. |
| `CONFIG__FALLBACK_MODELS` | `[agent.pr_agent] fallback_models`. |
| `CONFIG__PUBLISH_OUTPUT` | `false`: pr-agent writes nothing to the PR. |
| `CONFIG__PROPAGATE_TOOL_ERRORS` | `true`: a failed tool exits with an error code. |
| `PR_REVIEWER__EXTRA_INSTRUCTIONS`, `PR_CODE_SUGGESTIONS__EXTRA_INSTRUCTIONS`, `PR_DESCRIPTION__EXTRA_INSTRUCTIONS`, `PR_QUESTIONS__EXTRA_INSTRUCTIONS` | `[agent.pr_agent] extra_instructions` of the tool, when it is not empty. |

pr-agent reads env values as TOML, so chaoshub writes the texts in
quotes and the model list as a TOML list. A name that chaoshub sets
replaces the same name in the `env` tables. pr-agent does not accept
`config.fallback_models` as a command argument, so the env carries all
the configuration.

`--json-output` does not work for `ask`, and with
`publish_output=false`, pr-agent prints the answer nowhere. So for
`ask`, chaoshub writes the Python script
`~/.local/state/chaoshub/chaoshub-agent-ask` and runs it with the
Python of the pr-agent command:

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
  `["uvx", "--from", "pr-agent", "pr-agent"]`: chaoshub puts `python`
  in place of the last word. So `uvx --from pr-agent python <script>`
  runs the script in the env of the tool.
- `docker run ...` does not work for `ask`: the last word is the image,
  and the container cannot see the script. `review`, `improve` and
  `describe` work with Docker when the container gets the env names
  with `-e` and the state directory at the same path (see
  [Models for pr-agent](#models-for-pr-agent)).

At `timeout_secs`, chaoshub stops the process. A run that fails keeps
the old result file.

## The result files

The results are in `~/.local/state/chaoshub/agent/<owner>/<repo>/<n>/`
(`$XDG_STATE_HOME/chaoshub/agent/...`), one file per tool:
`review.json`, `improve.json`, `describe.json` and `ask.json`. chaoshub
writes a file to a temporary name first and then renames it, so a
reader never sees a part of a file. codeview reads the same files. `v`
gives codeview the root of the files in `CHAOSHUB_AGENT_ROOT` (see
[What `v` runs](rex.md#what-v-runs)).

Each file holds one JSON object:

| Field | Meaning |
|---|---|
| `version` | `1`. A reader refuses other versions. |
| `backend` | `open_code_review` or `pr_agent`. A file without this field is from pr-agent. |
| `tool` | `review`, `improve`, `describe` or `ask`. |
| `pr` | The number of the PR. |
| `head_sha` | pr-agent: the head commit of the PR when the run started. ocr: the head commit that ocr reviewed. |
| `model` | pr-agent: the LiteLLM name of the model. ocr: `<provider>/<model>`, or only the model. |
| `started_at`, `finished_at` | The times of the run, in RFC 3339. |
| `usage` | The token counts: `prompt_tokens`, `completion_tokens` and `total_tokens` of pr-agent, or `input_tokens`, `output_tokens` and `total_tokens` of ocr. `{}` without counts. |
| `data` | The JSON of pr-agent, or the ocr review in the form of a pr-agent review. For `ask`, a list of `{question, answer, head_sha, at}`, newest first. |

The `data` of pr-agent:

- review: `{review: {key_issues_to_review: [{relevant_file, issue_header, issue_content, start_line, end_line}], ...}, usage, coverage}`.
- improve: `{code_suggestions: [{relevant_file, relevant_lines_start, relevant_lines_end, existing_code, improved_code, suggestion_content, one_sentence_summary, label, score}], usage}`.
- describe: `{title, type, description, changes_diagram, pr_files: [{filename, changes_title, changes_summary, label}], usage}`.

The `data` of an ocr review has the form of a pr-agent review, so
codeview shows it in the same way:
`{review: {files_reviewed, summary, status, warnings, key_issues_to_review: [{relevant_file, issue_header, issue_content, start_line, end_line, severity, category, existing_code, suggestion_code}]}}`.
Each comment of ocr gives one key issue, the most severe first.
`issue_header` is the severity and the category, for example
`High bug`. A key issue has no `start_line` and `end_line` when ocr did
not find the lines. `summary` is the `message` of ocr. `status` is
there only when it is not a success, and `warnings` is the number of
ocr warnings.

The JSON has no markdown. chaoshub makes the text of the review from
the fields. The body that `p` applies is the `description`, then a
`Files` list with one line per entry of `pr_files`.
