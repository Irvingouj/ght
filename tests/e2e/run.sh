#!/usr/bin/env bash
# End-to-end tests: drive the real ght binary inside headless tmux (WSL) and
# zellij (Windows), against a fake GitHub on 127.0.0.1 with fixed fixture data.
#
#   bash tests/e2e/run.sh [tmux|zellij|all]
#
# Needs Windows with Git Bash, WSL Ubuntu with tmux, zellij, nvim and cargo.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
WORK="$REPO/target/e2e"
PORT=18765
win() { cygpath -w "$1"; }

setup() {
  (cd "$REPO" && cargo build --release --quiet && cargo build --release --examples --quiet) || exit 1
  rm -rf "$WORK"; mkdir -p "$WORK/fakebin" "$WORK/root"
  cp "$REPO/target/release/examples/fake_gh.exe" "$WORK/fakebin/gh.exe"
  printf '@echo %%* >> "%s"\r\n' "$(win "$WORK")\\opened.txt" > "$WORK/browser.cmd"
  printf '@findstr "^" > "%s"\r\n' "$(win "$WORK")\\clipboard.txt" > "$WORK/clip.cmd"

  # A clone of acme/widgets on feat/evict (#11), with feat/cache (#10) in a worktree.
  local clone="$WORK/root/widgets"
  git init -q -b main "$clone"
  git -C "$clone" -c user.name=t -c user.email=t@t commit -q --allow-empty -m init
  git -C "$clone" remote add origin git@github.com:acme/widgets.git
  git -C "$clone" checkout -q -b feat/evict
  git -C "$clone" worktree add -q -b feat/cache "$WORK/root/widgets.worktrees/cache" main

  cat > "$WORK/launch.cmd" <<EOF
@echo off
set "PATH=$(win "$WORK/fakebin");%PATH%"
set "GHT_API_URL=http://127.0.0.1:$PORT"
set "GHT_FIXTURE=$(win "$HERE/fixture.json")"
set "BROWSER=$(win "$WORK/browser.cmd")"
set "GHT_CLIPBOARD=$(win "$WORK/clip.cmd")"
set "EDITOR=nvim --clean"
set "GHT_CACHE_DIR=$(win "$WORK/cache")"
cd /d "$(win "$clone")"
"$(win "$REPO/target/release/ght.exe")"
EOF
  serve 800
}

# Every fake response waits $1 ms, so in-between states stay on screen long enough to check.
serve() {
  [ -f "$WORK/fake_github.pid" ] && kill "$(cat "$WORK/fake_github.pid")" 2>/dev/null && sleep 0.5
  "$REPO/target/release/examples/fake_github.exe" "$HERE/fixture.json" "$PORT" "$1" >> "$WORK/fake_github.log" 2>&1 &
  echo $! > "$WORK/fake_github.pid"
  sleep 1
}

teardown() {
  kill "$(cat "$WORK/fake_github.pid")" 2>/dev/null
}

run_with() {
  source "$HERE/drivers/$1.sh"
  source "$HERE/scenario.sh"
  echo "################ $1"
  serve 800; start 150 40; run_main
  # A slow refresh proves the cached list shows first.
  serve 4000; start 120 18; run_scroll
  stop
  echo "## $1: $PASS passed, $FAIL failed"
  TOTAL_FAIL=$((TOTAL_FAIL + FAIL))
}

TOTAL_FAIL=0
setup
trap teardown EXIT
case "${1:-all}" in
  tmux) run_with tmux ;;
  zellij) run_with zellij ;;
  all) run_with tmux; run_with zellij ;;
esac
exit $((TOTAL_FAIL > 0))
