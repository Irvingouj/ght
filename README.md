# ght

Your open GitHub pull requests, their review threads, and their diffs in one terminal window.

```
cargo install --git https://github.com/Irvingouj/ght
ght
```

Or download `ght.exe` from the [latest release](https://github.com/Irvingouj/ght/releases/latest).

`ght` uses the accounts you logged in to with `gh auth login`. It never writes to GitHub.

## Screens

- **PR list**: every open PR you authored, grouped by repo. Columns show CI, review state, and unresolved/total threads. Drafts are hidden until you press `d`. A PR whose base branch is another open PR's branch is drawn under it as a stack.
- **Threads tab**: unresolved review threads with their code and comments. Press `enter` to jump to the line.
- **Files tab**: a folder tree of changed files next to the diff. Review threads sit under the lines they comment on.
- **Status tab**: each reviewer's latest verdict (and who still has to review), then every CI check of the last commit. Failures and required checks come first.

The bottom line always lists the keys that work on the current screen. Press `?` for all of them.

## Keys

| Where | Keys |
|---|---|
| Everywhere | `j` `k` move (counts work: `5j`), `gg` `G` top/bottom, `C-d` `C-u` half page, `o` open in browser, `y` copy the PR link, `e` open editor in the PR's local worktree, `r` reload, `q` back/quit |
| PR list | `enter` open, `/` fuzzy search, `{` `}` previous/next repo, `h` `l` `space` fold repo, `d` show/hide drafts, `a` switch account |
| Threads | `enter` go to code, `C-d` `C-u` `C-e` `C-y` scroll, `t` show resolved, `tab` next tab (Threads → Files → Status) |
| Status | `enter`/`o` open the review or check on GitHub |
| Files tree | `enter`/`l` open, `h` fold or go to parent, `/` find file, `]` `[` next/previous file, `n` `N` next/previous unresolved thread |
| Diff | `{` `}` hunks, `]` `[` files, `n` `N` threads, `t` expand resolved threads, `h` back to tree |

Search uses fzf syntax: `'exact`, `^prefix`, `suffix$`, `!not`.

Set `BROWSER` to choose the program that opens links. `y` copies with `clip` (Windows), `pbcopy` (macOS), `wl-copy` (Wayland) or `xclip`.

`e` looks for a local worktree that has the PR's branch checked out (same name, or tracking it). It searches clones in the folder you start `ght` from, its subfolders, and the folder next to the current clone. It opens `$EDITOR` there, or `nvim` when `EDITOR` is not set.

## Speed

`ght` keeps the last PR list and PR details in a cache (`%LOCALAPPDATA%\ght`, or `~/.cache/ght`). On start it shows the cached list at once and refreshes in the background; the top bar says `refreshing…` until fresh data lands. After each refresh it prefetches the details of PRs that changed, so opening a PR is usually instant.

## Tests

```
cargo test                 # unit and render tests
bash tests/e2e/run.sh      # drives the real binary in headless tmux (WSL) and zellij
```

The end-to-end run talks to a fake GitHub on 127.0.0.1 that serves `tests/e2e/fixture.json`, so it never depends on live data.

## License

MIT
