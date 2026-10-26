use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Checks {
    Passing,
    Failing,
    Pending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Review {
    Approved,
    ChangesRequested,
    Required,
}

#[derive(Debug, Clone)]
pub struct Pr {
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub is_draft: bool,
    pub head: String,
    pub base: String,
    pub updated_at: DateTime<Utc>,
    pub additions: u64,
    pub deletions: u64,
    pub review: Option<Review>,
    pub checks: Option<Checks>,
    pub threads_total: usize,
    pub threads_unresolved: usize,
}

impl Pr {
    pub fn key(&self) -> (String, u64) {
        (self.repo.clone(), self.number)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

#[derive(Debug, Clone)]
pub struct Comment {
    pub author: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct Thread {
    pub path: String,
    pub line: Option<u32>,
    pub side: Side,
    pub is_resolved: bool,
    pub is_outdated: bool,
    pub diff_hunk: String,
    pub comments: Vec<Comment>,
}

impl Thread {
    pub fn url(&self) -> Option<&str> {
        self.comments.first().map(|c| c.url.as_str())
    }

    pub fn location(&self) -> String {
        match self.line {
            Some(line) => format!("{}:{line}", self.path),
            None => self.path.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Modified,
    Removed,
    Renamed,
    Other,
}

impl FileStatus {
    pub fn letter(self) -> char {
        match self {
            FileStatus::Added => 'A',
            FileStatus::Modified => 'M',
            FileStatus::Removed => 'D',
            FileStatus::Renamed => 'R',
            FileStatus::Other => '?',
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Hunk,
    Add,
    Del,
    Ctx,
    Meta,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old: Option<u32>,
    pub new: Option<u32>,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct FileDiff {
    pub path: String,
    pub previous_path: Option<String>,
    pub status: FileStatus,
    pub additions: u64,
    pub deletions: u64,
    /// GitHub leaves the patch out for binary files and very large diffs.
    pub has_patch: bool,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, Default)]
pub struct PrDetail {
    pub threads: Vec<Thread>,
    pub files: Vec<FileDiff>,
}

pub fn age(now: DateTime<Utc>, then: DateTime<Utc>) -> String {
    let secs = (now - then).num_seconds().max(0);
    let (n, unit) = match secs {
        s if s < 60 => return "now".into(),
        s if s < 3600 => (s / 60, "m"),
        s if s < 86_400 => (s / 3600, "h"),
        s if s < 7 * 86_400 => (s / 86_400, "d"),
        s if s < 30 * 86_400 => (s / (7 * 86_400), "w"),
        s if s < 365 * 86_400 => (s / (30 * 86_400), "mo"),
        s => (s / (365 * 86_400), "y"),
    };
    format!("{n}{unit}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn age_buckets() {
        let now = Utc::now();
        assert_eq!(age(now, now - Duration::seconds(10)), "now");
        assert_eq!(age(now, now - Duration::minutes(5)), "5m");
        assert_eq!(age(now, now - Duration::hours(3)), "3h");
        assert_eq!(age(now, now - Duration::days(2)), "2d");
        assert_eq!(age(now, now - Duration::days(15)), "2w");
        assert_eq!(age(now, now - Duration::days(70)), "2mo");
        assert_eq!(age(now, now - Duration::days(800)), "2y");
        assert_eq!(age(now, now + Duration::days(1)), "now");
    }
}
