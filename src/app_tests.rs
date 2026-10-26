use ratatui::crossterm::event::KeyCode;

use super::*;
use crate::testutil::*;

fn row_labels(app: &App) -> Vec<String> {
    app.dash_rows()
        .iter()
        .map(|r| match r {
            DashRow::Group { repo, count, collapsed } => format!("{}{repo} {count}", if *collapsed { "+" } else { "-" }),
            DashRow::Pr { idx, .. } => format!("#{}", app.prs[*idx].number),
        })
        .collect()
}

fn detail(app: &App) -> &Detail {
    app.detail.as_ref().expect("detail open")
}

fn data(app: &App) -> Rc<PrDetail> {
    detail(app).ready().expect("data ready")
}

fn cursor_row(app: &App) -> DiffRow {
    let d = detail(app);
    d.diff_rows(&data(app))[d.diff_cursor].clone()
}

#[test]
fn groups_by_repo_case_insensitive_newest_first() {
    let app = loaded_app();
    assert_eq!(row_labels(&app), vec!["-o/alpha 2", "#12", "#3", "-o/beta 1", "#5", "-O/Gamma 1", "#7"]);
    assert_eq!(app.dash.cursor, 1, "starts on the first PR, not a header");
}

#[test]
fn vim_motions_with_counts() {
    let mut app = loaded_app();
    typ(&mut app, "j");
    assert_eq!(app.dash.cursor, 2);
    typ(&mut app, "3j");
    assert_eq!(app.dash.cursor, 5);
    typ(&mut app, "gg");
    assert_eq!(app.dash.cursor, 0);
    typ(&mut app, "G");
    assert_eq!(app.dash.cursor, 6);
    typ(&mut app, "10k");
    assert_eq!(app.dash.cursor, 0);
    app.handle_key(ctrl('d'));
    assert_eq!(app.dash.cursor, 6.min(app.dash.page / 2));
}

#[test]
fn braces_jump_between_repos() {
    let mut app = loaded_app();
    typ(&mut app, "}");
    assert_eq!(app.dash.cursor, 3);
    typ(&mut app, "}");
    assert_eq!(app.dash.cursor, 5);
    typ(&mut app, "}");
    assert_eq!(app.dash.cursor, 5);
    typ(&mut app, "{");
    assert_eq!(app.dash.cursor, 3);
    typ(&mut app, "{{");
    assert_eq!(app.dash.cursor, 0);
}

#[test]
fn folding_repos() {
    let mut app = loaded_app();
    typ(&mut app, "h");
    assert_eq!(app.dash.cursor, 0, "h on a PR folds its repo and lands on the header");
    assert_eq!(row_labels(&app)[..2], ["+o/alpha 2".to_string(), "-o/beta 1".to_string()]);
    typ(&mut app, "l");
    assert_eq!(row_labels(&app)[1], "#12");
    app.handle_key(key(KeyCode::Char(' ')));
    assert_eq!(row_labels(&app)[0], "+o/alpha 2");
    assert_eq!(app.handle_key(key(KeyCode::Enter)), None, "enter on a header toggles, never opens");
    assert_eq!(row_labels(&app)[0], "-o/alpha 2");
}

#[test]
fn fuzzy_search_filters_highlights_and_enter_opens() {
    let mut app = loaded_app();
    typ(&mut app, "/agpk");
    assert!(app.dash.searching);
    assert_eq!(row_labels(&app), vec!["-o/alpha 1", "#12"]);
    let DashRow::Pr { hl, .. } = &app.dash_rows()[1] else { panic!() };
    assert!(!hl.is_empty() && hl.iter().all(|&i| i < "Add agent picker".len() as u32));
    assert_eq!(app.dash.cursor, 1);

    app.handle_key(key(KeyCode::Backspace));
    assert_eq!(app.dash.query, "agp");
    let effect = app.handle_key(key(KeyCode::Enter));
    assert_eq!(effect, Some(Effect::LoadDetail { repo: "o/alpha".into(), number: 12 }));
    assert!(!app.dash.searching);
    assert_eq!(app.dash.query, "agp", "filter stays after opening");

    typ(&mut app, "q");
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.dash.query, "");
    assert_eq!(row_labels(&app).len(), 7);
}

#[test]
fn search_typing_q_or_j_does_not_trigger_commands() {
    let mut app = loaded_app();
    let effects = typ(&mut app, "/qj");
    assert!(effects.is_empty());
    assert_eq!(app.dash.query, "qj");
    app.handle_key(ctrl('w'));
    assert_eq!(app.dash.query, "");
}

#[test]
fn search_arrows_skip_headers_and_ranks_best_repo_first() {
    let mut app = loaded_app();
    typ(&mut app, "/login");
    assert_eq!(row_labels(&app), vec!["-o/beta 1", "#5"]);
    app.handle_key(ctrl('u'));
    typ(&mut app, "e");
    let labels = row_labels(&app);
    assert!(labels.len() > 2);
    let first = app.dash.cursor;
    app.handle_key(key(KeyCode::Down));
    assert!(matches!(app.dash_rows()[app.dash.cursor], DashRow::Pr { .. }));
    assert!(app.dash.cursor > first);
    app.handle_key(key(KeyCode::Esc));
    assert!(!app.dash.searching);
}

#[test]
fn open_in_browser_and_quit() {
    let mut app = loaded_app();
    assert_eq!(typ(&mut app, "o"), vec![Effect::Open("https://github.com/o/alpha/pull/12".into())]);
    assert_eq!(typ(&mut app, "q"), vec![Effect::Quit]);
    assert_eq!(app.handle_key(ctrl('c')), Some(Effect::Quit));
}

#[test]
fn account_switch_reloads_and_ignores_stale_results() {
    let mut app = loaded_app();
    typ(&mut app, "a");
    assert!(matches!(app.popup, Some(Popup::Accounts { cursor: 0 })));
    typ(&mut app, "j");
    assert_eq!(app.handle_key(key(KeyCode::Enter)), Some(Effect::LoadPrs));
    assert_eq!(app.account.as_deref(), Some("home"));
    assert!(app.prs.is_empty());
    assert_eq!(app.status, Status::Loading);

    app.on_msg(Msg::Prs { account: "work".into(), result: Ok(prs()) });
    assert!(app.prs.is_empty(), "late result for the old account is dropped");
    app.on_msg(Msg::Prs { account: "home".into(), result: Ok(prs()[..1].to_vec()) });
    assert_eq!(app.prs.len(), 1);
}

#[test]
fn refresh_keeps_cursor_on_same_pr() {
    let mut app = loaded_app();
    typ(&mut app, "G");
    assert_eq!(typ(&mut app, "r"), vec![Effect::LoadPrs]);
    let mut fresh = prs();
    fresh.remove(0);
    app.on_msg(Msg::Prs { account: "work".into(), result: Ok(fresh) });
    assert_eq!(app.selected_pr().map(|p| p.number), Some(7));
}

#[test]
fn detail_starts_on_threads_when_unresolved_and_hides_resolved() {
    let app = detail_app();
    let d = detail(&app);
    assert_eq!(d.tab, Tab::Threads);
    assert_eq!(visible_threads(&data(&app), false), vec![2, 3, 0], "README first, then main.rs by line; resolved hidden");
    assert_eq!(visible_threads(&data(&app), true), vec![2, 3, 0, 1]);
}

#[test]
fn detail_without_open_threads_starts_on_files() {
    let mut app = loaded_app();
    typ(&mut app, "j");
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(detail(&app).tab, Tab::Files);
    assert!(matches!(detail(&app).data, Load::Loading));
    typ(&mut app, "q");
    assert!(app.detail.is_none());
}

#[test]
fn reopening_uses_cache_and_refreshes() {
    let mut app = detail_app();
    typ(&mut app, "q");
    assert_eq!(app.handle_key(key(KeyCode::Enter)), Some(Effect::LoadDetail { repo: "o/alpha".into(), number: 12 }));
    assert!(detail(&app).ready().is_some(), "cached data shows at once");
    assert!(detail(&app).refreshing);
}

#[test]
fn toggle_resolved_threads() {
    let mut app = detail_app();
    typ(&mut app, "G");
    assert_eq!(detail(&app).th_cursor, 2);
    typ(&mut app, "t");
    assert!(detail(&app).show_resolved);
    typ(&mut app, "G");
    assert_eq!(detail(&app).th_cursor, 3);
    typ(&mut app, "t");
    assert_eq!(detail(&app).th_cursor, 2, "cursor clamps when rows disappear");
}

#[test]
fn thread_enter_jumps_to_its_line_in_diff() {
    let mut app = detail_app();
    typ(&mut app, "j");
    app.handle_key(key(KeyCode::Enter));
    let d = detail(&app);
    assert_eq!((d.tab, d.focus, d.file), (Tab::Files, Focus::Diff, Some(0)));
    assert_eq!(cursor_row(&app), DiffRow::ThreadHead(3));
    assert_eq!(d.tree_cursor, 5, "tree follows to main.rs");
}

#[test]
fn threads_pane_scrolls_and_esc_goes_back() {
    let mut app = detail_app();
    app.handle_key(ctrl('d'));
    assert!(detail(&app).th_scroll > 0);
    app.handle_key(ctrl('u'));
    assert_eq!(detail(&app).th_scroll, 0);
    app.handle_key(ctrl('e'));
    assert_eq!(detail(&app).th_scroll, 1);
    typ(&mut app, "j");
    assert_eq!(detail(&app).th_scroll, 0, "new thread starts at the top");
    assert_eq!(typ(&mut app, "o"), vec![Effect::Open("https://example.test/src/main.rs#11".into())]);
    app.handle_key(key(KeyCode::Esc));
    assert!(app.detail.is_none());
}

#[test]
fn diff_rows_place_threads_under_their_lines() {
    let d = crate::testutil::detail();
    let rows = diff_rows(&d, 0, false, 80);
    let shape: Vec<String> = rows
        .iter()
        .map(|r| match r {
            DiffRow::Line(i) => format!("L{i}"),
            DiffRow::ThreadHead(t) => format!("T{t}"),
            DiffRow::Comment { .. } => "C".into(),
            DiffRow::Body(s) => format!("B:{s}"),
            DiffRow::Gap => "-".into(),
            DiffRow::Note(_) => "N".into(),
        })
        .collect();
    assert_eq!(
        shape,
        vec!["L0", "L1", "L2", "T3", "C", "B:keep b = 2", "-", "L3", "L4", "T0", "C", "B:why add c?", "-", "L5", "L6", "L7", "L8"]
    );
}

#[test]
fn diff_rows_outdated_on_top_resolved_collapsed_binary_noted() {
    let d = crate::testutil::detail();
    let readme = diff_rows(&d, 1, false, 80);
    assert!(matches!(readme[0], DiffRow::Note(_)));
    assert_eq!(readme[1], DiffRow::ThreadHead(2));

    let module = diff_rows(&d, 2, false, 80);
    assert_eq!(module.iter().filter(|r| matches!(r, DiffRow::Comment { .. })).count(), 0);
    assert!(module.contains(&DiffRow::ThreadHead(1)));
    let expanded = diff_rows(&d, 2, true, 80);
    assert_eq!(expanded.iter().filter(|r| matches!(r, DiffRow::Comment { .. })).count(), 1);

    let logo = diff_rows(&d, 3, false, 80);
    assert!(matches!(&logo[..], [DiffRow::Note(_)]));
}

#[test]
fn files_tab_tree_navigation_previews_files() {
    let mut app = detail_app();
    app.handle_key(key(KeyCode::Tab));
    let d = detail(&app);
    assert_eq!((d.tab, d.focus, d.file, d.tree_cursor), (Tab::Files, Focus::Tree, Some(3), 1));
    typ(&mut app, "jjj");
    assert_eq!(detail(&app).file, Some(2), "moving onto mod.rs previews it");
    typ(&mut app, "h");
    assert_eq!(detail(&app).tree_cursor, 3, "h on a file goes to its folder");
    typ(&mut app, "h");
    assert_eq!(detail(&app).tree_items(&data(&app)).len(), 6, "h on an open folder folds it");
    typ(&mut app, "l");
    assert_eq!(detail(&app).tree_items(&data(&app)).len(), 7);
    typ(&mut app, "k");
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(detail(&app).tree_items(&data(&app)).len(), 4, "enter toggles src/");
    app.handle_key(key(KeyCode::Enter));
    typ(&mut app, "G");
    app.handle_key(key(KeyCode::Enter));
    assert_eq!((detail(&app).focus, detail(&app).file), (Focus::Diff, Some(1)));
    typ(&mut app, "h");
    assert_eq!(detail(&app).focus, Focus::Tree);
}

#[test]
fn bracket_keys_walk_files_in_tree_order() {
    let mut app = detail_app();
    app.handle_key(key(KeyCode::Tab));
    let mut seen = vec![detail(&app).file];
    for _ in 0..4 {
        typ(&mut app, "]");
        seen.push(detail(&app).file);
    }
    assert_eq!(seen, vec![Some(3), Some(2), Some(0), Some(1), Some(1)]);
    typ(&mut app, "[");
    assert_eq!(detail(&app).file, Some(0));
    assert_eq!(detail(&app).tree_cursor, 5);
}

#[test]
fn folded_folder_reopens_when_jumping_into_it() {
    let mut app = detail_app();
    app.handle_key(key(KeyCode::Tab));
    typ(&mut app, "j");
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(detail(&app).tree_items(&data(&app)).len(), 4);
    typ(&mut app, "]");
    assert_eq!(detail(&app).file, Some(2));
    assert_eq!(detail(&app).tree_items(&data(&app)).len(), 7);
}

#[test]
fn file_fuzzy_find() {
    let mut app = detail_app();
    app.handle_key(key(KeyCode::Tab));
    typ(&mut app, "/mainrs");
    let d = detail(&app);
    assert!(d.file_searching);
    let items = d.tree_items(&data(&app));
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].0.name, "src/main.rs");
    assert_eq!(d.file, Some(0), "top match previews while typing");
    app.handle_key(key(KeyCode::Enter));
    let d = detail(&app);
    assert_eq!((d.file_searching, d.file_query.as_str(), d.focus, d.file), (false, "", Focus::Diff, Some(0)));
    assert_eq!(d.tree_cursor, 5);

    typ(&mut app, "/zzz");
    assert!(detail(&app).tree_items(&data(&app)).is_empty());
    app.handle_key(key(KeyCode::Esc));
    assert!(!detail(&app).file_searching);
    assert_eq!(detail(&app).file, Some(0));
}

#[test]
fn n_and_shift_n_cycle_unresolved_threads_across_files() {
    let mut app = detail_app();
    app.handle_key(key(KeyCode::Tab));
    typ(&mut app, "n");
    assert_eq!(cursor_row(&app), DiffRow::ThreadHead(3));
    assert_eq!(detail(&app).focus, Focus::Diff);
    typ(&mut app, "n");
    assert_eq!(cursor_row(&app), DiffRow::ThreadHead(0));
    typ(&mut app, "n");
    assert_eq!((detail(&app).file, cursor_row(&app)), (Some(1), DiffRow::ThreadHead(2)));
    typ(&mut app, "n");
    assert_eq!(cursor_row(&app), DiffRow::ThreadHead(3), "wraps around");
    typ(&mut app, "N");
    assert_eq!(cursor_row(&app), DiffRow::ThreadHead(2), "wraps backwards");
}

#[test]
fn hunk_jumps_and_diff_motions() {
    let mut app = detail_app();
    typ(&mut app, "j");
    app.handle_key(key(KeyCode::Enter));
    typ(&mut app, "gg");
    assert_eq!(detail(&app).diff_cursor, 0);
    typ(&mut app, "}");
    assert_eq!(cursor_row(&app), DiffRow::Line(6));
    typ(&mut app, "}");
    assert_eq!(cursor_row(&app), DiffRow::Line(6), "no further hunk");
    typ(&mut app, "{");
    assert_eq!(detail(&app).diff_cursor, 0);
    typ(&mut app, "G");
    assert_eq!(cursor_row(&app), DiffRow::Line(8));
    typ(&mut app, "2k");
    assert_eq!(cursor_row(&app), DiffRow::Line(6));
}

#[test]
fn t_in_diff_expands_resolved_threads() {
    let mut app = detail_app();
    app.handle_key(key(KeyCode::Tab));
    typ(&mut app, "]");
    let before = detail(&app).diff_rows(&data(&app)).len();
    typ(&mut app, "t");
    assert!(detail(&app).diff_rows(&data(&app)).len() > before);
}

#[test]
fn slash_in_diff_starts_file_search() {
    let mut app = detail_app();
    typ(&mut app, "j");
    app.handle_key(key(KeyCode::Enter));
    typ(&mut app, "/");
    assert!(detail(&app).file_searching);
    assert_eq!(detail(&app).focus, Focus::Tree);
}

#[test]
fn detail_errors_and_refresh() {
    let mut app = loaded_app();
    app.handle_key(key(KeyCode::Enter));
    app.on_msg(Msg::Detail { repo: "o/alpha".into(), number: 12, result: Err("boom".into()) });
    assert!(matches!(detail(&app).data, Load::Failed(ref e) if e == "boom"));
    assert_eq!(typ(&mut app, "r"), vec![Effect::LoadDetail { repo: "o/alpha".into(), number: 12 }]);
    app.on_msg(Msg::Detail { repo: "o/alpha".into(), number: 12, result: Ok(crate::testutil::detail()) });
    assert!(detail(&app).ready().is_some());
    app.on_msg(Msg::Detail { repo: "o/alpha".into(), number: 12, result: Err("later".into()) });
    assert!(detail(&app).ready().is_some(), "a failed refresh keeps the data");
    assert!(app.flash.as_deref().is_some_and(|f| f.contains("later")));
}

#[test]
fn files_tab_open_goes_to_files_page() {
    let mut app = detail_app();
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(typ(&mut app, "o"), vec![Effect::Open("https://github.com/o/alpha/pull/12/files".into())]);
}

#[test]
fn help_popup_closes_on_any_key() {
    let mut app = loaded_app();
    typ(&mut app, "?");
    assert!(matches!(app.popup, Some(Popup::Help)));
    assert_eq!(typ(&mut app, "q"), vec![], "q closes help instead of quitting");
    assert!(app.popup.is_none());
}

#[test]
fn wrap_respects_width_and_newlines() {
    assert_eq!(wrap("aaa bbb ccc", 7), vec!["aaa bbb", "ccc"]);
    assert_eq!(wrap("x\r\n\r\ny", 10), vec!["x", "", "y"]);
    assert_eq!(wrap("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
    assert_eq!(wrap("", 4), vec![""]);
}

#[test]
fn jumping_to_a_thread_scrolls_it_near_the_top() {
    let mut app = detail_app();
    typ(&mut app, "G");
    app.handle_key(key(KeyCode::Enter));
    let d = detail(&app);
    assert_eq!(cursor_row(&app), DiffRow::ThreadHead(0));
    assert_eq!(d.diff_offset, d.diff_cursor - 3);
}

fn stacked_prs() -> Vec<Pr> {
    let on = |number: u64, base: &str, hours: i64, draft: bool| {
        let mut p = pr("o/stack", number, &format!("pr {number}"), hours, 0, 0);
        p.base = base.into();
        p.is_draft = draft;
        p
    };
    vec![
        on(1, "master", 9, false),
        on(2, "b1", 5, false),
        on(3, "b2", 4, false),
        on(4, "b1", 1, false),
        on(5, "master", 2, true),
        on(6, "b5", 3, false),
        on(7, "master", 1, true),
    ]
}

fn stack_labels(rows: &[DashRow], prs: &[Pr]) -> Vec<String> {
    rows.iter()
        .filter_map(|r| match r {
            DashRow::Pr { idx, stack, .. } => Some(format!("{stack}#{}", prs[*idx].number)),
            DashRow::Group { .. } => None,
        })
        .collect()
}

#[test]
fn stacked_prs_nest_under_their_base_pr() {
    let prs = stacked_prs();
    let rows = dash_rows(&prs, "", &HashSet::new(), true);
    assert_eq!(rows[0], DashRow::Group { repo: "o/stack".into(), count: 7, collapsed: false });
    assert_eq!(stack_labels(&rows, &prs), vec!["#7", "#5", "└─ #6", "#1", "├─ #4", "└─ #2", "   └─ #3"]);
}

#[test]
fn drafts_hidden_unless_something_is_stacked_on_them() {
    let prs = stacked_prs();
    let rows = dash_rows(&prs, "", &HashSet::new(), false);
    assert_eq!(stack_labels(&rows, &prs), vec!["#5", "└─ #6", "#1", "├─ #4", "└─ #2", "   └─ #3"], "#7 hidden, #5 kept for #6");
    assert!(matches!(rows[0], DashRow::Group { count: 6, .. }));
}

#[test]
fn search_lists_matches_flat_and_skips_hidden_drafts() {
    let prs = stacked_prs();
    let rows = dash_rows(&prs, "pr", &HashSet::new(), false);
    let labels = stack_labels(&rows, &prs);
    assert_eq!(labels.len(), 5, "drafts #5 and #7 hidden: {labels:?}");
    assert!(labels.iter().all(|l| l.starts_with('#')));
}

#[test]
fn base_branch_cycle_still_lists_every_pr() {
    let mut prs = stacked_prs();
    prs[0].base = "b3".into();
    let rows = dash_rows(&prs, "", &HashSet::new(), true);
    assert_eq!(stack_labels(&rows, &prs).len(), 7);
}

#[test]
fn d_toggles_drafts_and_keeps_cursor() {
    let mut app = loaded_app();
    let mut fresh = prs();
    fresh[3].is_draft = true;
    app.on_msg(Msg::Prs { account: "work".into(), result: Ok(fresh) });
    assert!(!row_labels(&app).contains(&"#7".to_string()), "drafts hidden by default");
    assert_eq!(app.hidden_drafts(), 1);
    typ(&mut app, "j");
    let before = app.selected_pr().map(|p| p.number);
    typ(&mut app, "d");
    assert!(row_labels(&app).contains(&"#7".to_string()));
    assert_eq!(app.hidden_drafts(), 0);
    assert_eq!(app.selected_pr().map(|p| p.number), before);
    typ(&mut app, "Gd");
    assert_eq!(app.selected_pr().map(|p| p.number), Some(12), "cursor falls back when its PR hides");
}
