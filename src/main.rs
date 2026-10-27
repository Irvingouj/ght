mod app;
mod cache;
mod diff;
mod fuzzy;
mod github;
mod model;
#[cfg(test)]
mod testutil;
mod tree;
mod ui;
mod worktree;

use std::path::Path;
use std::collections::HashMap;
use std::io::Write;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyEventKind};

use app::{App, Effect, Msg};

const USAGE: &str = "ght: your open GitHub pull requests, review threads and diffs in the terminal.

Usage: ght

Uses the accounts you are logged in to with `gh auth login`.
Press ? inside the app for keys. Set BROWSER to choose the program that opens links,
and EDITOR for the editor `e` opens in the PR's local worktree (default: nvim).
`y` copies the PR link with clip, pbcopy, wl-copy or xclip.";

fn main() -> Result<()> {
    if std::env::args().skip(1).any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return Ok(());
    }
    let (tx, rx) = mpsc::channel();
    {
        let tx = tx.clone();
        thread::spawn(move || {
            let _ = tx.send(Msg::Accounts(github::accounts().map_err(|e| format!("{e:#}"))));
        });
    }

    let store = cache::Store::new();
    let tokens = Tokens::default();
    let mut app = App::new();
    app.here = github::local_repos();
    let roots = worktree::search_roots();
    // The last run's accounts let the cached list show before `gh auth status` answers.
    let mut startup: Vec<Effect> = match store.accounts() {
        Some(accounts) => app.on_msg(Msg::Accounts(Ok(accounts))).into_iter().collect(),
        None => Vec::new(),
    };
    let mut dirty = false;
    let mut saved = Instant::now();
    let mut terminal = ratatui::init();
    let result = (|| -> Result<()> {
        loop {
            terminal.draw(|f| ui::draw(f, &mut app))?;
            let mut effects = std::mem::take(&mut startup);
            if event::poll(Duration::from_millis(100))?
                && let Event::Key(key) = event::read()?
                && key.kind != KeyEventKind::Release
            {
                effects.extend(app.handle_key(key));
            }
            while let Ok(msg) = rx.try_recv() {
                match &msg {
                    Msg::Accounts(Ok(list)) => store.save_accounts(list),
                    Msg::Prs { result: Ok(_), .. } | Msg::Detail { result: Ok(_), .. } => dirty = true,
                    _ => {}
                }
                effects.extend(app.on_msg(msg));
            }
            if dirty && saved.elapsed() > Duration::from_secs(2) {
                save(&store, &app);
                (dirty, saved) = (false, Instant::now());
            }
            for effect in effects {
                match effect {
                    Effect::Quit => {
                        if dirty {
                            save(&store, &app);
                        }
                        return Ok(());
                    }
                    Effect::LoadPrs => {
                        if let Some(account) = &app.account
                            && app.prs.is_empty()
                            && let Some(snapshot) = store.snapshot(account)
                        {
                            app.restore(snapshot);
                        }
                        load_prs(&app, &tx, &tokens);
                    }
                    Effect::LoadDetail { repo, number } => load_details(&app, &tx, &tokens, vec![(repo, number)]),
                    Effect::Prefetch(keys) => load_details(&app, &tx, &tokens, keys),
                    Effect::Edit { repo, branch } => match worktree::find(&roots, &repo, &branch) {
                        None => {
                            let looked: Vec<String> = roots.iter().map(|r| r.display().to_string()).collect();
                            app.flash = Some(format!("No local worktree of {repo} has {branch} checked out (looked in {})", looked.join(", ")));
                        }
                        Some(dir) => {
                            ratatui::restore();
                            let status = edit(&dir);
                            terminal = ratatui::init();
                            app.flash = Some(match status {
                                Ok(_) => format!("Back from editor in {}", dir.display()),
                                Err(e) => format!("Could not start editor in {}: {e}", dir.display()),
                            });
                        }
                    },
                    Effect::Open(url) => {
                        app.flash = Some(match open(&url) {
                            Ok(()) => format!("Opened {url}"),
                            Err(e) => format!("Could not open browser: {e}"),
                        });
                    }
                    Effect::Copy(text) => {
                        app.flash = Some(match copy(&text) {
                            Ok(()) => format!("Copied {text}"),
                            Err(e) => format!("Could not copy to clipboard: {e}"),
                        });
                    }
                }
            }
        }
    })();
    ratatui::restore();
    result
}

fn save(store: &cache::Store, app: &App) {
    if let Some(account) = &app.account {
        store.save_snapshot(account, &app.snapshot());
    }
}

/// Tokens per account; asking `gh` costs a process start every time.
#[derive(Clone, Default)]
struct Tokens(Arc<Mutex<HashMap<String, String>>>);

impl Tokens {
    fn get(&self, account: &str) -> Result<String> {
        if let Some(t) = self.0.lock().expect("token lock").get(account) {
            return Ok(t.clone());
        }
        let t = github::token(account)?;
        self.0.lock().expect("token lock").insert(account.to_string(), t.clone());
        Ok(t)
    }
}

fn load_prs(app: &App, tx: &Sender<Msg>, tokens: &Tokens) {
    let Some(account) = app.account.clone() else { return };
    let (tx, tokens) = (tx.clone(), tokens.clone());
    thread::spawn(move || {
        let result = tokens.get(&account).and_then(|t| github::fetch_prs(&t)).map_err(|e| format!("{e:#}"));
        let _ = tx.send(Msg::Prs { account, result });
    });
}

/// Fetches details four PRs at a time, sending each one as soon as it lands.
fn load_details(app: &App, tx: &Sender<Msg>, tokens: &Tokens, keys: Vec<(String, u64)>) {
    let Some(account) = app.account.clone() else { return };
    let (tx, tokens) = (tx.clone(), tokens.clone());
    thread::spawn(move || {
        for chunk in keys.chunks(4) {
            thread::scope(|s| {
                for (repo, number) in chunk {
                    let (tx, tokens, account) = (&tx, &tokens, &account);
                    s.spawn(move || {
                        let result = tokens.get(account).and_then(|t| github::fetch_detail(&t, repo, *number)).map_err(|e| format!("{e:#}"));
                        let _ = tx.send(Msg::Detail { repo: repo.clone(), number: *number, result });
                    });
                }
            });
        }
    });
}

fn edit(dir: &Path) -> std::io::Result<ExitStatus> {
    let editor = std::env::var("EDITOR").ok().filter(|e| !e.trim().is_empty()).unwrap_or_else(|| "nvim".into());
    let mut words = editor.split_whitespace();
    let program = words.next().unwrap_or("nvim");
    Command::new(program).args(words).arg(".").current_dir(dir).status()
}

fn open(url: &str) -> std::io::Result<()> {
    let mut cmd = match std::env::var("BROWSER") {
        Ok(browser) if !browser.is_empty() => Command::new(browser),
        _ if cfg!(windows) => {
            let mut c = Command::new("rundll32");
            c.arg("url.dll,FileProtocolHandler");
            c
        }
        _ if cfg!(target_os = "macos") => Command::new("open"),
        _ => Command::new("xdg-open"),
    };
    cmd.arg(url).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map(|_| ())
}

/// `GHT_CLIPBOARD` exists so tests can catch the copy without touching the real clipboard.
fn copy(text: &str) -> std::io::Result<()> {
    let mut cmd = match std::env::var("GHT_CLIPBOARD") {
        Ok(program) if !program.is_empty() => Command::new(program),
        _ if cfg!(windows) => Command::new("clip"),
        _ if cfg!(target_os = "macos") => Command::new("pbcopy"),
        _ if std::env::var_os("WAYLAND_DISPLAY").is_some() => Command::new("wl-copy"),
        _ => {
            let mut c = Command::new("xclip");
            c.args(["-selection", "clipboard"]);
            c
        }
    };
    let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    child.stdin.take().expect("piped stdin").write_all(text.as_bytes())?;
    match child.wait()? {
        status if status.success() => Ok(()),
        status => Err(std::io::Error::other(format!("clipboard program exited with {status}"))),
    }
}
