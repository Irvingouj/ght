use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::diff::parse_patch;
use crate::model::{Checks, Comment, FileDiff, FileStatus, Pr, PrDetail, Review, Side, Thread};

const GRAPHQL: &str = "https://api.github.com/graphql";

#[derive(Debug, Clone, PartialEq, Eq)]
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
    let resp: Value = ureq::post(GRAPHQL)
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

pub fn fetch_detail(token: &str, repo: &str, number: u64) -> Result<PrDetail> {
    let (owner, name) = repo.split_once('/').context("repo must be owner/name")?;
    let mut threads = Vec::new();
    let mut after = Value::Null;
    loop {
        let data = graphql(token, THREADS_QUERY, json!({ "owner": owner, "name": name, "number": number, "after": after }))?;
        let (page, next) = parse_threads(&data)?;
        threads.extend(page);
        match next {
            Some(cursor) => after = Value::String(cursor),
            None => break,
        }
    }

    let mut files = Vec::new();
    for page in 1..=30 {
        let url = format!("https://api.github.com/repos/{repo}/pulls/{number}/files?per_page=100&page={page}");
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
    Ok(PrDetail { threads, files })
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
}
