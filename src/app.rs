use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

use chrono::{DateTime, Utc};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_width::UnicodeWidthChar;

use crate::fuzzy::Fuzzy;
use crate::github::Account;
use crate::model::{FileDiff, LineKind, Pr, PrDetail, Side, Thread};
use crate::tree::{self, RowKind, TreeRow};

/// Columns the diff view spends before a thread's body text starts.
pub const THREAD_INDENT: usize = 16;

/// Code lines kept above a thread you jump to, so its comments fit below it.
const THREAD_CONTEXT: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Quit,
    LoadPrs,
    LoadDetail { repo: String, number: u64 },
    Open(String),
}

pub enum Msg {
    Accounts(Result<Vec<Account>, String>),
    Prs { account: String, result: Result<Vec<Pr>, String> },
    Detail { repo: String, number: u64, result: Result<PrDetail, String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Loading,
    Failed(String),
    Idle,
}

pub enum Load<T> {
    Loading,
    Failed(String),
    Ready(T),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    Down(usize),
    Up(usize),
    Top,
    Bottom,
    HalfDown,
    HalfUp,
}

pub fn apply(motion: Motion, cursor: usize, len: usize, page: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let half = (page / 2).max(1);
    let last = len - 1;
    match motion {
        Motion::Down(n) => (cursor + n).min(last),
        Motion::Up(n) => cursor.saturating_sub(n),
        Motion::Top => 0,
        Motion::Bottom => last,
        Motion::HalfDown => (cursor + half).min(last),
        Motion::HalfUp => cursor.saturating_sub(half),
    }
}

pub fn follow(offset: &mut usize, cursor: usize, height: usize) {
    if height == 0 {
        return;
    }
    if cursor < *offset {
        *offset = cursor;
    } else if cursor >= *offset + height {
        *offset = cursor + 1 - height;
    }
}

enum Pre {
    Motion(Motion),
    Consumed,
    Other,
}

#[derive(Default)]
struct Keys {
    count: Option<usize>,
    g: bool,
}

impl Keys {
    fn motion(&mut self, key: &KeyEvent) -> Pre {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if let KeyCode::Char(c) = key.code {
            if !ctrl && c.is_ascii_digit() && (c != '0' || self.count.is_some()) {
                self.count = Some(self.count.unwrap_or(0) * 10 + c.to_digit(10).unwrap_or(0) as usize);
                self.g = false;
                return Pre::Consumed;
            }
            if !ctrl && c == 'g' {
                if std::mem::take(&mut self.g) {
                    self.count = None;
                    return Pre::Motion(Motion::Top);
                }
                self.g = true;
                return Pre::Consumed;
            }
        }
        self.g = false;
        let n = self.count.take().unwrap_or(1);
        let m = match key.code {
            KeyCode::Char('j') | KeyCode::Down if !ctrl => Motion::Down(n),
            KeyCode::Char('k') | KeyCode::Up if !ctrl => Motion::Up(n),
            KeyCode::Char('G') | KeyCode::End => Motion::Bottom,
            KeyCode::Home => Motion::Top,
            KeyCode::Char('d') if ctrl => Motion::HalfDown,
            KeyCode::Char('u') if ctrl => Motion::HalfUp,
            KeyCode::PageDown => Motion::HalfDown,
            KeyCode::PageUp => Motion::HalfUp,
            _ => return Pre::Other,
        };
        Pre::Motion(m)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DashRow {
    Group { repo: String, count: usize, collapsed: bool },
    /// `stack` is the tree drawing (`├─ `, `│  └─ `) that puts a stacked PR under the PR it builds on.
    Pr { idx: usize, hl: Vec<u32>, stack: String },
}

/// The open PR in the same repo whose head branch this PR merges into.
pub fn stack_parent(prs: &[Pr], i: usize) -> Option<usize> {
    let pr = &prs[i];
    prs.iter().position(|p| p.repo == pr.repo && p.head == pr.base && p.number != pr.number)
}

/// Which PRs the list shows. Drafts are hidden unless `show_drafts`, but a hidden
/// draft that a shown PR is stacked on stays, so the stack still reads top to bottom.
pub fn shown_prs(prs: &[Pr], show_drafts: bool) -> Vec<bool> {
    let kept: Vec<bool> = prs.iter().map(|p| show_drafts || !p.is_draft).collect();
    let mut shown = kept.clone();
    for i in (0..prs.len()).filter(|&i| kept[i]) {
        let mut at = stack_parent(prs, i);
        for _ in 0..prs.len() {
            let Some(p) = at else { break };
            shown[p] = true;
            at = stack_parent(prs, p);
        }
    }
    shown
}

/// Without a query: repos A→Z, newest stack first, stacked PRs indented under
/// their base PR. With a query: only matches, best-scoring first, like fzf.
pub fn dash_rows(prs: &[Pr], query: &str, collapsed: &HashSet<String>, show_drafts: bool) -> Vec<DashRow> {
    if !query.trim().is_empty() {
        return search_rows(prs, query, show_drafts);
    }
    let shown = shown_prs(prs, show_drafts);
    let mut groups: BTreeMap<String, (String, Vec<usize>)> = BTreeMap::new();
    for i in (0..prs.len()).filter(|&i| shown[i]) {
        groups.entry(prs[i].repo.to_lowercase()).or_insert_with(|| (prs[i].repo.clone(), Vec::new())).1.push(i);
    }
    let newest = |v: &mut Vec<usize>| v.sort_by(|&a, &b| prs[b].updated_at.cmp(&prs[a].updated_at));
    let mut rows = Vec::new();
    for (repo, items) in groups.into_values() {
        let is_collapsed = collapsed.contains(&repo);
        rows.push(DashRow::Group { repo, count: items.len(), collapsed: is_collapsed });
        if is_collapsed {
            continue;
        }
        let mut children: HashMap<usize, Vec<usize>> = HashMap::new();
        let mut roots = Vec::new();
        for &i in &items {
            match stack_parent(prs, i).filter(|&p| shown[p]) {
                Some(p) => children.entry(p).or_default().push(i),
                None => roots.push(i),
            }
        }
        newest(&mut roots);
        children.values_mut().for_each(newest);
        let mut seen = HashSet::new();
        // Items left over after the walk sit on a base-branch cycle; list them flat.
        for i in roots.into_iter().chain(items) {
            push_stack(i, "", "", &children, &mut seen, &mut rows);
        }
    }
    rows
}

fn push_stack(i: usize, lead: &str, branch: &str, children: &HashMap<usize, Vec<usize>>, seen: &mut HashSet<usize>, rows: &mut Vec<DashRow>) {
    if !seen.insert(i) {
        return;
    }
    rows.push(DashRow::Pr { idx: i, hl: Vec::new(), stack: format!("{lead}{branch}") });
    let lead = match branch {
        "" => lead.to_string(),
        "└─ " => format!("{lead}   "),
        _ => format!("{lead}│  "),
    };
    let kids = children.get(&i).map(Vec::as_slice).unwrap_or_default();
    for (n, &k) in kids.iter().enumerate() {
        push_stack(k, &lead, if n + 1 == kids.len() { "└─ " } else { "├─ " }, children, seen, rows);
    }
}

fn search_rows(prs: &[Pr], query: &str, show_drafts: bool) -> Vec<DashRow> {
    type Hit = (usize, u32, Vec<u32>);
    let mut groups: BTreeMap<String, (String, Vec<Hit>)> = BTreeMap::new();
    let mut fuzzy = Fuzzy::new(query);
    for (i, pr) in prs.iter().enumerate().filter(|(_, p)| show_drafts || !p.is_draft) {
        let hay = format!("{} #{} {} {}", pr.title, pr.number, pr.repo, pr.head);
        let Some((score, idx)) = fuzzy.matches(&hay) else { continue };
        let title_len = pr.title.chars().count() as u32;
        let hl = idx.into_iter().filter(|&c| c < title_len).collect();
        groups.entry(pr.repo.to_lowercase()).or_insert_with(|| (pr.repo.clone(), Vec::new())).1.push((i, score, hl));
    }
    let mut groups: Vec<(String, Vec<Hit>)> = groups.into_values().collect();
    for (_, items) in &mut groups {
        items.sort_by(|a, b| b.1.cmp(&a.1).then(prs[b.0].updated_at.cmp(&prs[a.0].updated_at)));
    }
    groups.sort_by_key(|(_, items)| std::cmp::Reverse(items.iter().map(|x| x.1).max().unwrap_or(0)));
    let mut rows = Vec::new();
    for (repo, items) in groups {
        rows.push(DashRow::Group { repo, count: items.len(), collapsed: false });
        rows.extend(items.into_iter().map(|(idx, _, hl)| DashRow::Pr { idx, hl, stack: String::new() }));
    }
    rows
}

#[derive(Default)]
pub struct Dash {
    pub cursor: usize,
    pub offset: usize,
    pub page: usize,
    pub collapsed: HashSet<String>,
    pub query: String,
    pub searching: bool,
    pub show_drafts: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Threads,
    Files,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Tree,
    Diff,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffRow {
    Note(String),
    Line(usize),
    ThreadHead(usize),
    Comment { thread: usize, comment: usize },
    Body(String),
    Gap,
}

pub fn visible_threads(data: &PrDetail, show_resolved: bool) -> Vec<usize> {
    let mut v: Vec<usize> = (0..data.threads.len()).filter(|&i| show_resolved || !data.threads[i].is_resolved).collect();
    v.sort_by(|&a, &b| {
        let (x, y) = (&data.threads[a], &data.threads[b]);
        (x.is_resolved, &x.path, x.line).cmp(&(y.is_resolved, &y.path, y.line))
    });
    v
}

pub fn tree_items(files: &[FileDiff], collapsed: &HashSet<String>, query: &str) -> Vec<(TreeRow, Vec<u32>)> {
    let paths: Vec<String> = files.iter().map(|f| f.path.clone()).collect();
    if query.trim().is_empty() {
        return tree::rows(&paths, collapsed).into_iter().map(|r| (r, Vec::new())).collect();
    }
    let mut fuzzy = Fuzzy::new(query);
    let mut hits: Vec<(u32, usize, Vec<u32>)> =
        paths.iter().enumerate().filter_map(|(i, p)| fuzzy.matches(p).map(|(s, hl)| (s, i, hl))).collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(paths[a.1].cmp(&paths[b.1])));
    hits.into_iter()
        .map(|(_, i, hl)| (TreeRow { depth: 0, name: paths[i].clone(), kind: RowKind::File(i), collapsed: false }, hl))
        .collect()
}

fn anchor(file: &FileDiff, t: &Thread) -> Option<usize> {
    if t.is_outdated {
        return None;
    }
    let line = t.line?;
    file.lines.iter().position(|l| match t.side {
        Side::Right => l.new == Some(line),
        Side::Left => l.old == Some(line) && l.kind == LineKind::Del,
    })
}

/// Diff lines with each review thread placed right under the line it comments on.
/// Resolved threads collapse to their head row unless `show_resolved` is on.
pub fn diff_rows(data: &PrDetail, file: usize, show_resolved: bool, width: usize) -> Vec<DiffRow> {
    let f = &data.files[file];
    let mut anchored: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut unplaced = Vec::new();
    let mut threads: Vec<usize> = (0..data.threads.len()).filter(|&i| data.threads[i].path == f.path).collect();
    threads.sort_by_key(|&i| data.threads[i].line);
    for t in threads {
        match anchor(f, &data.threads[t]) {
            Some(line) => anchored.entry(line).or_default().push(t),
            None => unplaced.push(t),
        }
    }

    let body_width = width.saturating_sub(THREAD_INDENT).max(20);
    let push_thread = |rows: &mut Vec<DiffRow>, t: usize| {
        rows.push(DiffRow::ThreadHead(t));
        let thread = &data.threads[t];
        if thread.is_resolved && !show_resolved {
            return;
        }
        for (ci, c) in thread.comments.iter().enumerate() {
            rows.push(DiffRow::Comment { thread: t, comment: ci });
            rows.extend(wrap(&c.body, body_width).into_iter().map(DiffRow::Body));
        }
        rows.push(DiffRow::Gap);
    };

    let mut rows = Vec::new();
    if !f.has_patch {
        rows.push(DiffRow::Note("No text diff: binary file, or too large for GitHub to show.".into()));
    }
    if !unplaced.is_empty() {
        rows.push(DiffRow::Note(format!("{} thread(s) not on a line of this diff (outdated or moved):", unplaced.len())));
        for t in unplaced {
            push_thread(&mut rows, t);
        }
    }
    for i in 0..f.lines.len() {
        rows.push(DiffRow::Line(i));
        if let Some(ts) = anchored.get(&i) {
            for &t in ts {
                push_thread(&mut rows, t);
            }
        }
    }
    rows
}

pub struct Detail {
    pub pr: Pr,
    pub data: Load<Rc<PrDetail>>,
    pub refreshing: bool,
    pub tab: Tab,
    pub show_resolved: bool,
    pub th_cursor: usize,
    pub th_offset: usize,
    pub th_page: usize,
    pub th_scroll: u16,
    pub focus: Focus,
    pub collapsed: HashSet<String>,
    pub tree_cursor: usize,
    pub tree_offset: usize,
    pub tree_page: usize,
    pub file: Option<usize>,
    pub diff_cursor: usize,
    pub diff_offset: usize,
    pub diff_page: usize,
    pub diff_width: usize,
    pub file_query: String,
    pub file_searching: bool,
}

impl Detail {
    fn new(pr: Pr, data: Load<Rc<PrDetail>>) -> Self {
        let tab = if pr.threads_unresolved > 0 { Tab::Threads } else { Tab::Files };
        let mut d = Self {
            pr,
            data: Load::Loading,
            refreshing: false,
            tab,
            show_resolved: false,
            th_cursor: 0,
            th_offset: 0,
            th_page: 20,
            th_scroll: 0,
            focus: Focus::Tree,
            collapsed: HashSet::new(),
            tree_cursor: 0,
            tree_offset: 0,
            tree_page: 20,
            file: None,
            diff_cursor: 0,
            diff_offset: 0,
            diff_page: 20,
            diff_width: 0,
            file_query: String::new(),
            file_searching: false,
        };
        d.set_data(data);
        d
    }

    pub fn ready(&self) -> Option<Rc<PrDetail>> {
        match &self.data {
            Load::Ready(d) => Some(d.clone()),
            _ => None,
        }
    }

    fn set_data(&mut self, data: Load<Rc<PrDetail>>) {
        self.data = data;
        let Some(d) = self.ready() else { return };
        self.th_cursor = self.th_cursor.min(visible_threads(&d, self.show_resolved).len().saturating_sub(1));
        match self.file {
            Some(f) if f < d.files.len() => self.clamp_diff(&d),
            _ => match tree::file_order(&d.files.iter().map(|f| f.path.clone()).collect::<Vec<_>>()).first() {
                Some(&first) => self.select_file(&d, first),
                None => self.file = None,
            },
        }
    }

    pub fn tree_items(&self, d: &PrDetail) -> Vec<(TreeRow, Vec<u32>)> {
        tree_items(&d.files, &self.collapsed, &self.file_query)
    }

    pub fn diff_rows(&self, d: &PrDetail) -> Vec<DiffRow> {
        match self.file {
            Some(f) => diff_rows(d, f, self.show_resolved, self.diff_width),
            None => Vec::new(),
        }
    }

    fn clamp_diff(&mut self, d: &PrDetail) {
        self.diff_cursor = self.diff_cursor.min(self.diff_rows(d).len().saturating_sub(1));
    }

    fn select_file(&mut self, d: &PrDetail, file: usize) {
        let path = &d.files[file].path;
        self.collapsed.retain(|dir| !path.starts_with(&format!("{dir}/")));
        self.file = Some(file);
        self.diff_cursor = 0;
        self.diff_offset = 0;
        if let Some(pos) = self.tree_items(d).iter().position(|(r, _)| r.kind == RowKind::File(file)) {
            self.tree_cursor = pos;
        }
    }

    fn step_file(&mut self, d: &PrDetail, forward: bool) {
        let order = tree::file_order(&d.files.iter().map(|f| f.path.clone()).collect::<Vec<_>>());
        let Some(cur) = self.file.and_then(|f| order.iter().position(|&x| x == f)) else { return };
        let next = if forward { (cur + 1).min(order.len() - 1) } else { cur.saturating_sub(1) };
        self.select_file(d, order[next]);
    }

    fn jump_to_thread(&mut self, d: &PrDetail, t: usize) -> Option<String> {
        let path = &d.threads[t].path;
        let Some(file) = d.files.iter().position(|f| &f.path == path) else {
            return Some(format!("{path} is no longer part of this diff"));
        };
        self.tab = Tab::Files;
        self.focus = Focus::Diff;
        self.file_query.clear();
        self.file_searching = false;
        self.select_file(d, file);
        self.diff_cursor = self.diff_rows(d).iter().position(|r| *r == DiffRow::ThreadHead(t)).unwrap_or(0);
        self.diff_offset = self.diff_cursor.saturating_sub(THREAD_CONTEXT);
        None
    }

    /// Next/previous unresolved thread in tree order, wrapping across files.
    fn step_thread(&mut self, d: &PrDetail, forward: bool) -> Option<String> {
        let order = tree::file_order(&d.files.iter().map(|f| f.path.clone()).collect::<Vec<_>>());
        let mut stops: Vec<(usize, usize, usize)> = Vec::new();
        for (fpos, &f) in order.iter().enumerate() {
            for (rpos, row) in diff_rows(d, f, self.show_resolved, self.diff_width).iter().enumerate() {
                if let DiffRow::ThreadHead(t) = row
                    && !d.threads[*t].is_resolved
                {
                    stops.push((fpos, rpos, *t));
                }
            }
        }
        if stops.is_empty() {
            return Some("No unresolved threads in this PR".into());
        }
        let here = (self.file.and_then(|f| order.iter().position(|&x| x == f)).unwrap_or(0), self.diff_cursor);
        let target = if forward {
            stops.iter().find(|s| (s.0, s.1) > here).unwrap_or(&stops[0])
        } else {
            stops.iter().rev().find(|s| (s.0, s.1) < here).unwrap_or(&stops[stops.len() - 1])
        };
        self.focus = Focus::Diff;
        self.select_file(d, order[target.0]);
        self.diff_cursor = target.1;
        self.diff_offset = self.diff_cursor.saturating_sub(THREAD_CONTEXT);
        None
    }
}

pub enum Popup {
    Help,
    Accounts { cursor: usize },
}

pub struct App {
    pub accounts: Vec<String>,
    pub account: Option<String>,
    pub prs: Vec<Pr>,
    pub status: Status,
    pub loaded_at: Option<DateTime<Utc>>,
    pub dash: Dash,
    pub detail: Option<Detail>,
    pub cache: HashMap<(String, u64), Rc<PrDetail>>,
    pub popup: Option<Popup>,
    pub flash: Option<String>,
    keys: Keys,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        Self {
            accounts: Vec::new(),
            account: None,
            prs: Vec::new(),
            status: Status::Loading,
            loaded_at: None,
            dash: Dash { page: 20, ..Dash::default() },
            detail: None,
            cache: HashMap::new(),
            popup: None,
            flash: None,
            keys: Keys::default(),
        }
    }

    pub fn dash_rows(&self) -> Vec<DashRow> {
        dash_rows(&self.prs, &self.dash.query, &self.dash.collapsed, self.dash.show_drafts)
    }

    fn put_cursor_on(&mut self, key: Option<(String, u64)>) {
        let rows = self.dash_rows();
        self.dash.cursor = key
            .and_then(|k| rows.iter().position(|r| matches!(r, DashRow::Pr { idx, .. } if self.prs[*idx].key() == k)))
            .unwrap_or_else(|| first_pr_row(&rows));
    }

    pub fn hidden_drafts(&self) -> usize {
        if self.dash.show_drafts {
            return 0;
        }
        let shown = shown_prs(&self.prs, false);
        self.prs.iter().zip(shown).filter(|(p, shown)| p.is_draft && !shown).count()
    }

    pub fn selected_pr(&self) -> Option<&Pr> {
        match self.dash_rows().get(self.dash.cursor) {
            Some(DashRow::Pr { idx, .. }) => self.prs.get(*idx),
            _ => None,
        }
    }

    pub fn on_msg(&mut self, msg: Msg) -> Option<Effect> {
        match msg {
            Msg::Accounts(Ok(list)) => {
                self.account = list.iter().find(|a| a.active).or(list.first()).map(|a| a.login.clone());
                self.accounts = list.into_iter().map(|a| a.login).collect();
                if self.account.is_none() {
                    self.status = Status::Failed("No GitHub account found. Run `gh auth login` first.".into());
                    return None;
                }
                Some(Effect::LoadPrs)
            }
            Msg::Accounts(Err(e)) => {
                self.status = Status::Failed(e);
                None
            }
            Msg::Prs { account, result } => {
                if self.account.as_deref() != Some(account.as_str()) {
                    return None;
                }
                match result {
                    Ok(prs) => {
                        let keep = self.selected_pr().map(Pr::key);
                        self.prs = prs;
                        self.status = Status::Idle;
                        self.loaded_at = Some(Utc::now());
                        self.put_cursor_on(keep);
                    }
                    Err(e) => self.status = Status::Failed(e),
                }
                None
            }
            Msg::Detail { repo, number, result } => {
                let open = self.detail.as_mut().filter(|d| d.pr.repo == repo && d.pr.number == number);
                match result {
                    Ok(data) => {
                        let data = Rc::new(data);
                        self.cache.insert((repo, number), data.clone());
                        if let Some(d) = open {
                            d.refreshing = false;
                            d.set_data(Load::Ready(data));
                        }
                    }
                    Err(e) => {
                        if let Some(d) = open {
                            d.refreshing = false;
                            if d.ready().is_some() {
                                self.flash = Some(format!("Refresh failed: {e}"));
                            } else {
                                d.data = Load::Failed(e);
                            }
                        }
                    }
                }
                None
            }
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Option<Effect> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Some(Effect::Quit);
        }
        self.flash = None;
        if self.popup.is_some() {
            return self.handle_popup(key);
        }
        if self.detail.is_some() {
            return self.handle_detail(key);
        }
        self.handle_dash(key)
    }

    fn handle_popup(&mut self, key: KeyEvent) -> Option<Effect> {
        let Some(Popup::Accounts { cursor }) = &mut self.popup else {
            self.popup = None;
            return None;
        };
        match self.keys.motion(&key) {
            Pre::Motion(m) => *cursor = apply(m, *cursor, self.accounts.len(), 10),
            Pre::Consumed => {}
            Pre::Other => match key.code {
                KeyCode::Enter | KeyCode::Char('l') => {
                    let chosen = self.accounts.get(*cursor).cloned();
                    self.popup = None;
                    if let Some(login) = chosen
                        && self.account.as_deref() != Some(login.as_str())
                    {
                        self.account = Some(login);
                        self.prs.clear();
                        self.cache.clear();
                        self.detail = None;
                        self.dash = Dash { page: self.dash.page, show_drafts: self.dash.show_drafts, ..Dash::default() };
                        self.status = Status::Loading;
                        return Some(Effect::LoadPrs);
                    }
                }
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('a') => self.popup = None,
                _ => {}
            },
        }
        None
    }

    fn handle_dash(&mut self, key: KeyEvent) -> Option<Effect> {
        if self.dash.searching {
            return self.handle_dash_search(key);
        }
        let rows = self.dash_rows();
        match self.keys.motion(&key) {
            Pre::Motion(m) => {
                self.dash.cursor = apply(m, self.dash.cursor, rows.len(), self.dash.page);
                return None;
            }
            Pre::Consumed => return None,
            Pre::Other => {}
        }
        let cur = rows.get(self.dash.cursor).cloned();
        match key.code {
            KeyCode::Char('q') => return Some(Effect::Quit),
            KeyCode::Esc if !self.dash.query.is_empty() => {
                self.dash.query.clear();
                self.dash.cursor = first_pr_row(&self.dash_rows());
            }
            KeyCode::Char('/') => {
                self.dash.searching = true;
                self.dash.cursor = first_pr_row(&rows);
            }
            KeyCode::Char('r') => {
                self.status = Status::Loading;
                return Some(Effect::LoadPrs);
            }
            KeyCode::Char('a') => {
                if self.accounts.len() > 1 {
                    let cursor = self.accounts.iter().position(|a| Some(a) == self.account.as_ref()).unwrap_or(0);
                    self.popup = Some(Popup::Accounts { cursor });
                } else {
                    self.flash = Some("Only one account is logged in to gh. Add one with `gh auth login`.".into());
                }
            }
            KeyCode::Char('?') => self.popup = Some(Popup::Help),
            KeyCode::Char('d') => {
                let keep = self.selected_pr().map(Pr::key);
                self.dash.show_drafts = !self.dash.show_drafts;
                self.put_cursor_on(keep);
            }
            KeyCode::Char('}') => {
                if let Some(p) = (self.dash.cursor + 1..rows.len()).find(|&i| matches!(rows[i], DashRow::Group { .. })) {
                    self.dash.cursor = p;
                }
            }
            KeyCode::Char('{') => {
                if let Some(p) = (0..self.dash.cursor).rev().find(|&i| matches!(rows[i], DashRow::Group { .. })) {
                    self.dash.cursor = p;
                }
            }
            KeyCode::Char('o') => {
                if let Some(pr) = self.selected_pr() {
                    return Some(Effect::Open(pr.url.clone()));
                }
            }
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right | KeyCode::Char(' ') | KeyCode::Char('h') | KeyCode::Left => {
                return self.dash_fold_or_open(key.code, cur, &rows);
            }
            _ => {}
        }
        None
    }

    fn dash_fold_or_open(&mut self, code: KeyCode, cur: Option<DashRow>, rows: &[DashRow]) -> Option<Effect> {
        let header = (0..=self.dash.cursor).rev().find(|&i| matches!(rows.get(i), Some(DashRow::Group { .. })));
        let repo = header.and_then(|h| match &rows[h] {
            DashRow::Group { repo, .. } => Some(repo.clone()),
            _ => None,
        })?;
        if let (KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right, Some(DashRow::Pr { idx, .. })) = (code, &cur) {
            return self.open_detail(*idx);
        }
        let collapse = match code {
            KeyCode::Char('h') | KeyCode::Left => true,
            KeyCode::Char('l') | KeyCode::Right => false,
            _ => !self.dash.collapsed.contains(&repo),
        };
        if !self.dash.query.is_empty() {
            return None;
        }
        if collapse {
            self.dash.collapsed.insert(repo);
            self.dash.cursor = header.unwrap_or(0);
        } else {
            self.dash.collapsed.remove(&repo);
        }
        None
    }

    fn handle_dash_search(&mut self, key: KeyEvent) -> Option<Effect> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let rows = self.dash_rows();
        let step = |cursor: usize, down: bool| {
            let range: Box<dyn Iterator<Item = usize>> =
                if down { Box::new(cursor + 1..rows.len()) } else { Box::new((0..cursor).rev()) };
            range.into_iter().find(|&i| matches!(rows[i], DashRow::Pr { .. })).unwrap_or(cursor)
        };
        match key.code {
            KeyCode::Esc => {
                self.dash.searching = false;
                self.dash.query.clear();
                self.dash.cursor = first_pr_row(&self.dash_rows());
                return None;
            }
            KeyCode::Enter => {
                self.dash.searching = false;
                if let Some(DashRow::Pr { idx, .. }) = rows.get(self.dash.cursor) {
                    return self.open_detail(*idx);
                }
                return None;
            }
            KeyCode::Down => self.dash.cursor = step(self.dash.cursor, true),
            KeyCode::Up => self.dash.cursor = step(self.dash.cursor, false),
            KeyCode::Char('n') | KeyCode::Char('j') if ctrl => self.dash.cursor = step(self.dash.cursor, true),
            KeyCode::Char('p') | KeyCode::Char('k') if ctrl => self.dash.cursor = step(self.dash.cursor, false),
            _ => {
                if edit_line(&mut self.dash.query, &key) {
                    self.dash.cursor = first_pr_row(&self.dash_rows());
                }
            }
        }
        None
    }

    fn open_detail(&mut self, idx: usize) -> Option<Effect> {
        let pr = self.prs.get(idx)?.clone();
        let data = match self.cache.get(&pr.key()) {
            Some(d) => Load::Ready(d.clone()),
            None => Load::Loading,
        };
        let mut detail = Detail::new(pr.clone(), data);
        detail.refreshing = detail.ready().is_some();
        self.detail = Some(detail);
        Some(Effect::LoadDetail { repo: pr.repo, number: pr.number })
    }

    fn handle_detail(&mut self, key: KeyEvent) -> Option<Effect> {
        let d = self.detail.as_mut()?;
        if d.file_searching {
            return self.handle_file_search(key);
        }
        let in_threads = d.tab == Tab::Threads;
        match key.code {
            KeyCode::Char('q') => {
                self.detail = None;
                return None;
            }
            KeyCode::Tab | KeyCode::BackTab => {
                d.tab = if in_threads { Tab::Files } else { Tab::Threads };
                return None;
            }
            KeyCode::Char('r') => {
                d.refreshing = true;
                return Some(Effect::LoadDetail { repo: d.pr.repo.clone(), number: d.pr.number });
            }
            KeyCode::Char('?') => {
                self.popup = Some(Popup::Help);
                return None;
            }
            KeyCode::Char('t') => {
                d.show_resolved = !d.show_resolved;
                if let Some(data) = d.ready() {
                    d.th_cursor = d.th_cursor.min(visible_threads(&data, d.show_resolved).len().saturating_sub(1));
                    d.clamp_diff(&data);
                }
                return None;
            }
            KeyCode::Char('o') => {
                let thread_url = d.ready().and_then(|data| {
                    let t = *visible_threads(&data, d.show_resolved).get(d.th_cursor)?;
                    data.threads[t].url().map(str::to_string)
                });
                return Some(Effect::Open(match (in_threads, thread_url) {
                    (true, Some(url)) => url,
                    (false, _) => format!("{}/files", d.pr.url),
                    _ => d.pr.url.clone(),
                }));
            }
            _ => {}
        }
        let Some(data) = d.ready() else {
            if matches!(key.code, KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left) {
                self.detail = None;
            }
            return None;
        };
        if in_threads {
            return self.handle_threads(key, &data);
        }
        match d.focus {
            Focus::Tree => self.handle_tree(key, &data),
            Focus::Diff => self.handle_diff(key, &data),
        }
    }

    fn handle_threads(&mut self, key: KeyEvent, data: &PrDetail) -> Option<Effect> {
        let d = self.detail.as_mut()?;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let half = (d.th_page / 2).max(1) as u16;
        match key.code {
            KeyCode::Char('d') if ctrl => d.th_scroll = d.th_scroll.saturating_add(half),
            KeyCode::Char('u') if ctrl => d.th_scroll = d.th_scroll.saturating_sub(half),
            KeyCode::Char('e') if ctrl => d.th_scroll = d.th_scroll.saturating_add(1),
            KeyCode::Char('y') if ctrl => d.th_scroll = d.th_scroll.saturating_sub(1),
            _ => {}
        }
        if ctrl && matches!(key.code, KeyCode::Char('d' | 'u' | 'e' | 'y')) {
            return None;
        }
        let visible = visible_threads(data, d.show_resolved);
        match self.keys.motion(&key) {
            Pre::Motion(m) => {
                let next = apply(m, d.th_cursor, visible.len(), d.th_page);
                if next != d.th_cursor {
                    d.th_cursor = next;
                    d.th_scroll = 0;
                }
                return None;
            }
            Pre::Consumed => return None,
            Pre::Other => {}
        }
        match key.code {
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => {
                if let Some(&t) = visible.get(d.th_cursor) {
                    self.flash = d.jump_to_thread(data, t);
                }
            }
            KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left => self.detail = None,
            _ => {}
        }
        None
    }

    fn handle_tree(&mut self, key: KeyEvent, data: &PrDetail) -> Option<Effect> {
        let d = self.detail.as_mut()?;
        let items = d.tree_items(data);
        match self.keys.motion(&key) {
            Pre::Motion(m) => {
                d.tree_cursor = apply(m, d.tree_cursor, items.len(), d.tree_page);
                if let Some((TreeRow { kind: RowKind::File(f), .. }, _)) = items.get(d.tree_cursor)
                    && d.file != Some(*f)
                {
                    d.file = Some(*f);
                    d.diff_cursor = 0;
                    d.diff_offset = 0;
                }
                return None;
            }
            Pre::Consumed => return None,
            Pre::Other => {}
        }
        let cur = items.get(d.tree_cursor).map(|(r, _)| r.clone());
        match (key.code, cur.as_ref().map(|r| &r.kind)) {
            (KeyCode::Char('/'), _) => d.file_searching = true,
            (KeyCode::Esc, _) if !d.file_query.is_empty() => {
                d.file_query.clear();
                if let Some(f) = d.file {
                    d.select_file(data, f);
                }
            }
            (KeyCode::Esc, _) => self.detail = None,
            (KeyCode::Char(']'), _) => d.step_file(data, true),
            (KeyCode::Char('['), _) => d.step_file(data, false),
            (KeyCode::Char('n'), _) => self.flash = d.step_thread(data, true),
            (KeyCode::Char('N'), _) => self.flash = d.step_thread(data, false),
            (KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right, Some(RowKind::File(f))) => {
                let f = *f;
                d.select_file(data, f);
                d.focus = Focus::Diff;
            }
            (KeyCode::Enter | KeyCode::Char(' '), Some(RowKind::Dir(dir))) => {
                if !d.collapsed.remove(dir) {
                    d.collapsed.insert(dir.clone());
                }
            }
            (KeyCode::Char('l') | KeyCode::Right, Some(RowKind::Dir(dir))) => {
                d.collapsed.remove(dir);
            }
            (KeyCode::Char('h') | KeyCode::Left, Some(kind)) => {
                let row = cur.as_ref().expect("row exists");
                match kind {
                    RowKind::Dir(dir) if !row.collapsed => {
                        d.collapsed.insert(dir.clone());
                    }
                    _ => {
                        if let Some(parent) = (0..d.tree_cursor).rev().find(|&i| items[i].0.depth < row.depth) {
                            d.tree_cursor = parent;
                        }
                    }
                }
            }
            _ => {}
        }
        None
    }

    fn handle_file_search(&mut self, key: KeyEvent) -> Option<Effect> {
        let d = self.detail.as_mut()?;
        let data = d.ready()?;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let len = d.tree_items(&data).len();
        match key.code {
            KeyCode::Esc => {
                d.file_searching = false;
                d.file_query.clear();
                if let Some(f) = d.file {
                    d.select_file(&data, f);
                }
            }
            KeyCode::Enter => {
                let picked = d.tree_items(&data).get(d.tree_cursor).and_then(|(r, _)| match r.kind {
                    RowKind::File(f) => Some(f),
                    RowKind::Dir(_) => None,
                });
                d.file_searching = false;
                d.file_query.clear();
                if let Some(f) = picked.or(d.file) {
                    d.select_file(&data, f);
                    d.focus = Focus::Diff;
                }
            }
            KeyCode::Down => d.tree_cursor = apply(Motion::Down(1), d.tree_cursor, len, 0),
            KeyCode::Up => d.tree_cursor = apply(Motion::Up(1), d.tree_cursor, len, 0),
            KeyCode::Char('n' | 'j') if ctrl => d.tree_cursor = apply(Motion::Down(1), d.tree_cursor, len, 0),
            KeyCode::Char('p' | 'k') if ctrl => d.tree_cursor = apply(Motion::Up(1), d.tree_cursor, len, 0),
            _ => {
                if edit_line(&mut d.file_query, &key) {
                    d.tree_cursor = 0;
                    d.tree_offset = 0;
                }
            }
        }
        if let Some((TreeRow { kind: RowKind::File(f), .. }, _)) = d.tree_items(&data).get(d.tree_cursor)
            && d.file_searching
        {
            d.file = Some(*f);
            d.diff_cursor = 0;
            d.diff_offset = 0;
        }
        None
    }

    fn handle_diff(&mut self, key: KeyEvent, data: &PrDetail) -> Option<Effect> {
        let d = self.detail.as_mut()?;
        let rows = d.diff_rows(data);
        match self.keys.motion(&key) {
            Pre::Motion(m) => {
                d.diff_cursor = apply(m, d.diff_cursor, rows.len(), d.diff_page);
                return None;
            }
            Pre::Consumed => return None,
            Pre::Other => {}
        }
        let file = d.file.map(|f| &data.files[f]);
        let is_hunk = |r: &DiffRow| matches!(r, DiffRow::Line(i) if file.is_some_and(|f| f.lines[*i].kind == LineKind::Hunk));
        match key.code {
            KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left => d.focus = Focus::Tree,
            KeyCode::Char('}') => {
                if let Some(p) = (d.diff_cursor + 1..rows.len()).find(|&i| is_hunk(&rows[i])) {
                    d.diff_cursor = p;
                }
            }
            KeyCode::Char('{') => {
                if let Some(p) = (0..d.diff_cursor).rev().find(|&i| is_hunk(&rows[i])) {
                    d.diff_cursor = p;
                }
            }
            KeyCode::Char(']') => d.step_file(data, true),
            KeyCode::Char('[') => d.step_file(data, false),
            KeyCode::Char('n') => self.flash = d.step_thread(data, true),
            KeyCode::Char('N') => self.flash = d.step_thread(data, false),
            KeyCode::Char('/') => {
                d.focus = Focus::Tree;
                d.file_searching = true;
            }
            _ => {}
        }
        None
    }
}

fn first_pr_row(rows: &[DashRow]) -> usize {
    rows.iter().position(|r| matches!(r, DashRow::Pr { .. })).unwrap_or(0)
}

/// Readline-style editing for a one-line prompt. Returns true when the text changed.
fn edit_line(text: &mut String, key: &KeyEvent) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Backspace => text.pop().is_some(),
        KeyCode::Char('u') if ctrl => {
            text.clear();
            true
        }
        KeyCode::Char('w') if ctrl => {
            let trimmed = text.trim_end().len();
            let cut = text[..trimmed].rfind(' ').map_or(0, |i| i + 1);
            text.truncate(cut);
            true
        }
        KeyCode::Char(c) if !ctrl => {
            text.push(c);
            true
        }
        _ => false,
    }
}

/// Word-wraps by display width; words longer than a line are split.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    for para in text.replace("\r\n", "\n").split('\n') {
        let mut line = String::new();
        let mut w = 0;
        for word in para.split(' ') {
            let ww: usize = word.chars().map(|c| c.width().unwrap_or(0)).sum();
            if w > 0 && w + 1 + ww > width {
                out.push(std::mem::take(&mut line));
                w = 0;
            }
            if w > 0 {
                line.push(' ');
                w += 1;
            }
            for c in word.chars() {
                let cw = c.width().unwrap_or(0);
                if w + cw > width && w > 0 {
                    out.push(std::mem::take(&mut line));
                    w = 0;
                }
                line.push(if c == '\t' { ' ' } else { c });
                w += cw;
            }
        }
        out.push(line);
    }
    while out.last().is_some_and(|l| l.is_empty()) && out.len() > 1 {
        out.pop();
    }
    out
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;
