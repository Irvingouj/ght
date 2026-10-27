use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::github::repo_from_remote;

/// Folders that may hold clones: where ght started, and the folder that holds
/// the current checkout's main clone (so its sibling clones are found too).
pub fn search_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }
    if let Some(common) = git(Path::new("."), &["rev-parse", "--path-format=absolute", "--git-common-dir"])
        && let Some(parent) = Path::new(common.trim()).parent().and_then(Path::parent)
        && !roots.iter().any(|r| r == parent)
    {
        roots.push(parent.to_path_buf());
    }
    roots
}

/// The worktree that has the PR's branch checked out, in any clone of `repo` at or right under `roots`.
pub fn find(roots: &[PathBuf], repo: &str, branch: &str) -> Option<PathBuf> {
    let repo = repo.to_lowercase();
    clones(roots).into_iter().filter(|c| remotes(c).contains(&repo)).find_map(|c| {
        let list = git(&c, &["worktree", "list", "--porcelain"])?;
        let tracks = git(&c, &["for-each-ref", "--format=%(refname:short) %(upstream:lstrip=3)", "refs/heads"]).unwrap_or_default();
        pick(&list, &tracks, branch)
    })
}

/// Main clones only: a worktree has a `.git` file, and its main clone lists it anyway.
fn clones(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for root in roots {
        let children = std::fs::read_dir(root).into_iter().flatten().flatten().map(|e| e.path());
        for dir in std::iter::once(root.clone()).chain(children) {
            if dir.join(".git").is_dir() && !out.contains(&dir) {
                out.push(dir);
            }
        }
    }
    out
}

/// Read straight from `.git/config` so scanning a folder of many clones stays fast.
fn remotes(clone: &Path) -> Vec<String> {
    std::fs::read_to_string(clone.join(".git").join("config"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.trim().strip_prefix("url")?.trim_start().strip_prefix('='))
        .filter_map(|url| repo_from_remote(url.trim()))
        .collect()
}

/// First worktree whose branch is `branch`, or whose branch tracks a remote branch named `branch`.
fn pick(porcelain: &str, tracks: &str, branch: &str) -> Option<PathBuf> {
    let upstream: HashMap<&str, &str> = tracks.lines().filter_map(|l| l.split_once(' ')).collect();
    let mut path = None;
    for line in porcelain.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            path = Some(PathBuf::from(p));
        } else if let Some(b) = line.strip_prefix("branch refs/heads/")
            && (b == branch || upstream.get(b) == Some(&branch))
        {
            return path;
        }
    }
    None
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = "worktree D:/IronRDP\nHEAD 9b15\nbranch refs/heads/master\n\nworktree D:/IronRDP.worktrees/uplift\nHEAD 2043\nbranch refs/heads/local-name\n\nworktree D:/IronRDP.worktrees/detached\nHEAD 1111\ndetached\n";

    #[test]
    fn picks_worktree_by_branch_name_or_upstream() {
        let tracks = "master master\nlocal-name ci/ffi-bindings-drift\n";
        assert_eq!(pick(LIST, tracks, "master"), Some(PathBuf::from("D:/IronRDP")));
        assert_eq!(pick(LIST, tracks, "ci/ffi-bindings-drift"), Some(PathBuf::from("D:/IronRDP.worktrees/uplift")));
        assert_eq!(pick(LIST, tracks, "local-name"), Some(PathBuf::from("D:/IronRDP.worktrees/uplift")));
        assert_eq!(pick(LIST, tracks, "nope"), None);
    }

    #[test]
    fn finds_worktree_in_a_sibling_clone() {
        let root = std::env::temp_dir().join(format!("ght-wt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let clone = root.join("proj");
        std::fs::create_dir_all(&clone).unwrap();
        let run = |dir: &Path, args: &[&str]| assert!(Command::new("git").arg("-C").arg(dir).args(args).output().unwrap().status.success(), "git {args:?}");
        run(&clone, &["init", "-q", "-b", "main"]);
        run(&clone, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "init"]);
        run(&clone, &["remote", "add", "origin", "git@github.com:Owner/Proj.git"]);
        run(&clone, &["worktree", "add", "-q", "-b", "feature/x", "../proj.worktrees/x"]);
        std::fs::create_dir_all(root.join("not-a-repo")).unwrap();

        let found = find(std::slice::from_ref(&root), "owner/proj", "feature/x").expect("worktree found");
        assert!(found.ends_with("proj.worktrees/x"), "{found:?}");
        assert_eq!(find(std::slice::from_ref(&root), "other/proj", "feature/x"), None, "remote must match the PR repo");
        assert_eq!(find(std::slice::from_ref(&root), "owner/proj", "feature/y"), None);
        let _ = std::fs::remove_dir_all(&root);
    }
}
