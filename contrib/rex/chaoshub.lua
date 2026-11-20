-- The Rex palette action "chaoshub: open PR…", and the action
-- "chaoshub_ask" that the chaoshub TUI uses to ask its questions in the
-- palette.
--
-- The action reads the pull requests of the chaoshub tabs with
-- `chaoshub list --json`, shows them in the command palette with
-- rex.ui.pick, and opens the chosen pull request in a new Rex window. The
-- window runs `chaoshub pr OWNER/NAME N`, the PR view that `enter` opens
-- in the list.
--
-- Load this file from ~/.config/rex/init.lua:
--
--   dofile("/path/to/chaoshub.lua").setup()
--
-- setup takes a table with these optional fields:
--
--   bin     The chaoshub program. Defaults to "chaoshub". The Rex server
--           runs `chaoshub list` without a shell, so the program must be
--           on the PATH of the server, or bin must be a full path.
--   config  The chaoshub configuration file, as with `--config`.
--
-- The action is "chaoshub_open_pr". To bind a key to it, add for example:
--
--   rex.bind("cmd+shift+o", "chaoshub_open_pr")
--
-- See "Open a PR from the Rex palette" and "Questions in the Rex palette"
-- in docs/rex.md of the chaoshub repo.

local M = {}

local SHELL = "com.superlogical.terminal.shell"

-- The icon of each tab, from the fixed icon set of rex.ui.pick.
local ICONS = { mine = "branch", review = "person", team = "code" }

-- The seconds that `chaoshub list` can take. It reads only the cache.
local LIST_TIMEOUT = 10

-- The id of the item that an empty list shows. A pull request id always
-- has a "#", so this id is never one.
local EMPTY_ID = "empty"

-- The chaoshub argv with the global options of `opts`, then `args`.
local function argv(opts, args)
  local out = { opts.bin }
  if opts.config then
    out[#out + 1] = "--config"
    out[#out + 1] = opts.config
  end
  for _, arg in ipairs(args) do out[#out + 1] = arg end
  return out
end

-- The pull requests of `chaoshub list --json`. Raises an error with the
-- reason when the command fails.
local function list(opts)
  local r, err = rex.exec(argv(opts, { "list", "--json" }), { timeout = LIST_TIMEOUT })
  if r == nil then error("chaoshub: " .. tostring(err), 0) end
  if not r.ok then
    local why = r.stderr ~= "" and r.stderr or ("exit code " .. tostring(r.code))
    error("chaoshub list failed: " .. why, 0)
  end
  local listing, derr = rex.json.decode(r.stdout)
  if listing == nil then error("chaoshub list: the JSON does not decode: " .. tostring(derr), 0) end
  return listing.prs or {}
end

-- The picker item of a pull request. The id is `owner/name#N`.
local function item(pr)
  return {
    id = pr.repo .. "#" .. pr.number,
    title = "#" .. pr.number .. " " .. pr.title,
    subtitle = pr.repo .. " · " .. pr.author,
    detail = pr.status,
    icon = ICONS[pr.tab],
  }
end

-- Opens `pr` in a new window and shows that window. The PR view keeps
-- the label of its block at the PR overview, as in the detail pane of
-- the list (CHAOSHUB_REX_LABEL=1).
local function open(ctx, opts, pr)
  local command = { "env", "CHAOSHUB_REX_LABEL=1" }
  for _, arg in ipairs(argv(opts, { "pr", pr.repo, tostring(pr.number) })) do
    command[#command + 1] = arg
  end
  local name = pr.repo:match("[^/]+$") or pr.repo
  local w, err = rex.session.new_window{
    window_label = name .. "#" .. pr.number,
    layout = {
      block = {
        flavor = SHELL,
        label = pr.label,
        -- Without a path, the block starts in the home directory.
        options = { command = command, cwd = pr.path },
      },
    },
  }
  if w == nil then error("chaoshub: cannot open a window: " .. tostring(err), 0) end
  -- From the palette, the old window stays active. Show the new one.
  -- There is no client when the action runs from the command line.
  rex.session.focus_window{ window_id = w.window_id }
  rex.session.focus_block{ block_id = w.block_ids[1] }
  pcall(rex.client.queue, "session.select", {
    session_id = ctx.session_id,
    window_id = w.window_id,
  })
  return w
end

-- The run function of "chaoshub_ask". The chaoshub TUI runs
--
--   rex do chaoshub_ask --args - --json
--
-- with {"ask": {...}, "timeout": N} on stdin. `ask.kind` is "input" or
-- "pick", and the other fields of `ask` are the ones of rex.ui: title,
-- placeholder, value and confirm, and for a pick items and allow_other.
-- The result is {answer = "..."} with the text or the id of the picked
-- item, {other = "..."} with the typed text of a pick with allow_other,
-- or {reason = "..."} with the reason of rex.ui when no answer came:
-- cancelled, timeout, no_client, unsupported, busy or failed.
local function ask(_, args)
  local q = args.ask
  -- The action also shows in the palette. Started from there, it has no
  -- question.
  if type(q) ~= "table" then return { reason = "failed" } end
  local fields = {
    title = q.title,
    placeholder = q.placeholder,
    value = q.value,
    confirm = q.confirm,
    timeout = args.timeout,
  }
  local answer, why
  if q.kind == "pick" then
    fields.items = q.items
    fields.allow_other = q.allow_other
    answer, why = rex.ui.pick(fields)
  else
    answer, why = rex.ui.input(fields)
  end
  if type(answer) == "string" then return { answer = answer } end
  -- With allow_other, a typed text comes as { text = "..." }.
  if type(answer) == "table" and type(answer.text) == "string" then
    return { other = answer.text }
  end
  return { reason = why or "failed" }
end

-- Registers the actions. See the top of this file for `opts`.
function M.setup(opts)
  opts = opts or {}
  local o = { bin = opts.bin or "chaoshub", config = opts.config }
  rex.action{
    name = "chaoshub_open_pr",
    title = "chaoshub: open PR…",
    description = "Open a PR from the chaoshub tabs in a new window.",
    category = "chaoshub",
    keywords = "pull request github review pr",
    run = function(ctx)
      local prs = list(o)
      local items, by_id = {}, {}
      for _, pr in ipairs(prs) do
        local it = item(pr)
        items[#items + 1] = it
        by_id[it.id] = pr
      end
      -- An empty Lua table becomes a JSON object, not a list. Thus an
      -- empty cache gets one item that opens nothing.
      if #items == 0 then
        items[1] = {
          id = EMPTY_ID,
          title = "No PRs in the cache",
          subtitle = "Start chaoshub to sync",
        }
      end
      local id, why = rex.ui.pick{
        title = "chaoshub: open PR",
        placeholder = "Filter PRs",
        confirm = "Open",
        items = items,
      }
      -- No answer: cancelled, timeout, no client and the other reasons.
      if id == nil then return { opened = false, reason = why } end
      local pr = by_id[id]
      if pr == nil then return { opened = false, reason = "empty" } end
      local w = open(ctx, o, pr)
      return { opened = true, pr = id, window_id = w.window_id }
    end,
  }
  rex.action{
    name = "chaoshub_ask",
    title = "chaoshub: TUI question (internal)",
    description = "The chaoshub TUI runs this action to ask one question in the palette.",
    category = "chaoshub",
    args = { ask = "object?", timeout = "integer?" },
    run = ask,
  }
end

return M
