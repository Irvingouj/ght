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

The bottom line always lists the keys that work on the current screen. Press `?` for all of them.

## Keys

| Where | Keys |
|---|---|
| Everywhere | `j` `k` move (counts work: `5j`), `gg` `G` top/bottom, `C-d` `C-u` half page, `o` open in browser, `r` reload, `q` back/quit |
| PR list | `enter` open, `/` fuzzy search, `{` `}` previous/next repo, `h` `l` `space` fold repo, `d` show/hide drafts, `a` switch account |
| Threads | `enter` go to code, `C-d` `C-u` `C-e` `C-y` scroll, `t` show resolved, `tab` Files |
| Files tree | `enter`/`l` open, `h` fold or go to parent, `/` find file, `]` `[` next/previous file, `n` `N` next/previous unresolved thread |
| Diff | `{` `}` hunks, `]` `[` files, `n` `N` threads, `t` expand resolved threads, `h` back to tree |

Search uses fzf syntax: `'exact`, `^prefix`, `suffix$`, `!not`.

Set `BROWSER` to choose the program that opens links.

## License

MIT
