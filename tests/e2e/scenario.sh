# Scenarios against tests/e2e/fixture.json. Needs a driver sourced first:
# c (type chars), k (press key), d (screen text), cur (cursor row), K_* key names.
#
# ght starts in a clone of acme/widgets, logged in as alice. With drafts hidden the list is:
#   0 ▾ acme/widgets (this repo)   1 #10   2 └─ #11   3 #14
#   4 ▾ acme/gadgets               5 #3    6 #4 (draft, kept: #5 builds on it)   7 └─ #5
#   8 ▾ acme/zeta-old              9 #1
PASS=0
FAIL=0
ok() { PASS=$((PASS + 1)); echo "  PASS $1"; }
bad() { FAIL=$((FAIL + 1)); echo "  FAIL $1"; }
step() { echo "== $1"; }
expect() { d > "$WORK/scr.txt"; grep -qF -- "$1" "$WORK/scr.txt" && ok "screen has: $1" || bad "screen has: $1"; }
expect_not() { d > "$WORK/scr.txt"; grep -qF -- "$1" "$WORK/scr.txt" && bad "screen lacks: $1" || ok "screen lacks: $1"; }
expect_cur() { cur > "$WORK/cur.txt"; grep -qF -- "$1" "$WORK/cur.txt" && ok "cursor on: $1" || bad "cursor on: $1 (got: $(tr '\n' '|' < "$WORK/cur.txt" | head -c 160))"; }
expect_copied() { sleep 1; grep -qF -- "$1" "$WORK/clipboard.txt" 2>/dev/null && ok "clipboard got: $1" || bad "clipboard got: $1"; expect "Copied $1"; }
expect_opened() { sleep 1.5; grep -qF -- "$1" "$WORK/opened.txt" 2>/dev/null && ok "browser got: $1" || bad "browser got: $1"; }
wait_for() { for _ in $(seq 1 40); do d | grep -qF -- "$1" && return 0; sleep 0.25; done; return 1; }
expect_soon() { wait_for "$1" && ok "shows: $1" || bad "shows: $1"; }
# Text order on screen, so it also holds when a multiplexer joins rows in its dump.
expect_order() {
  d > "$WORK/scr.txt"
  local last=-1 pos
  for s in "$@"; do
    pos=$(grep -boF -- "$s" "$WORK/scr.txt" | head -1 | cut -d: -f1)
    if [ -z "$pos" ] || [ "$pos" -le "$last" ]; then bad "order: $* (stuck at: $s)"; cat "$WORK/scr.txt"; return; fi
    last=$pos
  done
  ok "order: $*"
}

run_main() {
  rm -f "$WORK/opened.txt" "$WORK/clipboard.txt"
  step "PR list: this repo first, stacks, drafts hidden"
  expect_soon "updated"
  expect " alice  9 open PRs in 3 repos  2 drafts hidden (d shows)"
  expect "▾ acme/widgets  3  (this repo)"
  expect_order "▾ acme/widgets" "▾ acme/gadgets" "▾ acme/zeta-old"
  expect_order "Add widget cache" "└─ Cache eviction" "Docs for cache"
  expect "draft Gadget experiment base"; expect "└─ Gadget experiment child"
  expect_not "Eviction metrics"; expect_not "WIP rewrite"
  expect_cur "#10"

  step "d shows and hides drafts"
  c d; expect "drafts shown (d hides)"; expect "   └─ draft Eviction metrics"; expect "WIP rewrite"; expect_cur "#10"
  c d; expect "2 drafts hidden (d shows)"

  step "motions: j, counts, G, gg"
  c j; expect_cur "#11"
  c 3j; expect_cur "#3"
  c G; expect_cur "#1"
  c gg; expect_cur "▾ acme/widgets"

  step "repo jumps: } } {"
  c "}"; expect_cur "▾ acme/gadgets"
  c "}"; expect_cur "▾ acme/zeta-old"
  c "{"; expect_cur "▾ acme/gadgets"

  step "half pages: C-d C-u"
  k "$K_CD"; expect_cur "#1"
  k "$K_CU"; expect_cur "▾ acme/widgets"

  step "fold: h l space"
  c h; expect_cur "▸ acme/widgets"; expect_not "Add widget cache"
  c l; expect_cur "▾ acme/widgets"; expect "Add widget cache"
  k "$K_SPACE"; expect "▸ acme/widgets"
  k "$K_SPACE"; expect "▾ acme/widgets"
  c j; c h; expect_cur "▸ acme/widgets"; c l

  step "fuzzy search"
  c "/'cache" 1; expect "/ 'cache▏  3/7"; expect_not "Fix gadget leak"
  cur > "$WORK/first.txt"
  k "$K_DOWN"; cur > "$WORK/second.txt"; cmp -s "$WORK/first.txt" "$WORK/second.txt" && bad "down moves" || ok "down moves"
  k "$K_CP"; cur > "$WORK/third.txt"; cmp -s "$WORK/first.txt" "$WORK/third.txt" && ok "C-p moves back" || bad "C-p moves back"
  c x; expect "0/7"; k "$K_BS"; expect "3/7"
  k "$K_CW"; expect "7/7"
  c "^docs" 1; expect "1/7"; expect_cur "#14"
  k "$K_ESC"; expect "/ search"; expect "▾ acme/zeta-old"

  step "enter opens a PR on its Threads tab"
  # No waiting: the list load prefetched this PR's threads and files.
  c "/^add" 1; k "$K_ENTER"; expect "Unresolved threads"
  expect "acme/widgets #10 Add widget cache"; expect "Threads 3/4"; expect "Files 5"
  expect_cur "src/cache/mod.rs:2"; expect "Keep Mutex? RwLock changes the locking semantics."; expect "alice ·"
  c j; expect_cur "src/cache/mod.rs:3"
  c j; expect_cur "src/cache/store.rs:1"
  expect "it's worth checking the \"flush\" path & the <Drop> impl."; expect_not "&#39;"

  step "t shows resolved threads, o opens a thread"
  c t; expect "All threads"; c G; expect_cur "README.md"; expect "✓ Resolved"
  c o; expect_opened "https://github.com/acme/widgets/pull/10#discussion_r30"
  c t; expect "Unresolved threads"; expect_cur "src/cache/store.rs:1"

  step "enter jumps to the code, comments visible"
  c gg; k "$K_ENTER"
  expect "src/cache/mod.rs +4 −2"; expect_cur "┃ ● Unresolved thread · 2 comment(s)"; expect "carol ·"; expect "RwLock is fine, reads dominate."
  expect "j/k line  C-d/C-u page"

  step "diff motions and hunks"
  c gg; expect_cur "@@ -1,5 +1,6 @@"
  c 2j; expect_cur "-use std::sync::Mutex;"
  c "}"; expect_cur "@@ -40,3 +41,4 @@ impl Cache {"
  c "{"; expect_cur "@@ -1,5 +1,6 @@"
  c G; expect_cur "   42    44"

  step "n / N walk unresolved threads across files"
  c gg; c n; expect_cur "● Unresolved thread · 2 comment(s)"
  c n; expect_cur "● Unresolved thread · 1 comment(s)"; expect "src/cache/mod.rs +4 −2"
  c n; expect "src/cache/store.rs +4 −0"; expect_cur "● Unresolved thread"
  c n; expect "src/cache/mod.rs +4 −2"; expect_cur "· 2 comment(s)"
  c N; expect "src/cache/store.rs +4 −0"

  step "] [ walk files in tree order"
  c "]"; expect "src/lib.rs +1 −0"
  c "]"; expect "README.md +1 −1"; expect "thread(s) not on a line of this diff"; expect "┃ ✓ Resolved · carol: typo in the intro"
  c t; expect "┃ ✓ Resolved thread · 1 comment(s)"; c t; expect "┃ ✓ Resolved · carol"
  c "]"; expect "README.md +1 −1"
  c "["; expect "src/lib.rs +1 −0"
  c "[[["; expect "assets/logo.png +0 −0"; expect "No text diff"

  step "o on Files opens the files page, y copies the PR link"
  c o; expect_opened "https://github.com/acme/widgets/pull/10/files"
  c y; expect_copied "https://github.com/acme/widgets/pull/10"

  step "tree: focus, preview, parent, fold"
  c h; expect "enter view diff"; expect_cur "logo.png"
  c j; expect_cur "▾ src"
  c j; expect_cur "▾ cache"
  c j; expect_cur "mod.rs"; expect "src/cache/mod.rs +4 −2"
  c h; expect_cur "▾ cache"
  c h; expect_cur "▸ cache"
  c l; expect_cur "▾ cache"
  k "$K_ENTER"; expect_cur "▸ cache"
  k "$K_ENTER"; expect_cur "▾ cache"
  c j; k "$K_ENTER"; expect "j/k line  C-d/C-u page"
  c h; expect "enter view diff"

  step "fuzzy find a file"
  c "/store" 1; expect "/ store"; expect_cur "store.rs"
  k "$K_ENTER"; expect "src/cache/store.rs +4 −0"; expect "j/k line  C-d/C-u page"
  c "/"; k "$K_ESC"; expect "enter view diff"

  step "tab, reload, help, back"
  k "$K_TAB"; expect "Reviews and checks"; k "$K_TAB"; expect "Unresolved threads"
  c r 0.1; expect "refreshing…"; wait_for "Unresolved threads"; sleep 1.5; expect_not "refreshing…"
  c "?"; expect "Keys · any key closes"; c x; expect_not "Keys · any key closes"
  c q; expect "/ search"; expect "▾ acme/widgets  1"
  k "$K_ESC"; expect "▾ acme/gadgets"

  step "o on a PR, y copies, r refreshes"
  c gg; c j; c o; expect_opened "https://github.com/acme/widgets/pull/10"
  c j; c y; expect_copied "https://github.com/acme/widgets/pull/11"
  c r 0.1; expect "refreshing…"; expect_soon "updated now"; expect_not "refreshing…"

  step "a switches account"
  c a; expect "Switch account · enter"; c j; k "$K_ENTER"
  expect_soon " bob  1 open PRs in 1 repos"; expect_soon "▾ bob/dotfiles"
  c a; c k; k "$K_ENTER"; expect_soon " alice  9 open PRs"

  step "a PR without open threads starts on Files"
  c "/^docs" 1; k "$K_ENTER"; expect_soon "enter view diff"; expect_soon "CHANGELOG.md +1 −1"

  step "Status tab: reviewers and checks, blockers first"
  expect "Status ✗1 ●1 ✓2"
  k "$K_TAB"; expect "Reviews and checks"; expect "enter/o open in browser"
  expect_order "✗ carol" "changes requested" "re-requested" "✓ dave" "● docs-team" "review requested" "· erin" "commented"
  expect_order "Checks  ✗1 ●1 ✓2" "✗ Docs / linkcheck" "7m" "● Docs / deploy-preview" "running" "✓ CI / build" "4m" "✓ ci/legacy" "– Docs / spell" "… 2 more checks, only shown on GitHub"
  expect_cur "carol"
  c o; expect_opened "https://github.com/acme/widgets/pull/14#pullrequestreview-1"
  c 4j; expect_cur "Docs / linkcheck"; expect_cur "required"; k "$K_ENTER"; expect_opened "https://github.com/acme/widgets/actions/runs/1/job/2"
  c G; expect_cur "Docs / spell"
  k "$K_TAB"; expect "No review threads on this PR."
  c q; k "$K_ESC"

  step "e opens the editor in the PR's worktree"
  c gg; c j; expect_cur "#10"; c e 3; expect_not "open PRs in"
  c ":pwd" 0.3; k "$K_ENTER"; expect "widgets.worktrees\\cache"
  c ":qa!" 0.3; k "$K_ENTER"; expect_soon "Back from editor in"
  c j; expect_cur "#11"; c e 3
  c ":pwd" 0.3; k "$K_ENTER"; expect "\\root\\widgets"; expect_not "worktrees"
  c ":qa!" 0.3; k "$K_ENTER"; expect_soon "Back from editor in"
  c j; expect_cur "#14"; c e 1; expect "No local worktree of acme/widgets has docs/cache checked out"

  step "q quits"
  c q 1.5; expect_not "open PRs in"
}

run_scroll() {
  step "second start shows the cached list and details while refreshing"
  expect_soon "Add widget cache"; expect "refreshing…"
  k "$K_ENTER"; expect "Unresolved threads"
  expect_soon "updated now"

  step "thread pane scrolls in a short terminal"
  c jj; expect "enter: jump to this line"
  k "$K_CD"; expect_not "enter: jump to this line"
  k "$K_CU"; expect "enter: jump to this line"
  k "$K_CE"; k "$K_CE"; expect_not "enter: jump to this line"
  k "$K_CY"; k "$K_CY"; expect "enter: jump to this line"
  for _ in 1 2 3 4 5 6 7 8 9 10; do k "$K_CD"; done; expect "END-OF-LONG-COMMENT"
  c q; c q 1.5; expect_not "open PRs in"
}
