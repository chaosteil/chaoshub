#!/usr/bin/env bash
# Times the open of a pull request review in codeview.
#
# Usage: bench/bench.sh <command> [options]
#
# Commands:
#   setup     Clone the repo into $BENCH_DIR: a warm clone, and a template
#             for the stale base case.
#   headless  Run nvim headless, without a terminal.
#   rex       Run nvim in a pane of a private Rex server, with split and zoom
#             as chaoshub runs them.
#   network   Time the GitHub calls alone.
#   clean     Remove $BENCH_DIR.
#
# Options (environment):
#   BENCH_DIR      Directory of the clones and the results. Required.
#   BENCH_REPO     Repository, as owner/name. Default chaosteil/chaoshub.
#   BENCH_PR       Number of the pull request. Default 1.
#   BENCH_RUNS     Runs per case. Default 5.
#   BENCH_CASE     warm or stale. Default warm.
#   BENCH_CONFIG   full (the Neovim configuration of the user) or plugin
#                  (codeview alone). Default full.
#   BENCH_CODEVIEW Directory of codeview for the plugin configuration.
#   BENCH_PRDATA   1 gives nvim the pull request data in $CODEVIEW_PR, as
#                  chaoshub does. Default 0.
#   BENCH_WAIT     Events that end a run: comments, or comments,checked.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
PROBE="$HERE/probe.lua"
: "${BENCH_DIR:?set BENCH_DIR to a scratch directory}"
REPO=${BENCH_REPO:-chaosteil/chaoshub}
PR=${BENCH_PR:-1}
RUNS=${BENCH_RUNS:-5}
CASE=${BENCH_CASE:-warm}
CONFIG=${BENCH_CONFIG:-full}
WAIT=${BENCH_WAIT:-comments}
NAME=${REPO#*/}
WARM="$BENCH_DIR/$NAME-warm"
STALE="$BENCH_DIR/$NAME-stale"
RESULTS="$BENCH_DIR/results"

now() { date +%s%N; }
ms() { echo $((($2 - $1) / 1000000)); }

# The nvim command line of chaoshub, with the probe in front.
nvim_args() {
  local args=(nvim)
  if [[ $CONFIG == plugin ]]; then
    : "${BENCH_CODEVIEW:?set BENCH_CODEVIEW for the plugin configuration}"
    args+=(--clean -i NONE --cmd "set rtp^=$BENCH_CODEVIEW")
  fi
  args+=(--cmd "luafile $PROBE")
  if [[ $CONFIG == plugin ]]; then
    args+=(-c "runtime plugin/codeview.lua")
  fi
  args+=(-c "autocmd User CodeViewSessionClosed ++once qall" -c "Codeview pr $PR")
  printf '%s\0' "${args[@]}"
}

# The env words of one run: the chaoshub env, the probe, and the pull
# request data.
env_words() {
  local out=$1 t0=$2
  local words=("CHAOSHUB_AGENT_ROOT=$BENCH_DIR/agent" "BENCH_OUT=$out" "BENCH_WAIT=$WAIT")
  if [[ -n $t0 ]]; then
    words+=("BENCH_T0=$t0")
  fi
  if [[ ${BENCH_PRDATA:-0} == 1 ]]; then
    words+=("CODEVIEW_PR=$(cat "$BENCH_DIR/prdata.json")")
  fi
  printf '%s\0' "${words[@]}"
}

# The clone of one run. A stale run gets a fresh copy of the template.
prepare() {
  case $CASE in
  warm) echo "$WARM" ;;
  stale)
    rm -rf "$STALE-run"
    cp -a "$STALE" "$STALE-run"
    echo "$STALE-run"
    ;;
  *)
    echo "unknown case $CASE" >&2
    exit 1
    ;;
  esac
}

# Median of the numbers on stdin.
median() { sort -n | awk '{a[NR]=$1} END {if (NR) print a[int((NR+1)/2)]; else print "-"}'; }

# Medians of every stage of a result file.
summary() {
  local file=$1
  echo "== $(basename "$file") ($(wc -l <"$file") runs)"
  for stage in launch start vimenter opened paint comments checked; do
    local values
    values=$(jq -r --arg s "$stage" '.[$s] // empty | floor' "$file" | median)
    printf '  %-9s %s ms\n' "$stage" "$values"
  done
  jq -r 'select(.reason != "comments" and .reason != "checked") | "  run ended by \(.reason): \(.messages // "")"' "$file" | head -5
}

cmd_setup() {
  mkdir -p "$BENCH_DIR" "$RESULTS" "$BENCH_DIR/agent"
  if [[ ! -d $WARM ]]; then
    git clone -q "git@github.com:$REPO" "$WARM"
  fi
  git -C "$WARM" fetch -q origin "refs/pull/$PR/head"
  local head base merge
  head=$(gh pr view "$PR" --repo "$REPO" --json headRefOid --jq .headRefOid)
  base=$(gh pr view "$PR" --repo "$REPO" --json baseRefOid --jq .baseRefOid)
  git -C "$WARM" fetch -q origin "$base"
  merge=$(git -C "$WARM" merge-base "$base" "$head")
  # The template holds the head and the merge base, not the tip of the base
  # branch. It gets its objects from the warm clone, not from GitHub.
  rm -rf "$STALE"
  git clone -q --no-local --no-checkout "$WARM" "$STALE"
  git -C "$STALE" remote set-url origin "git@github.com:$REPO"
  git -C "$STALE" fetch -q "$WARM" "$head:refs/bench/head"
  git -C "$STALE" update-ref refs/remotes/origin/main "$merge"
  git -C "$STALE" update-ref refs/heads/main "$merge"
  git -C "$STALE" for-each-ref --format='%(refname)' refs/remotes/origin |
    grep -v '^refs/remotes/origin/main$' | while read -r ref; do git -C "$STALE" update-ref -d "$ref"; done
  git -C "$STALE" reflog expire --expire=now --all
  git -C "$STALE" gc -q --prune=now
  if git -C "$STALE" cat-file -e "$base^{commit}" 2>/dev/null; then
    echo "the template still holds the base $base" >&2
    exit 1
  fi
  # The fields that chaoshub gives, in the form of gh.
  gh pr view "$PR" --repo "$REPO" --json \
    number,title,state,isDraft,url,author,headRefName,baseRefName,headRefOid,baseRefOid,headRepository,isCrossRepository,body \
    --jq '.author = {login: .author.login} | .headRepository = {nameWithOwner: .headRepository.nameWithOwner}' \
    >"$BENCH_DIR/prdata.json"
  echo "warm clone: $WARM"
  echo "stale template: $STALE (base $base missing, merge base $merge)"
}

cmd_headless() {
  local file
  file="$RESULTS/headless-$CONFIG-$CASE-$(date +%H%M%S).jsonl"
  mkdir -p "$RESULTS"
  for _ in $(seq "$RUNS"); do
    local dir out t0
    dir=$(prepare)
    out=$(mktemp "$BENCH_DIR/out.XXXXXX")
    mapfile -d '' -t args < <(nvim_args)
    t0=$(now)
    mapfile -d '' -t words < <(env_words "$out" "$t0")
    (cd "$dir" && env "${words[@]}" "${args[0]}" --headless "${args[@]:1}") >/dev/null 2>&1 || true
    cat "$out" >>"$file"
    rm -f "$out"
  done
  summary "$file"
}

cmd_rex() {
  local file
  file="$RESULTS/rex-$CONFIG-$CASE-$(date +%H%M%S).jsonl"
  # A Unix socket path must be short, so it does not go into $BENCH_DIR.
  local sock=${BENCH_SOCK:-/tmp/chaoshub-bench-$UID.sock} state="$BENCH_DIR/rex-state"
  mkdir -p "$RESULTS" "$state"
  local rex=(env -u REX_BLOCK -u REX_SESSION -u REX_CLIENT rex)
  # A private server: the panes of the bench never reach the Rex of the user.
  "${rex[@]}" server run --listen "unix://$sock" --state-dir "$state" --restore=false \
    >"$BENCH_DIR/rex.log" 2>&1 &
  local server=$!
  # shellcheck disable=SC2064
  trap "kill $server 2>/dev/null || true; wait $server 2>/dev/null || true; rm -rf '$state' '$sock'" RETURN
  rex+=(-S "unix://$sock" --autostart=false)
  for _ in $(seq 50); do
    "${rex[@]}" ls >/dev/null 2>&1 && break
    sleep 0.1
  done
  local created anchor session
  created=$("${rex[@]}" new bench --json -- sleep 3600)
  session=$(jq -r .session_id <<<"$created")
  anchor=$(jq -r '.initial_windows[0].block_ids[0]' <<<"$created")
  for i in $(seq "$RUNS"); do
    local dir out t0 t1 t2 split_out block
    dir=$(prepare)
    out=$(mktemp "$BENCH_DIR/out.XXXXXX")
    mapfile -d '' -t args < <(nvim_args)
    t0=$(now)
    mapfile -d '' -t words < <(env_words "$out" "$t0")
    # The same calls as the Rex launcher of chaoshub: split, then zoom.
    split_out=$("${rex[@]}" split -b "$anchor" -s "$session" --split=right -c "$dir" \
      --label "bench-$i" --json -- env "${words[@]}" "${args[@]}")
    t1=$(now)
    block=$(jq -r '.block_ids[0]' <<<"$split_out")
    "${rex[@]}" zoom -b "$block" --on >/dev/null
    t2=$(now)
    for _ in $(seq 300); do
      [[ -s $out ]] && break
      sleep 0.1
    done
    "${rex[@]}" block close "$block" -s "$session" >/dev/null 2>&1 || true
    if [[ -s $out ]]; then
      jq -c --argjson split "$(ms "$t0" "$t1")" --argjson zoom "$(ms "$t1" "$t2")" \
        '. + {launch: ($split + $zoom), split: $split, zoom: $zoom}' "$out" >>"$file"
    fi
    rm -f "$out"
  done
  "${rex[@]}" kill "$session" >/dev/null 2>&1 || true
  echo "  split/zoom: $(jq -r .split "$file" | median) / $(jq -r .zoom "$file" | median) ms"
  summary "$file"
}

cmd_network() {
  local owner=${REPO%/*} name=${REPO#*/} fields
  fields=number,title,state,isDraft,url,author,headRefName,baseRefName,headRefOid,baseRefOid,headRepository,headRepositoryOwner,isCrossRepository,body
  # shellcheck disable=SC2016
  local query='query($owner:String!,$name:String!,$number:Int!){repository(owner:$owner,name:$name){pullRequest(number:$number){reviewThreads(first:50){nodes{id}}}}}'
  run_net() {
    local label=$1
    shift
    local values=()
    for _ in $(seq "$RUNS"); do
      local t0
      t0=$(now)
      "$@" >/dev/null 2>&1 || true
      values+=("$(ms "$t0" "$(now)")")
    done
    printf '  %-28s %s ms\n' "$label" "$(printf '%s\n' "${values[@]}" | median)"
  }
  echo "== network ($RUNS runs)"
  run_net "gh pr view (fields)" gh pr view "$PR" --repo "$REPO" --json "$fields,commits"
  run_net "gh pr view (no commits)" gh pr view "$PR" --repo "$REPO" --json "$fields"
  run_net "gh api pulls/N/comments" gh api "repos/$REPO/pulls/$PR/comments?per_page=100"
  run_net "gh api graphql threads" gh api graphql -f query="$query" -F owner="$owner" -F name="$name" -F number="$PR"
  run_net "git ls-remote" git ls-remote "git@github.com:$REPO" "refs/pull/$PR/head"
  run_net "nvim startup (full config)" nvim --headless -i NONE +qa
}

cmd_clean() {
  rm -rf "$BENCH_DIR"
}

case ${1:-} in
setup | headless | rex | network | clean) "cmd_$1" ;;
*)
  sed -n '2,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//'
  exit 1
  ;;
esac
