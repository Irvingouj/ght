use chrono::Utc;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{App, DashRow, Detail, DiffRow, Focus, Load, Popup, Status, Tab, follow, visible_threads, wrap};
use crate::model::{Checks, FileStatus, LineKind, PrDetail, Review, age};
use crate::tree::RowKind;

const ACCENT: Color = Color::Cyan;

// Plain text keeps the terminal's own foreground so it reads on light and dark themes alike.
fn dim() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

fn cursor_style() -> Style {
    Style::default().add_modifier(Modifier::REVERSED)
}

fn badge() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::REVERSED | Modifier::BOLD)
}

fn bold(color: Color) -> Style {
    Style::default().fg(color).add_modifier(Modifier::BOLD)
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let [top, body, foot] = Layout::vertical([Constraint::Length(1), Constraint::Fill(1), Constraint::Length(1)]).areas(f.area());
    draw_top(f, top, app);
    match app.detail.as_mut() {
        Some(d) => draw_detail(f, body, d),
        None => draw_dash(f, body, app),
    }
    draw_footer(f, foot, app);
    match &app.popup {
        Some(Popup::Help) => draw_help(f),
        Some(Popup::Accounts { cursor }) => draw_accounts(f, app, *cursor),
        None => {}
    }
}

fn draw_top(f: &mut Frame, area: Rect, app: &App) {
    let repos: std::collections::HashSet<&str> = app.prs.iter().map(|p| p.repo.as_str()).collect();
    let mut spans = vec![
        Span::styled(" ght ", badge()),
        Span::raw(" "),
        Span::styled(app.account.clone().unwrap_or_else(|| "…".into()), bold(Color::Reset)),
        Span::styled(format!("  {} open PRs in {} repos", app.prs.len(), repos.len()), dim()),
    ];
    match app.hidden_drafts() {
        0 if app.dash.show_drafts => spans.push(Span::styled("  drafts shown (d hides)", dim())),
        0 => {}
        n => spans.push(Span::styled(format!("  {n} drafts hidden (d shows)"), Style::default().fg(Color::Magenta))),
    }
    match &app.status {
        Status::Loading => spans.push(Span::styled("  loading…", bold(Color::Yellow))),
        Status::Failed(_) => spans.push(Span::styled("  load failed", bold(Color::Red))),
        Status::Idle => {
            if let Some(t) = app.loaded_at {
                spans.push(Span::styled(format!("  updated {}", age(Utc::now(), t)), dim()));
            }
        }
    }
    f.render_widget(Line::from(spans), area);
}

fn hints(pairs: &[(&str, &str)]) -> Line<'static> {
    let mut spans = vec![Span::raw(" ")];
    for (k, v) in pairs {
        spans.push(Span::styled(k.to_string(), bold(ACCENT)));
        spans.push(Span::styled(format!(" {v}  "), dim()));
    }
    Line::from(spans)
}

fn prompt(query: &str, extra: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(" / ", bold(Color::Yellow)),
        Span::styled(query.to_string(), bold(Color::Reset)),
        Span::styled("▏", Style::default().fg(Color::Yellow)),
        Span::styled(extra, dim()),
    ])
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    if let Some(msg) = &app.flash {
        f.render_widget(Line::styled(format!(" {msg}"), bold(Color::Yellow)), area);
        return;
    }
    let line = match &app.detail {
        None if app.dash.searching => {
            let n = app.dash_rows().iter().filter(|r| matches!(r, DashRow::Pr { .. })).count();
            let total = app.prs.iter().filter(|p| app.dash.show_drafts || !p.is_draft).count();
            prompt(&app.dash.query, format!("  {n}/{total}   ↑↓ move · enter open · esc clear"))
        }
        None => hints(&[
            ("j/k", "move"),
            ("enter", "open"),
            ("/", "search"),
            ("{ }", "repo"),
            ("h/l", "fold"),
            ("d", "drafts"),
            ("o", "browser"),
            ("r", "refresh"),
            ("a", "account"),
            ("?", "help"),
            ("q", "quit"),
        ]),
        Some(d) if d.file_searching => prompt(&d.file_query, "   ↑↓ move · enter open file · esc clear".into()),
        Some(d) => match (d.tab, d.focus) {
            (Tab::Threads, _) => hints(&[
                ("j/k", "thread"),
                ("enter", "go to code"),
                ("C-d/C-u", "scroll"),
                ("t", "show resolved"),
                ("tab", "files"),
                ("o", "browser"),
                ("r", "reload"),
                ("q", "back"),
            ]),
            (Tab::Files, Focus::Tree) => hints(&[
                ("j/k", "move"),
                ("enter", "view diff"),
                ("h/l", "fold"),
                ("/", "find file"),
                ("] [", "next/prev file"),
                ("n/N", "thread"),
                ("tab", "threads"),
                ("q", "back"),
            ]),
            (Tab::Files, Focus::Diff) => hints(&[
                ("j/k", "line"),
                ("C-d/C-u", "page"),
                ("{ }", "hunk"),
                ("] [", "file"),
                ("n/N", "thread"),
                ("t", "resolved"),
                ("h", "tree"),
                ("q", "back"),
            ]),
        },
    };
    f.render_widget(line, area);
}

fn truncate(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for c in text.chars() {
        let cw = c.width().unwrap_or(0);
        if w + cw + 1 > width {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push('…');
    out
}

/// Cuts from the left so a long path keeps its file name, shifting highlight positions to match.
fn tail_fit(text: &str, hl: &[u32], width: usize) -> (String, Vec<u32>) {
    if text.width() <= width || width < 2 {
        return (text.to_string(), hl.to_vec());
    }
    let chars: Vec<char> = text.chars().collect();
    let mut keep = 0;
    let mut w = 1;
    for c in chars.iter().rev() {
        let cw = c.width().unwrap_or(0);
        if w + cw > width {
            break;
        }
        w += cw;
        keep += 1;
    }
    let skip = (chars.len() - keep) as u32;
    let shown: String = std::iter::once('…').chain(chars[chars.len() - keep..].iter().copied()).collect();
    (shown, hl.iter().filter(|&&i| i >= skip).map(|&i| i - skip + 1).collect())
}

fn highlighted(text: &str, hl: &[u32], base: Style) -> Vec<Span<'static>> {
    if hl.is_empty() {
        return vec![Span::styled(text.to_string(), base)];
    }
    let hit = base.fg(Color::Yellow).add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
    text.chars()
        .enumerate()
        .map(|(i, c)| Span::styled(c.to_string(), if hl.contains(&(i as u32)) { hit } else { base }))
        .collect()
}

fn pad(s: &str, width: usize) -> String {
    format!("{s}{}", " ".repeat(width.saturating_sub(s.width())))
}

fn centered_msg(f: &mut Frame, area: Rect, lines: Vec<Line<'static>>) {
    let h = lines.len() as u16;
    let y = area.y + area.height.saturating_sub(h) / 2;
    f.render_widget(Paragraph::new(lines).centered(), Rect { y, height: h.min(area.height), ..area });
}

fn draw_dash(f: &mut Frame, area: Rect, app: &mut App) {
    let rows = app.dash_rows();
    if rows.is_empty() {
        let lines = match (&app.status, app.dash.query.is_empty()) {
            (_, false) => vec![Line::styled(format!("No pull request matches “{}”", app.dash.query), dim())],
            (Status::Loading, _) => vec![Line::styled("Loading your open pull requests…", bold(Color::Yellow))],
            (Status::Failed(e), _) => vec![
                Line::styled("Could not load pull requests", bold(Color::Red)),
                Line::raw(e.clone()),
                Line::styled("press r to retry, a to switch account", dim()),
            ],
            (Status::Idle, _) => vec![Line::styled("You have no open pull requests.", dim())],
        };
        centered_msg(f, area, lines);
        return;
    }

    let [head, list] = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(area);
    let num_w = app.prs.iter().map(|p| p.number.to_string().len() + 1).max().unwrap_or(2).max(2);
    let fixed = 2 + num_w + 2 + 3 + 9 + 8;
    let age_w = 5;
    let title_w = (area.width as usize).saturating_sub(fixed + age_w + 2);
    f.render_widget(
        Line::styled(
            format!("  {}  {}{}{}{}{}", pad("#", num_w), pad("CI", 3), pad("Review", 9), pad("Threads", 8), pad("Title", title_w), "  Age"),
            dim().add_modifier(Modifier::UNDERLINED),
        ),
        head,
    );

    let height = list.height as usize;
    app.dash.page = height;
    app.dash.cursor = app.dash.cursor.min(rows.len() - 1);
    follow(&mut app.dash.offset, app.dash.cursor, height);
    app.dash.offset = app.dash.offset.min(rows.len().saturating_sub(height));
    let now = Utc::now();
    for (i, row) in rows.iter().enumerate().skip(app.dash.offset).take(height) {
        let mut line = match row {
            DashRow::Group { repo, count, collapsed } => Line::from(vec![
                Span::styled(if *collapsed { "▸ " } else { "▾ " }, bold(ACCENT)),
                Span::styled(repo.clone(), bold(ACCENT)),
                Span::styled(format!("  {count}"), dim()),
            ]),
            DashRow::Pr { idx, hl, stack } => {
                let pr = &app.prs[*idx];
                let (ci, ci_style) = match pr.checks {
                    Some(Checks::Passing) => ("✓", bold(Color::Green)),
                    Some(Checks::Failing) => ("✗", bold(Color::Red)),
                    Some(Checks::Pending) => ("●", bold(Color::Yellow)),
                    None => ("", dim()),
                };
                let (rv, rv_style) = match pr.review {
                    Some(Review::Approved) => ("approved", Style::default().fg(Color::Green)),
                    Some(Review::ChangesRequested) => ("changes", Style::default().fg(Color::Red)),
                    Some(Review::Required) => ("review", Style::default().fg(Color::Yellow)),
                    None => ("", dim()),
                };
                let (th, th_style) = match (pr.threads_unresolved, pr.threads_total) {
                    (0, 0) => (String::new(), dim()),
                    (0, n) => (format!("✓ {n}"), Style::default().fg(Color::Green)),
                    (u, n) => (format!("● {u}/{n}"), bold(Color::Yellow)),
                };
                let mut spans = vec![
                    Span::raw("  "),
                    Span::styled(pad(&format!("#{}", pr.number), num_w), dim()),
                    Span::raw("  "),
                    Span::styled(pad(ci, 3), ci_style),
                    Span::styled(pad(rv, 9), rv_style),
                    Span::styled(pad(&th, 8), th_style),
                ];
                let mut room = title_w;
                if !stack.is_empty() {
                    spans.push(Span::styled(stack.clone(), Style::default().fg(Color::Blue)));
                    room = room.saturating_sub(stack.width());
                }
                if pr.is_draft {
                    spans.push(Span::styled("draft ", Style::default().fg(Color::Magenta)));
                    room = room.saturating_sub(6);
                }
                let title = truncate(&pr.title, room);
                let used = title.width();
                spans.extend(highlighted(&title, hl, Style::default()));
                spans.push(Span::raw(" ".repeat(room.saturating_sub(used))));
                spans.push(Span::styled(format!("{:>width$}", age(now, pr.updated_at), width = age_w + 1), dim()));
                Line::from(spans)
            }
        };
        if i == app.dash.cursor {
            line = line.style(cursor_style());
        }
        let y = list.y + (i - app.dash.offset) as u16;
        f.render_widget(line, Rect { y, height: 1, ..list });
    }
}

fn draw_detail(f: &mut Frame, area: Rect, d: &mut Detail) {
    let [head, tabs, body] = Layout::vertical([Constraint::Length(2), Constraint::Length(1), Constraint::Fill(1)]).areas(area);
    let pr = &d.pr;
    let mut meta = vec![
        Span::styled(format!(" {} → {}", pr.head, pr.base), dim()),
        Span::styled(format!("   +{}", pr.additions), Style::default().fg(Color::Green)),
        Span::styled(format!(" −{}", pr.deletions), Style::default().fg(Color::Red)),
    ];
    if pr.is_draft {
        meta.push(Span::styled("   draft", Style::default().fg(Color::Magenta)));
    }
    match pr.checks {
        Some(Checks::Passing) => meta.push(Span::styled("   CI ✓", Style::default().fg(Color::Green))),
        Some(Checks::Failing) => meta.push(Span::styled("   CI ✗", Style::default().fg(Color::Red))),
        Some(Checks::Pending) => meta.push(Span::styled("   CI ●", Style::default().fg(Color::Yellow))),
        None => {}
    }
    match pr.review {
        Some(Review::Approved) => meta.push(Span::styled("   approved", Style::default().fg(Color::Green))),
        Some(Review::ChangesRequested) => meta.push(Span::styled("   changes requested", Style::default().fg(Color::Red))),
        Some(Review::Required) => meta.push(Span::styled("   review required", Style::default().fg(Color::Yellow))),
        None => {}
    }
    if d.refreshing {
        meta.push(Span::styled("   refreshing…", bold(Color::Yellow)));
    }
    f.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(format!(" {} #{} ", pr.repo, pr.number), bold(ACCENT)),
                Span::styled(pr.title.clone(), bold(Color::Reset)),
            ]),
            Line::from(meta),
        ]),
        head,
    );

    let data = match &d.data {
        Load::Ready(data) => data.clone(),
        Load::Loading => {
            centered_msg(f, body, vec![Line::styled("Loading threads and diff…", bold(Color::Yellow))]);
            return;
        }
        Load::Failed(e) => {
            centered_msg(
                f,
                body,
                vec![Line::styled("Could not load this pull request", bold(Color::Red)), Line::raw(e.clone()), Line::styled("press r to retry, q to go back", dim())],
            );
            return;
        }
    };

    let unresolved = data.threads.iter().filter(|t| !t.is_resolved).count();
    let tab = |label: String, active: bool| {
        if active {
            Span::styled(label, badge())
        } else {
            Span::styled(label, dim())
        }
    };
    f.render_widget(
        Line::from(vec![
            Span::raw(" "),
            tab(format!(" Threads {unresolved}/{} ", data.threads.len()), d.tab == Tab::Threads),
            Span::raw(" "),
            tab(format!(" Files {} ", data.files.len()), d.tab == Tab::Files),
            Span::styled("   tab switches", dim()),
        ]),
        tabs,
    );

    match d.tab {
        Tab::Threads => draw_threads(f, body, d, &data),
        Tab::Files => draw_files(f, body, d, &data),
    }
}

fn pane(title: Line<'static>, focused: bool) -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(if focused { Style::default().fg(ACCENT) } else { dim() })
        .title(title)
}

fn draw_threads(f: &mut Frame, area: Rect, d: &mut Detail, data: &PrDetail) {
    let visible = visible_threads(data, d.show_resolved);
    if visible.is_empty() {
        let msg = if data.threads.is_empty() {
            vec![Line::styled("No review threads on this PR.", dim())]
        } else {
            vec![
                Line::styled(format!("All {} threads are resolved ✓", data.threads.len()), bold(Color::Green)),
                Line::styled("press t to show them · tab for files", dim()),
            ]
        };
        centered_msg(f, area, msg);
        return;
    }
    d.th_cursor = d.th_cursor.min(visible.len() - 1);
    let list_w = (area.width * 2 / 5).clamp(30.min(area.width), 70);
    let [left, right] = Layout::horizontal([Constraint::Length(list_w), Constraint::Fill(1)]).areas(area);

    let title = if d.show_resolved { " All threads " } else { " Unresolved threads " };
    let block = pane(Line::styled(title, bold(ACCENT)), true);
    let inner = block.inner(left);
    f.render_widget(block, left);
    let h = inner.height as usize;
    d.th_page = h;
    follow(&mut d.th_offset, d.th_cursor, h);
    for (row, &t) in visible.iter().enumerate().skip(d.th_offset).take(h) {
        let thread = &data.threads[t];
        let (icon, style) = if thread.is_resolved { ("✓ ", Style::default().fg(Color::Green)) } else { ("● ", bold(Color::Yellow)) };
        let first = thread.comments.first().map(|c| format!("{}: {}", c.author, c.body.lines().next().unwrap_or(""))).unwrap_or_default();
        let loc = thread.location();
        let room = (inner.width as usize).saturating_sub(2);
        let mut spans = vec![Span::styled(icon, style), Span::styled(truncate(&loc, room), bold(Color::Reset))];
        let rest = room.saturating_sub(loc.width() + 2);
        if rest > 4 {
            spans.push(Span::styled(format!("  {}", truncate(&first, rest)), dim()));
        }
        let mut line = Line::from(spans);
        if row == d.th_cursor {
            line = line.style(cursor_style());
        }
        f.render_widget(line, Rect { y: inner.y + (row - d.th_offset) as u16, height: 1, ..inner });
    }

    let t = &data.threads[visible[d.th_cursor]];
    let block = pane(Line::styled(format!(" {} ", t.location()), bold(Color::Reset)), false);
    let inner = block.inner(right);
    f.render_widget(block, right);
    let width = inner.width as usize;
    let mut lines: Vec<Line> = Vec::new();
    let mut status = vec![if t.is_resolved {
        Span::styled("✓ Resolved", bold(Color::Green))
    } else {
        Span::styled("● Unresolved", bold(Color::Yellow))
    }];
    if t.is_outdated {
        status.push(Span::styled("  outdated", Style::default().fg(Color::Magenta)));
    }
    status.push(Span::styled("   enter: jump to this line in the diff", dim()));
    lines.push(Line::from(status));
    lines.push(Line::raw(""));
    let hunk: Vec<&str> = t.diff_hunk.lines().collect();
    for l in &hunk[hunk.len().saturating_sub(8)..] {
        let style = match l.chars().next() {
            Some('+') => Style::default().fg(Color::Green),
            Some('-') => Style::default().fg(Color::Red),
            Some('@') => Style::default().fg(ACCENT),
            _ => Style::default(),
        };
        lines.push(Line::styled(truncate(&l.replace('\t', "    "), width), style));
    }
    for c in &t.comments {
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![
            Span::styled(c.author.clone(), bold(ACCENT)),
            Span::styled(format!(" · {}", age(Utc::now(), c.created_at)), dim()),
        ]));
        lines.extend(wrap(&c.body, width.saturating_sub(2)).into_iter().map(|l| Line::raw(format!("  {l}"))));
    }
    let max_scroll = lines.len().saturating_sub(inner.height as usize) as u16;
    d.th_scroll = d.th_scroll.min(max_scroll);
    f.render_widget(Paragraph::new(lines).scroll((d.th_scroll, 0)), inner);
}

fn draw_files(f: &mut Frame, area: Rect, d: &mut Detail, data: &PrDetail) {
    if data.files.is_empty() {
        centered_msg(f, area, vec![Line::styled("This PR changes no files.", dim())]);
        return;
    }
    let tree_w = if area.width < 70 { area.width / 3 } else { (area.width * 3 / 10).clamp(24, 50) };
    let [left, right] = Layout::horizontal([Constraint::Length(tree_w), Constraint::Fill(1)]).areas(area);

    let title = if d.file_searching || !d.file_query.is_empty() {
        Line::from(vec![Span::styled(" / ", bold(Color::Yellow)), Span::styled(format!("{} ", d.file_query), bold(Color::Reset))])
    } else {
        Line::styled(format!(" Files {} ", data.files.len()), bold(ACCENT))
    };
    let block = pane(title, d.focus == Focus::Tree);
    let inner = block.inner(left);
    f.render_widget(block, left);
    let items = d.tree_items(data);
    let h = inner.height as usize;
    d.tree_page = h;
    d.tree_cursor = d.tree_cursor.min(items.len().saturating_sub(1));
    follow(&mut d.tree_offset, d.tree_cursor, h);
    if items.is_empty() {
        f.render_widget(Line::styled(" no file matches", dim()), inner);
    }
    for (row, (item, hl)) in items.iter().enumerate().skip(d.tree_offset).take(h) {
        let indent = "  ".repeat(item.depth);
        let mut spans = vec![Span::raw(format!(" {indent}"))];
        match &item.kind {
            RowKind::Dir(_) => {
                spans.push(Span::styled(format!("{} {}", if item.collapsed { "▸" } else { "▾" }, item.name), bold(Color::Blue)));
            }
            RowKind::File(i) => {
                let file = &data.files[*i];
                let color = match file.status {
                    FileStatus::Added => Color::Green,
                    FileStatus::Removed => Color::Red,
                    FileStatus::Renamed => Color::Magenta,
                    _ => Color::Yellow,
                };
                spans.push(Span::styled(format!("{} ", file.status.letter()), bold(color)));
                let name_style = if d.file == Some(*i) { bold(Color::Reset) } else { Style::default() };
                let open = data.threads.iter().filter(|t| !t.is_resolved && t.path == file.path).count();
                let badge = if open > 0 { 3 + open.to_string().len() } else { 0 };
                let room = (inner.width as usize).saturating_sub(item.depth * 2 + 3 + badge);
                let (name, hl) = tail_fit(&item.name, hl, room);
                spans.extend(highlighted(&name, &hl, name_style));
                if open > 0 {
                    spans.push(Span::styled(format!(" ●{open}"), bold(Color::Yellow)));
                }
            }
        }
        let mut line = Line::from(spans);
        if row == d.tree_cursor {
            line = line.style(if d.focus == Focus::Tree { cursor_style() } else { Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED) });
        }
        f.render_widget(line, Rect { y: inner.y + (row - d.tree_offset) as u16, height: 1, ..inner });
    }

    let Some(fi) = d.file else { return };
    let file = &data.files[fi];
    let mut title = vec![Span::styled(format!(" {} ", file.path), bold(Color::Reset))];
    if let Some(prev) = &file.previous_path {
        title.push(Span::styled(format!("(from {prev}) "), dim()));
    }
    title.push(Span::styled(format!("+{} ", file.additions), Style::default().fg(Color::Green)));
    title.push(Span::styled(format!("−{} ", file.deletions), Style::default().fg(Color::Red)));
    let block = pane(Line::from(title), d.focus == Focus::Diff);
    let inner = block.inner(right);
    f.render_widget(block, right);
    d.diff_width = inner.width as usize;
    let h = inner.height as usize;
    d.diff_page = h;
    let rows = d.diff_rows(data);
    d.diff_cursor = d.diff_cursor.min(rows.len().saturating_sub(1));
    follow(&mut d.diff_offset, d.diff_cursor, h);
    for (i, row) in rows.iter().enumerate().skip(d.diff_offset).take(h) {
        let mut line = diff_line(row, data, file, d.show_resolved);
        if i == d.diff_cursor && d.focus == Focus::Diff {
            line = line.style(cursor_style());
        }
        f.render_widget(line, Rect { y: inner.y + (i - d.diff_offset) as u16, height: 1, ..inner });
    }
}

fn diff_line(row: &DiffRow, data: &PrDetail, file: &crate::model::FileDiff, show_resolved: bool) -> Line<'static> {
    let bar = |resolved: bool| Span::styled("            ┃ ", if resolved { Style::default().fg(Color::Green) } else { bold(Color::Yellow) });
    match row {
        DiffRow::Note(s) => Line::styled(format!(" {s}"), dim().add_modifier(Modifier::ITALIC)),
        DiffRow::Line(i) => {
            let l = &file.lines[*i];
            let num = |n: Option<u32>| n.map(|n| n.to_string()).unwrap_or_default();
            let text = l.text.replace('\t', "    ");
            let gutter = Span::styled(format!("{:>5} {:>5} ", num(l.old), num(l.new)), dim());
            match l.kind {
                LineKind::Hunk => Line::from(vec![Span::styled("            ", dim()), Span::styled(text, Style::default().fg(ACCENT))]),
                LineKind::Add => Line::from(vec![gutter, Span::styled(format!("+{text}"), Style::default().fg(Color::Green))]),
                LineKind::Del => Line::from(vec![gutter, Span::styled(format!("-{text}"), Style::default().fg(Color::Red))]),
                LineKind::Ctx => Line::from(vec![gutter, Span::raw(format!(" {text}"))]),
                LineKind::Meta => Line::from(vec![gutter, Span::styled(text, dim())]),
            }
        }
        DiffRow::ThreadHead(t) => {
            let thread = &data.threads[*t];
            let n = thread.comments.len();
            let outdated = if thread.is_outdated { " · outdated" } else { "" };
            let text = if !thread.is_resolved {
                Span::styled(format!("● Unresolved thread · {n} comment(s){outdated}"), bold(Color::Yellow))
            } else if show_resolved {
                Span::styled(format!("✓ Resolved thread · {n} comment(s){outdated}"), Style::default().fg(Color::Green))
            } else {
                let first = thread.comments.first().map(|c| format!("{}: {}", c.author, c.body.lines().next().unwrap_or(""))).unwrap_or_default();
                Span::styled(format!("✓ Resolved · {}{outdated}", truncate(&first, 80)), Style::default().fg(Color::Green))
            };
            Line::from(vec![bar(thread.is_resolved), text])
        }
        DiffRow::Comment { thread, comment } => {
            let t = &data.threads[*thread];
            let c = &t.comments[*comment];
            Line::from(vec![
                bar(t.is_resolved),
                Span::styled(c.author.clone(), bold(ACCENT)),
                Span::styled(format!(" · {}", age(Utc::now(), c.created_at)), dim()),
            ])
        }
        DiffRow::Body(s) => Line::from(vec![Span::styled("            ┃   ", dim()), Span::raw(s.clone())]),
        DiffRow::Gap => Line::raw(""),
    }
}

fn popup_area(f: &Frame, w: u16, h: u16) -> Rect {
    let a = f.area();
    let w = w.min(a.width);
    let h = h.min(a.height);
    Rect { x: a.x + (a.width - w) / 2, y: a.y + (a.height - h) / 2, width: w, height: h }
}

const HELP: &[(&str, &[(&str, &str)])] = &[
    (
        "Everywhere",
        &[
            ("j k ↓ ↑", "move (a count works: 5j)"),
            ("gg G", "top / bottom"),
            ("C-d C-u", "half page down / up"),
            ("o", "open in browser"),
            ("r", "reload"),
            ("q", "back / quit"),
        ],
    ),
    (
        "PR list",
        &[
            ("enter l", "open PR"),
            ("/", "fuzzy search (fzf syntax: 'exact ^start end$ !not)"),
            ("{ }", "previous / next repo"),
            ("h l space", "fold / unfold repo"),
            ("d", "show / hide drafts"),
            ("a", "switch gh account"),
        ],
    ),
    (
        "PR · Threads tab",
        &[("enter", "jump to the code of this thread"), ("C-d C-u C-e C-y", "scroll the thread"), ("t", "show / hide resolved threads"), ("tab", "go to Files")],
    ),
    (
        "PR · Files tab",
        &[
            ("enter l", "open file / unfold folder"),
            ("h", "fold folder · back to tree from diff"),
            ("/", "fuzzy find a file"),
            ("] [", "next / previous file"),
            ("{ }", "next / previous hunk"),
            ("n N", "next / previous unresolved thread"),
            ("t", "expand resolved threads"),
        ],
    ),
];

fn draw_help(f: &mut Frame) {
    let mut lines = Vec::new();
    for (section, keys) in HELP {
        lines.push(Line::styled(*section, bold(ACCENT)));
        for (k, v) in *keys {
            lines.push(Line::from(vec![Span::styled(format!("  {k:<17}"), bold(Color::Reset)), Span::raw(*v)]));
        }
        lines.push(Line::raw(""));
    }
    let area = popup_area(f, 72, lines.len() as u16 + 2);
    f.render_widget(Clear, area);
    f.render_widget(Paragraph::new(lines).block(pane(Line::styled(" Keys · any key closes ", bold(ACCENT)), true)), area);
}

fn draw_accounts(f: &mut Frame, app: &App, cursor: usize) {
    let lines: Vec<Line> = app
        .accounts
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let mark = if Some(a) == app.account.as_ref() { "● " } else { "  " };
            let line = Line::from(vec![Span::styled(mark, bold(Color::Green)), Span::raw(a.clone())]);
            if i == cursor { line.style(cursor_style()) } else { line }
        })
        .collect();
    let area = popup_area(f, 44, lines.len() as u16 + 2);
    f.render_widget(Clear, area);
    f.render_widget(Paragraph::new(lines).block(pane(Line::styled(" Switch account · enter ", bold(ACCENT)), true)), area);
}

#[cfg(test)]
#[path = "ui_tests.rs"]
mod tests;
