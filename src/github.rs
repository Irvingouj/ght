use std::collections::HashSet;
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::diff::parse_patch;
use crate::model::{Check, CheckState, Checks, Comment, FileDiff, FileStatus, Pr, PrDetail, Review, ReviewState, Reviewer, Side, Thread};

/// Overridable so the end-to-end tests can point ght at a fake GitHub on localhost.
fn api_base() -> String {
    std::env::var("GHT_API_URL").unwrap_or_else(|_| "https://api.github.com".into())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub login: String,
    pub active: bool,
}

pub fn accounts() -> Result<Vec<Account>> {
    let out = gh(&["auth", "status", "--json", "hosts"])?;
    parse_accounts(&out)
}

pub fn token(login: &str) -> Result<String> {
    Ok(gh(&["auth", "token", "--user", login])?.trim().to_string())
}

/// GitHub repos (lowercase `owner/name`) that the current directory's git remotes point at.
/// Empty outside a git checkout.
pub fn local_repos() -> HashSet<String> {
    let Ok(out) = Command::new("git").args(["remote", "-v"]).output() else { return HashSet::new() };
    String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| repo_from_remote(l.split_whitespace().nth(1)?)).collect()
}

pub fn repo_from_remote(url: &str) -> Option<String> {
    let (_, path) = url.split_once("github.com")?;
    let path = path.trim_start_matches([':', '/']).trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut parts = path.split('/');
    let (owner, name) = (parts.next()?, parts.next()?);
    (!owner.is_empty() && !name.is_empty()).then(|| format!("{owner}/{name}").to_lowercase())
}

fn gh(args: &[&str]) -> Result<String> {
    let out = Command::new("gh").args(args).output().context("could not run `gh`; is the GitHub CLI installed?")?;
    if !out.status.success() {
        bail!("gh {} failed: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8(out.stdout)?)
}

fn parse_accounts(json: &str) -> Result<Vec<Account>> {
    #[derive(Deserialize)]
    struct Entry {
        login: String,
        active: bool,
    }
    let v: Value = serde_json::from_str(json)?;
    let entries: Vec<Entry> = serde_json::from_value(v["hosts"]["github.com"].clone()).context("no github.com accounts in `gh auth status`")?;
    Ok(entries.into_iter().map(|e| Account { login: e.login, active: e.active }).collect())
}

fn graphql(token: &str, query: &str, variables: Value) -> Result<Value> {
    let resp: Value = ureq::post(&format!("{}/graphql", api_base()))
        .set("Authorization", &format!("bearer {token}"))
        .set("User-Agent", "ght")
        .send_json(json!({ "query": query, "variables": variables }))
        .map_err(|e| anyhow!("GitHub request failed: {e}"))?
        .into_json()?;
    if let Some(errors) = resp.get("errors") {
        bail!("GitHub GraphQL error: {errors}");
    }
    Ok(resp["data"].clone())
}

const SEARCH_QUERY: &str = r#"
query($q: String!, $after: String) {
  search(query: $q, type: ISSUE, first: 50, after: $after) {
    pageInfo { hasNextPage endCursor }
    nodes {
      ... on PullRequest {
        number title url isDraft headRefName baseRefName updatedAt additions deletions reviewDecision
        repository { nameWithOwner }
        commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }
        reviewThreads(first: 100) { totalCount nodes { isResolved } }
      }
    }
  }
}"#;

pub fn fetch_prs(token: &str) -> Result<Vec<Pr>> {
    let mut prs = Vec::new();
    let mut after = Value::Null;
    loop {
        let data = graphql(token, SEARCH_QUERY, json!({ "q": "is:pr is:open author:@me archived:false", "after": after }))?;
        let (page, next) = parse_search(&data)?;
        prs.extend(page);
        match next {
            Some(cursor) if prs.len() < 500 => after = Value::String(cursor),
            _ => return Ok(prs),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
}

fn parse_search(data: &Value) -> Result<(Vec<Pr>, Option<String>)> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Node {
        number: u64,
        title: String,
        url: String,
        is_draft: bool,
        head_ref_name: String,
        base_ref_name: String,
        updated_at: DateTime<Utc>,
        additions: u64,
        deletions: u64,
        review_decision: Option<String>,
        repository: Repo,
        commits: Commits,
        review_threads: Threads,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Repo {
        name_with_owner: String,
    }
    #[derive(Deserialize)]
    struct Commits {
        nodes: Vec<CommitNode>,
    }
    #[derive(Deserialize)]
    struct CommitNode {
        commit: Commit,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Commit {
        status_check_rollup: Option<Rollup>,
    }
    #[derive(Deserialize)]
    struct Rollup {
        state: String,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Threads {
        total_count: usize,
        nodes: Vec<ThreadNode>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ThreadNode {
        is_resolved: bool,
    }

    let search = &data["search"];
    let info: PageInfo = serde_json::from_value(search["pageInfo"].clone())?;
    let mut prs = Vec::new();
    for raw in search["nodes"].as_array().context("search.nodes missing")? {
        if raw.as_object().is_none_or(|o| o.is_empty()) {
            continue;
        }
        let n: Node = serde_json::from_value(raw.clone())?;
        let checks = n.commits.nodes.first().and_then(|c| c.commit.status_check_rollup.as_ref()).map(|r| match r.state.as_str() {
            "SUCCESS" => Checks::Passing,
            "FAILURE" | "ERROR" => Checks::Failing,
            _ => Checks::Pending,
        });
        let review = n.review_decision.as_deref().and_then(|d| match d {
            "APPROVED" => Some(Review::Approved),
            "CHANGES_REQUESTED" => Some(Review::ChangesRequested),
            "REVIEW_REQUIRED" => Some(Review::Required),
            _ => None,
        });
        prs.push(Pr {
            repo: n.repository.name_with_owner,
            number: n.number,
            title: n.title,
            url: n.url,
            is_draft: n.is_draft,
            head: n.head_ref_name,
            base: n.base_ref_name,
            updated_at: n.updated_at,
            additions: n.additions,
            deletions: n.deletions,
            review,
            checks,
            threads_total: n.review_threads.total_count,
            threads_unresolved: n.review_threads.nodes.iter().filter(|t| !t.is_resolved).count(),
        });
    }
    let next = if info.has_next_page { info.end_cursor } else { None };
    Ok((prs, next))
}

const THREADS_QUERY: &str = r#"
query($owner: String!, $name: String!, $number: Int!, $after: String) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      reviewThreads(first: 100, after: $after) {
        pageInfo { hasNextPage endCursor }
        nodes {
          isResolved isOutdated path line diffSide
          comments(first: 100) { nodes { author { login } body createdAt url diffHunk } }
        }
      }
    }
  }
}"#;

const STATUS_QUERY: &str = r#"
query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      latestReviews(first: 100) { nodes { author { login } state submittedAt url } }
      reviewRequests(first: 100) { nodes { requestedReviewer { __typename ... on User { login } ... on Team { name } } } }
      commits(last: 1) { nodes { commit { statusCheckRollup { contexts(first: 100) { totalCount nodes {
        __typename
        ... on CheckRun { name status conclusion detailsUrl startedAt completedAt isRequired(pullRequestNumber: $number) checkSuite { workflowRun { workflow { name } } } }
        ... on StatusContext { context state targetUrl createdAt isRequired(pullRequestNumber: $number) }
      } } } } } }
    }
  }
}"#;

/// Threads, files and reviews/checks are three separate requests, so fetch them side by side.
pub fn fetch_detail(token: &str, repo: &str, number: u64) -> Result<PrDetail> {
    let (threads, files, status) = std::thread::scope(|s| {
        let threads = s.spawn(|| fetch_threads(token, repo, number));
        let status = s.spawn(|| fetch_status(token, repo, number));
        let files = fetch_files(token, repo, number);
        (joined(threads.join()), files, joined(status.join()))
    });
    let (reviewers, checks, checks_total) = status?;
    Ok(PrDetail { threads: threads?, files: files?, reviewers, checks, checks_total })
}

fn joined<T>(r: std::thread::Result<Result<T>>) -> Result<T> {
    r.unwrap_or_else(|_| Err(anyhow!("fetch panicked")))
}

fn fetch_status(token: &str, repo: &str, number: u64) -> Result<(Vec<Reviewer>, Vec<Check>, usize)> {
    let (owner, name) = repo.split_once('/').context("repo must be owner/name")?;
    parse_status(&graphql(token, STATUS_QUERY, json!({ "owner": owner, "name": name, "number": number }))?)
}

fn parse_status(data: &Value) -> Result<(Vec<Reviewer>, Vec<Check>, usize)> {
    let pr = &data["repository"]["pullRequest"];
    if pr.is_null() {
        bail!("pull request not found");
    }
    let mut reviewers: Vec<Reviewer> = Vec::new();
    for r in pr["latestReviews"]["nodes"].as_array().into_iter().flatten() {
        let state = match r["state"].as_str() {
            Some("APPROVED") => ReviewState::Approved,
            Some("CHANGES_REQUESTED") => ReviewState::ChangesRequested,
            Some("COMMENTED") => ReviewState::Commented,
            Some("DISMISSED") => ReviewState::Dismissed,
            _ => continue,
        };
        reviewers.push(Reviewer {
            login: r["author"]["login"].as_str().unwrap_or("ghost").to_string(),
            state: Some(state),
            requested: false,
            at: r["submittedAt"].as_str().and_then(|t| t.parse().ok()),
            url: r["url"].as_str().map(str::to_string),
        });
    }
    for r in pr["reviewRequests"]["nodes"].as_array().into_iter().flatten() {
        let who = &r["requestedReviewer"];
        let Some(login) = who["login"].as_str().or(who["name"].as_str()) else { continue };
        match reviewers.iter_mut().find(|x| x.login == login) {
            Some(x) => x.requested = true,
            None => reviewers.push(Reviewer { login: login.to_string(), state: None, requested: true, at: None, url: None }),
        }
    }

    let contexts = &pr["commits"]["nodes"][0]["commit"]["statusCheckRollup"]["contexts"];
    let str_of = |v: &Value| v.as_str().map(str::to_string);
    let time_of = |v: &Value| v.as_str().and_then(|t| t.parse().ok());
    let checks = contexts["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| !c.is_null())
        .map(|c| {
            let run = c["__typename"] == "CheckRun";
            let state = if run {
                match (c["status"].as_str(), c["conclusion"].as_str()) {
                    (Some(s), _) if s != "COMPLETED" => CheckState::Pending,
                    (_, Some("SUCCESS")) => CheckState::Passing,
                    (_, Some("NEUTRAL" | "SKIPPED" | "STALE")) => CheckState::Skipped,
                    _ => CheckState::Failing,
                }
            } else {
                match c["state"].as_str() {
                    Some("SUCCESS") => CheckState::Passing,
                    Some("PENDING" | "EXPECTED") => CheckState::Pending,
                    _ => CheckState::Failing,
                }
            };
            Check {
                name: str_of(if run { &c["name"] } else { &c["context"] }).unwrap_or_default(),
                workflow: str_of(&c["checkSuite"]["workflowRun"]["workflow"]["name"]).map(|w| w.trim().to_string()),
                state,
                required: c["isRequired"].as_bool().unwrap_or(false),
                url: str_of(if run { &c["detailsUrl"] } else { &c["targetUrl"] }),
                started: time_of(if run { &c["startedAt"] } else { &c["createdAt"] }),
                completed: time_of(&c["completedAt"]),
            }
        })
        .collect::<Vec<_>>();
    let total = contexts["totalCount"].as_u64().map_or(checks.len(), |n| n as usize);
    Ok((reviewers, checks, total))
}

fn fetch_threads(token: &str, repo: &str, number: u64) -> Result<Vec<Thread>> {
    let (owner, name) = repo.split_once('/').context("repo must be owner/name")?;
    let mut threads = Vec::new();
    let mut after = Value::Null;
    loop {
        let data = graphql(token, THREADS_QUERY, json!({ "owner": owner, "name": name, "number": number, "after": after }))?;
        let (page, next) = parse_threads(&data)?;
        threads.extend(page);
        match next {
            Some(cursor) => after = Value::String(cursor),
            None => return Ok(threads),
        }
    }
}

fn fetch_files(token: &str, repo: &str, number: u64) -> Result<Vec<FileDiff>> {
    let mut files = Vec::new();
    for page in 1..=30 {
        let url = format!("{}/repos/{repo}/pulls/{number}/files?per_page=100&page={page}", api_base());
        let body: Value = ureq::get(&url)
            .set("Authorization", &format!("bearer {token}"))
            .set("User-Agent", "ght")
            .set("Accept", "application/vnd.github+json")
            .call()
            .map_err(|e| anyhow!("GitHub request failed: {e}"))?
            .into_json()?;
        let batch = parse_files(&body)?;
        let done = batch.len() < 100;
        files.extend(batch);
        if done {
            break;
        }
    }
    Ok(files)
}

fn parse_threads(data: &Value) -> Result<(Vec<Thread>, Option<String>)> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Node {
        is_resolved: bool,
        is_outdated: bool,
        path: String,
        line: Option<u32>,
        diff_side: String,
        comments: Comments,
    }
    #[derive(Deserialize)]
    struct Comments {
        nodes: Vec<CommentNode>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct CommentNode {
        author: Option<Author>,
        body: String,
        created_at: DateTime<Utc>,
        url: String,
        diff_hunk: String,
    }
    #[derive(Deserialize)]
    struct Author {
        login: String,
    }

    let rt = &data["repository"]["pullRequest"]["reviewThreads"];
    if rt.is_null() {
        bail!("pull request not found");
    }
    let info: PageInfo = serde_json::from_value(rt["pageInfo"].clone())?;
    let nodes: Vec<Node> = serde_json::from_value(rt["nodes"].clone())?;
    let threads = nodes
        .into_iter()
        .map(|n| Thread {
            path: n.path,
            line: n.line,
            side: if n.diff_side == "LEFT" { Side::Left } else { Side::Right },
            is_resolved: n.is_resolved,
            is_outdated: n.is_outdated,
            diff_hunk: n.comments.nodes.first().map(|c| c.diff_hunk.clone()).unwrap_or_default(),
            comments: n
                .comments
                .nodes
                .into_iter()
                .map(|c| Comment {
                    author: c.author.map(|a| a.login).unwrap_or_else(|| "ghost".into()),
                    body: unescape_html(&c.body),
                    created_at: c.created_at,
                    url: c.url,
                })
                .collect(),
        })
        .collect();
    let next = if info.has_next_page { info.end_cursor } else { None };
    Ok((threads, next))
}

/// GitHub bots often post bodies with HTML entities that only a browser would decode.
fn unescape_html(s: &str) -> String {
    s.replace("&#39;", "'").replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

fn parse_files(body: &Value) -> Result<Vec<FileDiff>> {
    #[derive(Deserialize)]
    struct Raw {
        filename: String,
        previous_filename: Option<String>,
        status: String,
        additions: u64,
        deletions: u64,
        patch: Option<String>,
    }
    let raws: Vec<Raw> = serde_json::from_value(body.clone()).context("unexpected files response")?;
    Ok(raws
        .into_iter()
        .map(|r| FileDiff {
            status: match r.status.as_str() {
                "added" => FileStatus::Added,
                "removed" => FileStatus::Removed,
                "modified" | "changed" => FileStatus::Modified,
                "renamed" | "copied" => FileStatus::Renamed,
                _ => FileStatus::Other,
            },
            path: r.filename,
            previous_path: r.previous_filename,
            additions: r.additions,
            deletions: r.deletions,
            has_patch: r.patch.is_some(),
            lines: r.patch.as_deref().map(parse_patch).unwrap_or_default(),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_urls_to_repo() {
        for url in [
            "https://github.com/Devolutions/IronRDP.git",
            "https://github.com/Devolutions/IronRDP",
            "git@github.com:Devolutions/IronRDP.git",
            "ssh://git@github.com/Devolutions/IronRDP/",
        ] {
            assert_eq!(repo_from_remote(url).as_deref(), Some("devolutions/ironrdp"), "{url}");
        }
        assert_eq!(repo_from_remote("https://gitlab.com/a/b"), None);
        assert_eq!(repo_from_remote("https://github.com/only"), None);
    }

    #[test]
    fn accounts_from_gh_status() {
        let json = r#"{"hosts":{"github.com":[{"state":"success","active":true,"login":"work"},{"state":"success","active":false,"login":"home"}]}}"#;
        assert_eq!(
            parse_accounts(json).unwrap(),
            vec![Account { login: "work".into(), active: true }, Account { login: "home".into(), active: false }]
        );
    }

    #[test]
    fn search_page_maps_status_fields() {
        let data = json!({ "search": {
            "pageInfo": { "hasNextPage": true, "endCursor": "abc" },
            "nodes": [
                {
                    "number": 7, "title": "Fix it", "url": "https://github.com/o/r/pull/7", "isDraft": true,
                    "headRefName": "fix", "baseRefName": "master", "updatedAt": "2026-10-01T00:00:00Z",
                    "additions": 3, "deletions": 1, "reviewDecision": "CHANGES_REQUESTED",
                    "repository": { "nameWithOwner": "o/r" },
                    "commits": { "nodes": [ { "commit": { "statusCheckRollup": { "state": "FAILURE" } } } ] },
                    "reviewThreads": { "totalCount": 3, "nodes": [ {"isResolved": true}, {"isResolved": false}, {"isResolved": false} ] }
                },
                {}
            ]
        }});
        let (prs, next) = parse_search(&data).unwrap();
        assert_eq!(next.as_deref(), Some("abc"));
        assert_eq!(prs.len(), 1);
        let pr = &prs[0];
        assert_eq!((pr.repo.as_str(), pr.number, pr.is_draft), ("o/r", 7, true));
        assert_eq!(pr.checks, Some(Checks::Failing));
        assert_eq!(pr.review, Some(Review::ChangesRequested));
        assert_eq!((pr.threads_unresolved, pr.threads_total), (2, 3));
    }

    #[test]
    fn threads_and_files_parse() {
        let data = json!({ "repository": { "pullRequest": { "reviewThreads": {
            "pageInfo": { "hasNextPage": false, "endCursor": null },
            "nodes": [ {
                "isResolved": false, "isOutdated": false, "path": "src/a.rs", "line": 12, "diffSide": "LEFT",
                "comments": { "nodes": [ { "author": null, "body": "why?", "createdAt": "2026-10-01T00:00:00Z", "url": "u", "diffHunk": "@@ -1 +1 @@" } ] }
            } ]
        }}}});
        let (threads, next) = parse_threads(&data).unwrap();
        assert!(next.is_none());
        assert_eq!(threads[0].side, Side::Left);
        assert_eq!(threads[0].comments[0].author, "ghost");
        assert_eq!(threads[0].location(), "src/a.rs:12");
        assert_eq!(unescape_html("it&#39;s &lt;b&gt; &amp;amp;"), "it's <b> &amp;");

        let files = parse_files(&json!([
            { "filename": "a.bin", "status": "added", "additions": 0, "deletions": 0 },
            { "filename": "b.rs", "previous_filename": "old.rs", "status": "renamed", "additions": 1, "deletions": 0, "patch": "@@ -1 +1,2 @@\n x\n+y" }
        ]))
        .unwrap();
        assert!(!files[0].has_patch);
        assert_eq!(files[1].status, FileStatus::Renamed);
        assert_eq!(files[1].lines.len(), 3);
    }

    #[test]
    fn status_maps_reviewers_and_checks() {
        let data = json!({ "repository": { "pullRequest": {
            "latestReviews": { "nodes": [
                { "author": { "login": "ann" }, "state": "APPROVED", "submittedAt": "2026-10-01T00:00:00Z", "url": "r1" },
                { "author": { "login": "bob" }, "state": "CHANGES_REQUESTED", "submittedAt": "2026-10-01T00:00:00Z", "url": "r2" },
                { "author": { "login": "me" }, "state": "PENDING", "submittedAt": null, "url": "r3" }
            ] },
            "reviewRequests": { "nodes": [
                { "requestedReviewer": { "__typename": "User", "login": "bob" } },
                { "requestedReviewer": { "__typename": "Team", "name": "core" } }
            ] },
            "commits": { "nodes": [ { "commit": { "statusCheckRollup": { "contexts": { "totalCount": 120, "nodes": [
                { "__typename": "CheckRun", "name": "test", "status": "COMPLETED", "conclusion": "FAILURE", "detailsUrl": "d1",
                  "startedAt": "2026-10-01T00:00:00Z", "completedAt": "2026-10-01T00:07:00Z", "isRequired": true,
                  "checkSuite": { "workflowRun": { "workflow": { "name": " CI " } } } },
                { "__typename": "CheckRun", "name": "lint", "status": "IN_PROGRESS", "conclusion": null, "detailsUrl": "d2",
                  "startedAt": null, "completedAt": null, "isRequired": false, "checkSuite": { "workflowRun": null } },
                { "__typename": "CheckRun", "name": "docs", "status": "COMPLETED", "conclusion": "SKIPPED", "detailsUrl": "d3", "isRequired": false },
                { "__typename": "StatusContext", "context": "ci/legacy", "state": "SUCCESS", "targetUrl": "t1", "createdAt": "2026-10-01T00:00:00Z", "isRequired": false },
                null
            ] } } } } ] }
        }}});
        let (reviewers, checks, total) = parse_status(&data).unwrap();
        let summary: Vec<_> = reviewers.iter().map(|r| (r.login.as_str(), r.state, r.requested)).collect();
        assert_eq!(
            summary,
            vec![("ann", Some(ReviewState::Approved), false), ("bob", Some(ReviewState::ChangesRequested), true), ("core", None, true)],
            "a pending (own draft) review is left out; a re-request marks the existing reviewer"
        );
        let summary: Vec<_> = checks.iter().map(|c| (c.name.as_str(), c.workflow.as_deref(), c.state, c.required)).collect();
        assert_eq!(
            summary,
            vec![
                ("test", Some("CI"), CheckState::Failing, true),
                ("lint", None, CheckState::Pending, false),
                ("docs", None, CheckState::Skipped, false),
                ("ci/legacy", None, CheckState::Passing, false),
            ]
        );
        assert_eq!(checks[3].url.as_deref(), Some("t1"));
        assert_eq!(total, 120);
    }
}
