-- Probe of the bench harness. Load it with `nvim --cmd "luafile probe.lua"`.
--
-- The probe writes the time of each stage of a codeview open into the file
-- $BENCH_OUT, as one JSON object. Each time is in milliseconds after
-- $BENCH_T0, the wall clock in nanoseconds before the start of the run.
-- The probe quits Neovim when every stage of $BENCH_WAIT has its time (by
-- default the review comments), or after $BENCH_TIMEOUT milliseconds.

local t0 = tonumber(vim.env.BENCH_T0 or "")
local out = vim.env.BENCH_OUT
if not t0 or not out or out == "" then
  return
end
local timeout = tonumber(vim.env.BENCH_TIMEOUT or "") or 20000

local function now()
  local sec, nsec = vim.uv.clock_gettime("realtime")
  if type(sec) == "table" then
    sec, nsec = sec.sec, sec.nsec
  end
  return (sec * 1e9 + nsec - t0) / 1e6
end

local marks = { start = now() }

-- A pane of a Rex server without a client has 80x24 cells. The summary line
-- of codeview is longer, and the hit-enter prompt then stops the open. The
-- pane of the user is wider, so the probe turns the prompt off.
vim.o.messagesopt = "wait:0,history:500"
local done = false

local function finish(reason)
  if done then
    return
  end
  done = true
  marks.reason = reason
  local file = io.open(out, "w")
  if file then
    file:write(vim.json.encode(marks), "\n")
    file:close()
  end
  vim.cmd("qall!")
end

local function mark(name)
  if marks[name] == nil then
    marks[name] = now()
  end
end

local group = vim.api.nvim_create_augroup("bench.probe", { clear = true })
vim.api.nvim_create_autocmd("VimEnter", {
  group = group,
  callback = function()
    mark("vimenter")
  end,
})

local events = {
  CodeViewSessionOpened = "opened",
  CodeViewFileOpened = "paint",
  CodeViewRemoteLoaded = "comments",
  CodeViewPrChecked = "checked",
}
-- The run ends when every stage of $BENCH_WAIT has its time.
local wait = vim.split(vim.env.BENCH_WAIT or "comments", ",", { trimempty = true })

vim.api.nvim_create_autocmd("User", {
  group = group,
  pattern = vim.tbl_keys(events),
  callback = function(event)
    mark(events[event.match])
    for _, name in ipairs(wait) do
      if marks[name] == nil then
        return
      end
    end
    -- The open can send more events in the same loop. Wait for them.
    vim.defer_fn(function()
      finish(wait[#wait])
    end, 50)
  end,
})

-- An error of the open shows as a message. Keep it in the result.
vim.api.nvim_create_autocmd("User", {
  group = group,
  pattern = "CodeViewSessionClosed",
  callback = function()
    mark("closed")
  end,
})

vim.defer_fn(function()
  local messages = vim.api.nvim_exec2("messages", { output = true }).output
  marks.messages = messages
  finish("timeout")
end, timeout)
