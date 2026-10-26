mod app;
mod diff;
mod fuzzy;
mod github;
mod model;
#[cfg(test)]
mod testutil;
mod tree;
mod ui;

use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Duration;

use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyEventKind};

use app::{App, Effect, Msg};

const USAGE: &str = "ght: your open GitHub pull requests, review threads and diffs in the terminal.

Usage: ght

Uses the accounts you are logged in to with `gh auth login`.
Press ? inside the app for keys. Set BROWSER to choose the program that opens links.";

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

    let mut app = App::new();
    let mut terminal = ratatui::init();
    let result = (|| -> Result<()> {
        loop {
            terminal.draw(|f| ui::draw(f, &mut app))?;
            let mut effects = Vec::new();
            if event::poll(Duration::from_millis(100))?
                && let Event::Key(key) = event::read()?
                && key.kind != KeyEventKind::Release
            {
                effects.extend(app.handle_key(key));
            }
            while let Ok(msg) = rx.try_recv() {
                effects.extend(app.on_msg(msg));
            }
            for effect in effects {
                match effect {
                    Effect::Quit => return Ok(()),
                    Effect::LoadPrs => load_prs(&app, &tx),
                    Effect::LoadDetail { repo, number } => load_detail(&app, &tx, repo, number),
                    Effect::Open(url) => {
                        app.flash = Some(match open(&url) {
                            Ok(()) => format!("Opened {url}"),
                            Err(e) => format!("Could not open browser: {e}"),
                        });
                    }
                }
            }
        }
    })();
    ratatui::restore();
    result
}

fn load_prs(app: &App, tx: &Sender<Msg>) {
    let Some(account) = app.account.clone() else { return };
    let tx = tx.clone();
    thread::spawn(move || {
        let result = github::token(&account).and_then(|t| github::fetch_prs(&t)).map_err(|e| format!("{e:#}"));
        let _ = tx.send(Msg::Prs { account, result });
    });
}

fn load_detail(app: &App, tx: &Sender<Msg>, repo: String, number: u64) {
    let Some(account) = app.account.clone() else { return };
    let tx = tx.clone();
    thread::spawn(move || {
        let result = github::token(&account).and_then(|t| github::fetch_detail(&t, &repo, number)).map_err(|e| format!("{e:#}"));
        let _ = tx.send(Msg::Detail { repo, number, result });
    });
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
