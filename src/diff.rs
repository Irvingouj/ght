use crate::model::{DiffLine, LineKind};

/// Parses the per-file `patch` that GitHub returns (hunks only, no file headers).
pub fn parse_patch(patch: &str) -> Vec<DiffLine> {
    let mut out = Vec::new();
    let (mut old, mut new) = (0u32, 0u32);
    for raw in patch.lines() {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        if raw.starts_with("@@") {
            if let Some((o, n)) = hunk_starts(raw) {
                old = o;
                new = n;
            }
            out.push(DiffLine { kind: LineKind::Hunk, old: None, new: None, text: raw.to_string() });
            continue;
        }
        let (kind, text) = match raw.chars().next() {
            Some('+') => (LineKind::Add, &raw[1..]),
            Some('-') => (LineKind::Del, &raw[1..]),
            Some('\\') => (LineKind::Meta, raw),
            Some(' ') => (LineKind::Ctx, &raw[1..]),
            _ => (LineKind::Ctx, raw),
        };
        let line = match kind {
            LineKind::Add => {
                new += 1;
                DiffLine { kind, old: None, new: Some(new - 1), text: text.into() }
            }
            LineKind::Del => {
                old += 1;
                DiffLine { kind, old: Some(old - 1), new: None, text: text.into() }
            }
            LineKind::Ctx => {
                old += 1;
                new += 1;
                DiffLine { kind, old: Some(old - 1), new: Some(new - 1), text: text.into() }
            }
            _ => DiffLine { kind, old: None, new: None, text: text.into() },
        };
        out.push(line);
    }
    out
}

fn hunk_starts(header: &str) -> Option<(u32, u32)> {
    let mut parts = header.split_whitespace().skip(1);
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    let start = |s: &str| s.split(',').next()?.parse().ok();
    Some((start(old)?, start(new)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_lines_per_side() {
        let patch = "@@ -10,3 +10,4 @@ fn main() {\n ctx\n-old\n+new1\n+new2\n ctx2\n\\ No newline at end of file";
        let lines = parse_patch(patch);
        let got: Vec<_> = lines.iter().map(|l| (l.kind, l.old, l.new, l.text.as_str())).collect();
        assert_eq!(
            got,
            vec![
                (LineKind::Hunk, None, None, "@@ -10,3 +10,4 @@ fn main() {"),
                (LineKind::Ctx, Some(10), Some(10), "ctx"),
                (LineKind::Del, Some(11), None, "old"),
                (LineKind::Add, None, Some(11), "new1"),
                (LineKind::Add, None, Some(12), "new2"),
                (LineKind::Ctx, Some(12), Some(13), "ctx2"),
                (LineKind::Meta, None, None, "\\ No newline at end of file"),
            ]
        );
    }

    #[test]
    fn second_hunk_resets_counters() {
        let lines = parse_patch("@@ -1 +1 @@\n-a\n+b\n@@ -50,2 +60,2 @@\n x\n+y");
        assert_eq!(lines[4].new, Some(60));
        assert_eq!(lines[5].new, Some(61));
        assert_eq!(lines[4].old, Some(50));
    }

    #[test]
    fn new_file_hunk() {
        let lines = parse_patch("@@ -0,0 +1,2 @@\n+a\n+b");
        assert_eq!(lines[1].new, Some(1));
        assert_eq!(lines[2].new, Some(2));
    }
}
