//! A fake GitHub API for end-to-end tests, so assertions run against fixed data.
//!
//! Usage: fake_github <fixture.json> [port] [delay-ms]
//! Listens on 127.0.0.1 only. The fixture describes PRs, threads and files in a
//! compact form; this server expands them into the GraphQL and REST shapes ght reads.
//! Reviews and checks (`status`) are already stored in GitHub's GraphQL shape.
//! The delay keeps "loading…" states on screen long enough to assert on.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: fake_github <fixture.json> [port] [delay-ms]");
    let port: u16 = args.next().map_or(18765, |p| p.parse().expect("port"));
    let delay = Duration::from_millis(args.next().map_or(400, |d| d.parse().expect("delay")));
    let fixture: Arc<Value> = Arc::new(serde_json::from_str(&std::fs::read_to_string(path).expect("read fixture")).expect("fixture is JSON"));
    let listener = TcpListener::bind(("127.0.0.1", port)).expect("bind");
    eprintln!("fake github on 127.0.0.1:{port}");
    for stream in listener.incoming().flatten() {
        let fixture = fixture.clone();
        std::thread::spawn(move || {
            if let Err(e) = serve(stream, &fixture, delay) {
                eprintln!("request failed: {e}");
            }
        });
    }
}

fn serve(stream: TcpStream, fixture: &Value, delay: Duration) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let (mut length, mut user) = (0usize, String::new());
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        let (name, value) = line.split_once(':').unwrap_or((line, ""));
        match name.to_ascii_lowercase().as_str() {
            "content-length" => length = value.trim().parse().unwrap_or(0),
            "authorization" => user = value.trim().trim_start_matches("bearer token-").to_string(),
            _ => {}
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    let mut parts = request_line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    eprintln!("{method} {target} as {user}");
    std::thread::sleep(delay);
    let reply = route(fixture, method, target, &user, &body);
    let text = reply.to_string();
    let mut stream = stream;
    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}", text.len())?;
    stream.flush()
}

fn route(fixture: &Value, method: &str, target: &str, user: &str, body: &[u8]) -> Value {
    if method == "POST" && target == "/graphql" {
        let req: Value = serde_json::from_slice(body).unwrap_or_default();
        let query = req["query"].as_str().unwrap_or("");
        let vars = &req["variables"];
        if query.contains("search(") {
            let nodes: Vec<Value> = fixture["prs"][user].as_array().cloned().unwrap_or_default().iter().map(|pr| search_node(fixture, pr)).collect();
            return json!({ "data": { "search": { "pageInfo": { "hasNextPage": false, "endCursor": null }, "nodes": nodes } } });
        }
        let key = format!("{}/{}#{}", vars["owner"].as_str().unwrap_or(""), vars["name"].as_str().unwrap_or(""), vars["number"]);
        if query.contains("latestReviews") {
            let s = &fixture["status"][&key];
            let checks = s["checks"].as_array().cloned().unwrap_or_default();
            let total = s["total"].as_u64().unwrap_or(checks.len() as u64);
            return json!({ "data": { "repository": { "pullRequest": {
                "latestReviews": { "nodes": s["reviews"].as_array().cloned().unwrap_or_default() },
                "reviewRequests": { "nodes": s["requests"].as_array().cloned().unwrap_or_default() },
                "commits": { "nodes": [ { "commit": { "statusCheckRollup": { "contexts": { "totalCount": total, "nodes": checks } } } } ] },
            } } } });
        }
        let nodes: Vec<Value> = threads(fixture, &key).iter().enumerate().map(|(i, t)| thread_node(&key, i, t)).collect();
        return json!({ "data": { "repository": { "pullRequest": { "reviewThreads": {
            "pageInfo": { "hasNextPage": false, "endCursor": null }, "nodes": nodes
        } } } } });
    }
    // GET /repos/{owner}/{name}/pulls/{number}/files?per_page=100&page=N
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let segs: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    if let ["repos", owner, name, "pulls", number, "files"] = segs.as_slice() {
        if !query.split('&').any(|kv| kv == "page=1") {
            return json!([]);
        }
        let key = format!("{owner}/{name}#{number}");
        return fixture["files"].get(&key).cloned().unwrap_or_else(|| {
            json!([{ "filename": "CHANGELOG.md", "status": "modified", "additions": 1, "deletions": 1, "patch": "@@ -1 +1 @@\n-old entry\n+new entry" }])
        });
    }
    json!({ "message": format!("fake github: no route for {method} {target}") })
}

fn threads(fixture: &Value, key: &str) -> Vec<Value> {
    fixture["threads"].get(key).and_then(Value::as_array).cloned().unwrap_or_default()
}

fn search_node(fixture: &Value, pr: &Value) -> Value {
    let key = format!("{}#{}", pr["repo"].as_str().unwrap_or(""), pr["number"]);
    let summary: Vec<Value> = threads(fixture, &key).iter().map(|t| json!({ "isResolved": t["resolved"] })).collect();
    let rollup = match pr["ci"].as_str() {
        Some(state) => json!({ "state": state }),
        None => Value::Null,
    };
    json!({
        "number": pr["number"], "title": pr["title"],
        "url": format!("https://github.com/{}/pull/{}", pr["repo"].as_str().unwrap_or(""), pr["number"]),
        "isDraft": pr["draft"].as_bool().unwrap_or(false),
        "headRefName": pr["head"], "baseRefName": pr["base"], "updatedAt": pr["updated"],
        "additions": 12, "deletions": 3, "reviewDecision": pr["review"],
        "repository": { "nameWithOwner": pr["repo"] },
        "commits": { "nodes": [ { "commit": { "statusCheckRollup": rollup } } ] },
        "reviewThreads": { "totalCount": summary.len(), "nodes": summary },
    })
}

fn thread_node(key: &str, index: usize, t: &Value) -> Value {
    let (repo, number) = key.split_once('#').unwrap_or((key, ""));
    let comments: Vec<Value> = t["comments"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(c, comment)| json!({
            "author": { "login": comment["author"] }, "body": comment["body"], "createdAt": comment["at"],
            "url": format!("https://github.com/{repo}/pull/{number}#discussion_r{}", index * 10 + c),
            "diffHunk": t["hunk"].as_str().unwrap_or("@@ -1,2 +1,2 @@\n context\n+added line"),
        }))
        .collect();
    json!({
        "isResolved": t["resolved"], "isOutdated": t["outdated"].as_bool().unwrap_or(false),
        "path": t["path"], "line": t["line"], "diffSide": t["side"].as_str().unwrap_or("RIGHT"),
        "comments": { "nodes": comments },
    })
}
