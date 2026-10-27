use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::KeyCode;

use crate::app::{App, Msg};
use crate::testutil::*;

fn screen(app: &mut App, w: u16, h: u16) -> String {
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| super::draw(f, app)).unwrap();
    let buf = term.backend().buffer();
    (0..h)
        .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect::<String>().trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn dashboard_shows_groups_columns_and_hints() {
    let mut app = loaded_app();
    let s = screen(&mut app, 110, 14);
    assert!(s.contains("work"), "{s}");
    assert!(s.contains("4 open PRs in 3 repos"), "{s}");
    for needle in ["▾ o/alpha", "▾ o/beta", "▾ O/Gamma", "#12", "Add agent picker", "● 3/4", "✓ 1", "Threads", "/ search"] {
        assert!(s.contains(needle), "missing {needle:?} in\n{s}");
    }
}

#[test]
fn dashboard_search_prompt_and_counts() {
    let mut app = loaded_app();
    typ(&mut app, "/agpk");
    let s = screen(&mut app, 110, 14);
    assert!(s.contains("/ agpk▏  1/4"), "{s}");
    assert!(!s.contains("Fix login bug"), "{s}");
    typ(&mut app, "zzz");
    assert!(screen(&mut app, 110, 14).contains("No pull request matches"));
}

#[test]
fn dashboard_loading_and_error_states() {
    let mut app = App::new();
    assert!(screen(&mut app, 80, 10).contains("Loading your open pull requests"));
    app.on_msg(Msg::Accounts(Err("gh not found".into())));
    let s = screen(&mut app, 80, 10);
    assert!(s.contains("gh not found") && s.contains("press r to retry"), "{s}");
}

#[test]
fn threads_tab_lists_threads_and_shows_conversation() {
    let mut app = detail_app();
    let s = screen(&mut app, 120, 24);
    for needle in ["o/alpha #12", "Threads 3/4", "Files 4", "Unresolved threads", "● README.md", "src/main.rs:11", "reviewer", "typo", "outdated"] {
        assert!(s.contains(needle), "missing {needle:?} in\n{s}");
    }
    assert!(!s.contains("nit: casing"), "resolved thread hidden\n{s}");
}

#[test]
fn status_tab_shows_reviewers_and_checks() {
    let mut app = detail_app();
    assert!(screen(&mut app, 120, 24).contains("Status ✗1 ●1 ✓1"));
    app.handle_key(key(KeyCode::BackTab));
    let s = screen(&mut app, 120, 24);
    for needle in ["Reviews and checks", "✗ bob", "changes requested", "✓ ann", "approved", "● cid", "review requested", "Checks  ✗1 ●1 ✓1", "✗ CI / test", "required", "7m", "running 10m", "✓ CI / lint", "enter/o open in browser"] {
        assert!(s.contains(needle), "missing {needle:?} in\n{s}");
    }

    let mut app = detail_app();
    let d = app.detail.as_mut().unwrap();
    d.tab = crate::app::Tab::Status;
    let mut empty = (*d.ready().unwrap()).clone();
    (empty.reviewers, empty.checks, empty.checks_total) = (Vec::new(), Vec::new(), 0);
    app.on_msg(Msg::Detail { repo: "o/alpha".into(), number: 12, result: Ok(empty) });
    let s = screen(&mut app, 120, 24);
    assert!(s.contains("Nobody has reviewed or been asked to review yet."), "{s}");
    assert!(s.contains("No CI checks on the last commit."), "{s}");
}

#[test]
fn files_tab_shows_tree_and_diff_with_inline_threads() {
    let mut app = detail_app();
    typ(&mut app, "j");
    app.handle_key(key(KeyCode::Enter));
    let s = screen(&mut app, 120, 30);
    for needle in ["▾ assets", "▾ src", "▾ app", "M main.rs ●2", "A README.md ●1", "src/main.rs +2 −1", "-let b = 2;", "+let c = 4;", "┃ ● Unresolved thread", "keep b = 2", "@@ -40,2 +41,3 @@"] {
        assert!(s.contains(needle), "missing {needle:?} in\n{s}");
    }
    assert!(s.contains("   11       -let b = 2;"), "old line number gutter\n{s}");
}

#[test]
fn binary_file_and_help_popup() {
    let mut app = detail_app();
    app.handle_key(key(KeyCode::Tab));
    let s = screen(&mut app, 120, 20);
    assert!(s.contains("No text diff"), "{s}");
    typ(&mut app, "?");
    let s = screen(&mut app, 120, 40);
    assert!(s.contains("Keys · any key closes") && s.contains("next / previous unresolved thread"), "{s}");
}

#[test]
fn narrow_terminal_does_not_panic() {
    let mut app = detail_app();
    for (w, h) in [(20, 5), (40, 8), (1, 1)] {
        screen(&mut app, w, h);
        app.handle_key(key(KeyCode::Tab));
        screen(&mut app, w, h);
    }
    let mut app = loaded_app();
    screen(&mut app, 10, 3);
}

#[test]
fn long_paths_keep_their_file_name() {
    let (shown, hl) = super::tail_fit("ffi/dotnet/Generated/ActiveStage.cs", &[0, 21, 22], 16);
    assert_eq!(shown, "…/ActiveStage.cs");
    assert_eq!(hl, vec![2, 3]);
    assert_eq!(super::tail_fit("a.rs", &[0], 16), ("a.rs".to_string(), vec![0]));
}

#[test]
fn resolved_thread_head_collapsed_then_expanded() {
    let mut app = detail_app();
    app.handle_key(key(KeyCode::Tab));
    typ(&mut app, "]");
    let s = screen(&mut app, 120, 20);
    assert!(s.contains("┃ ✓ Resolved · reviewer: nit: casing"), "{s}");
    typ(&mut app, "t");
    let s = screen(&mut app, 120, 20);
    assert!(s.contains("┃ ✓ Resolved thread · 1 comment(s)") && s.contains("nit: casing"), "{s}");
}

#[test]
fn stack_tree_and_hidden_draft_count_render() {
    let mut app = loaded_app();
    let mut fresh = prs();
    fresh[2].base = "b12".into();
    fresh[3].is_draft = true;
    app.on_msg(Msg::Prs { account: "work".into(), result: Ok(fresh) });
    let s = screen(&mut app, 110, 14);
    assert!(s.contains("1 drafts hidden (d shows)"), "{s}");
    assert!(s.contains("└─ Refactor tree"), "{s}");
    assert!(!s.contains("Bump deps"), "{s}");
}

#[test]
fn current_repo_header_is_marked() {
    let mut app = loaded_app();
    app.here = ["o/beta".to_string()].into();
    let s = screen(&mut app, 110, 14);
    assert!(s.contains("▾ o/beta  1  (this repo)"), "{s}");
}

#[test]
fn top_bar_shows_cached_age_while_refreshing() {
    let mut app = App::new();
    app.on_msg(Msg::Accounts(Ok(vec![crate::github::Account { login: "work".into(), active: true }])));
    app.restore(crate::cache::Snapshot { saved_at: chrono::Utc::now() - chrono::Duration::hours(3), prs: prs(), details: vec![] });
    let s = screen(&mut app, 110, 8);
    assert!(s.contains("updated 3h  refreshing…"), "{s}");
}

#[test]
fn search_total_matches_the_list_when_a_draft_is_kept() {
    let mut app = loaded_app();
    let mut fresh = prs();
    fresh[2].is_draft = true;
    fresh[2].head = "b3".into();
    fresh[3].repo = "o/alpha".into();
    fresh[3].base = "b3".into();
    app.on_msg(Msg::Prs { account: "work".into(), result: Ok(fresh) });
    typ(&mut app, "/");
    let s = screen(&mut app, 110, 12);
    assert!(s.contains("4/4"), "{s}");
}
