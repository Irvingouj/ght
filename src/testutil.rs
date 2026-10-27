use chrono::{Duration, Utc};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{App, Effect, Msg};
use crate::diff::parse_patch;
use crate::github::Account;
use crate::model::{Check, CheckState, Comment, FileDiff, FileStatus, Pr, PrDetail, ReviewState, Reviewer, Side, Thread};

pub fn pr(repo: &str, number: u64, title: &str, hours_ago: i64, unresolved: usize, total: usize) -> Pr {
    Pr {
        repo: repo.into(),
        number,
        title: title.into(),
        url: format!("https://github.com/{repo}/pull/{number}"),
        is_draft: false,
        head: format!("b{number}"),
        base: "master".into(),
        updated_at: Utc::now() - Duration::hours(hours_ago),
        additions: 10,
        deletions: 2,
        review: None,
        checks: None,
        threads_total: total,
        threads_unresolved: unresolved,
    }
}

/// Repos by latest update: alpha (2h), beta (3h), Gamma (24h).
/// Rows: 0 ▾o/alpha, 1 #12, 2 #3, 3 ▾o/beta, 4 #5, 5 ▾O/Gamma, 6 #7.
pub fn prs() -> Vec<Pr> {
    vec![
        pr("o/beta", 5, "Fix login bug", 3, 0, 0),
        pr("o/alpha", 12, "Add agent picker", 2, 3, 4),
        pr("o/alpha", 3, "Refactor tree", 5, 0, 1),
        pr("O/Gamma", 7, "Bump deps", 24, 0, 0),
    ]
}

fn thread(path: &str, line: Option<u32>, side: Side, resolved: bool, outdated: bool, body: &str) -> Thread {
    Thread {
        path: path.into(),
        line,
        side,
        is_resolved: resolved,
        is_outdated: outdated,
        diff_hunk: "@@ -1 +1 @@\n-old\n+new".into(),
        comments: vec![Comment {
            author: "reviewer".into(),
            body: body.into(),
            created_at: Utc::now() - Duration::hours(3),
            url: format!("https://example.test/{path}#{}", line.unwrap_or(0)),
        }],
    }
}

fn file(path: &str, status: FileStatus, patch: Option<&str>) -> FileDiff {
    FileDiff {
        path: path.into(),
        previous_path: None,
        status,
        additions: 2,
        deletions: 1,
        has_patch: patch.is_some(),
        lines: patch.map(parse_patch).unwrap_or_default(),
    }
}

/// Files 0 src/main.rs, 1 README.md, 2 src/app/mod.rs, 3 assets/logo.png.
/// Tree order: assets/, logo.png(3), src/, app/, mod.rs(2), main.rs(0), README.md(1).
/// Threads: 0 main.rs:12 right open, 1 mod.rs:2 resolved, 2 README outdated open, 3 main.rs:11 left open.
pub fn detail() -> PrDetail {
    PrDetail {
        files: vec![
            file(
                "src/main.rs",
                FileStatus::Modified,
                Some("@@ -10,3 +10,4 @@ fn main() {\n let a = 1;\n-let b = 2;\n+let b = 3;\n+let c = 4;\n run();\n@@ -40,2 +41,3 @@\n x\n+y"),
            ),
            file("README.md", FileStatus::Added, Some("@@ -0,0 +1 @@\n+hello")),
            file("src/app/mod.rs", FileStatus::Modified, Some("@@ -1,2 +1,2 @@\n a\n-b\n+B")),
            file("assets/logo.png", FileStatus::Added, None),
        ],
        threads: vec![
            thread("src/main.rs", Some(12), Side::Right, false, false, "why add c?"),
            thread("src/app/mod.rs", Some(2), Side::Right, true, false, "nit: casing"),
            thread("README.md", None, Side::Right, false, true, "typo"),
            thread("src/main.rs", Some(11), Side::Left, false, false, "keep b = 2"),
        ],
        reviewers: vec![
            reviewer("ann", Some(ReviewState::Approved), false),
            reviewer("bob", Some(ReviewState::ChangesRequested), false),
            reviewer("cid", None, true),
        ],
        checks: vec![
            check("lint", CheckState::Passing),
            check("test", CheckState::Failing),
            check("deploy", CheckState::Pending),
        ],
        checks_total: 3,
    }
}

fn reviewer(login: &str, state: Option<ReviewState>, requested: bool) -> Reviewer {
    let url = state.map(|_| format!("https://github.com/o/alpha/pull/12#review-{login}"));
    Reviewer { login: login.into(), state, requested, at: state.map(|_| Utc::now() - Duration::hours(2)), url }
}

fn check(name: &str, state: CheckState) -> Check {
    Check {
        name: name.into(),
        workflow: Some("CI".into()),
        state,
        required: name == "test",
        url: Some(format!("https://github.com/o/alpha/actions/{name}")),
        started: Some(Utc::now() - Duration::minutes(10)),
        completed: (state != CheckState::Pending).then(|| Utc::now() - Duration::minutes(3)),
    }
}

pub fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

pub fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

pub fn typ(app: &mut App, text: &str) -> Vec<Effect> {
    text.chars().filter_map(|c| app.handle_key(key(KeyCode::Char(c)))).collect()
}

pub fn loaded_app() -> App {
    let mut app = App::new();
    let accounts = vec![Account { login: "work".into(), active: true }, Account { login: "home".into(), active: false }];
    assert_eq!(app.on_msg(Msg::Accounts(Ok(accounts))), Some(Effect::LoadPrs));
    app.on_msg(Msg::Prs { account: "work".into(), result: Ok(prs()) });
    app
}

/// Opens PR o/alpha#12 (row 1) and delivers its detail.
pub fn detail_app() -> App {
    let mut app = loaded_app();
    assert_eq!(app.handle_key(key(KeyCode::Enter)), Some(Effect::LoadDetail { repo: "o/alpha".into(), number: 12 }));
    app.on_msg(Msg::Detail { repo: "o/alpha".into(), number: 12, result: Ok(detail()) });
    app
}
