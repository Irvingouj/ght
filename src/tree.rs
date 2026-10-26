use std::collections::{BTreeMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowKind {
    Dir(String),
    File(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    pub depth: usize,
    pub name: String,
    pub kind: RowKind,
    pub collapsed: bool,
}

#[derive(Default)]
struct Node {
    dirs: BTreeMap<String, Node>,
    files: Vec<(String, usize)>,
}

/// Directories first, then files, both sorted. Single-child directory chains are
/// merged into one row (`src/app`) the way GitHub's file tree shows them.
pub fn rows(paths: &[String], collapsed: &HashSet<String>) -> Vec<TreeRow> {
    let mut root = Node::default();
    for (i, path) in paths.iter().enumerate() {
        let mut parts: Vec<&str> = path.split('/').collect();
        let file = parts.pop().unwrap_or_default();
        let mut node = &mut root;
        for dir in parts {
            node = node.dirs.entry(dir.to_string()).or_default();
        }
        node.files.push((file.to_string(), i));
    }
    let mut out = Vec::new();
    walk(&root, "", 0, collapsed, &mut out);
    out
}

fn walk(node: &Node, prefix: &str, depth: usize, collapsed: &HashSet<String>, out: &mut Vec<TreeRow>) {
    for (name, child) in &node.dirs {
        let mut name = name.clone();
        let mut full = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
        let mut child = child;
        while child.files.is_empty() && child.dirs.len() == 1 {
            let (next_name, next) = child.dirs.iter().next().expect("one dir");
            name = format!("{name}/{next_name}");
            full = format!("{full}/{next_name}");
            child = next;
        }
        let is_collapsed = collapsed.contains(&full);
        out.push(TreeRow { depth, name, kind: RowKind::Dir(full.clone()), collapsed: is_collapsed });
        if !is_collapsed {
            walk(child, &full, depth + 1, collapsed, out);
        }
    }
    let mut files = node.files.clone();
    files.sort();
    for (name, i) in files {
        out.push(TreeRow { depth, name, kind: RowKind::File(i), collapsed: false });
    }
}

/// File indices in the order the fully expanded tree shows them.
pub fn file_order(paths: &[String]) -> Vec<usize> {
    rows(paths, &HashSet::new())
        .into_iter()
        .filter_map(|r| match r.kind {
            RowKind::File(i) => Some(i),
            RowKind::Dir(_) => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(p: &[&str]) -> Vec<String> {
        p.iter().map(|s| s.to_string()).collect()
    }

    fn render(rows: &[TreeRow]) -> Vec<String> {
        rows.iter()
            .map(|r| {
                let pad = "  ".repeat(r.depth);
                match &r.kind {
                    RowKind::Dir(_) => format!("{pad}{}/{}", r.name, if r.collapsed { " +" } else { "" }),
                    RowKind::File(i) => format!("{pad}{} [{i}]", r.name),
                }
            })
            .collect()
    }

    #[test]
    fn dirs_before_files_and_chains_merged() {
        let p = paths(&["README.md", "src/app/mod.rs", "src/app/keys.rs", "crates/core/src/lib.rs", "src/main.rs"]);
        assert_eq!(
            render(&rows(&p, &HashSet::new())),
            vec![
                "crates/core/src/",
                "  lib.rs [3]",
                "src/",
                "  app/",
                "    keys.rs [2]",
                "    mod.rs [1]",
                "  main.rs [4]",
                "README.md [0]",
            ]
        );
    }

    #[test]
    fn collapsed_dir_hides_children() {
        let p = paths(&["src/app/mod.rs", "src/main.rs", "b.txt"]);
        let collapsed: HashSet<String> = ["src/app".to_string()].into();
        assert_eq!(render(&rows(&p, &collapsed)), vec!["src/", "  app/ +", "  main.rs [1]", "b.txt [2]"]);
    }

    #[test]
    fn file_order_follows_tree() {
        let p = paths(&["z.rs", "a/b.rs", "a/a.rs"]);
        assert_eq!(file_order(&p), vec![2, 1, 0]);
    }
}
