use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

/// fzf-style matcher: space-separated terms, `'exact`, `^prefix`, `suffix$`, `!not`.
pub struct Fuzzy {
    pattern: Pattern,
    matcher: Matcher,
    buf: Vec<char>,
}

impl Fuzzy {
    pub fn new(query: &str) -> Self {
        Self {
            pattern: Pattern::parse(query, CaseMatching::Smart, Normalization::Smart),
            matcher: Matcher::new(Config::DEFAULT.match_paths()),
            buf: Vec::new(),
        }
    }

    /// Score and matched char positions, or `None` when `haystack` does not match.
    pub fn matches(&mut self, haystack: &str) -> Option<(u32, Vec<u32>)> {
        let mut indices = Vec::new();
        let score = self.pattern.indices(Utf32Str::new(haystack, &mut self.buf), &mut self.matcher, &mut indices)?;
        indices.sort_unstable();
        indices.dedup();
        Some((score, indices))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subsequence_matches_and_reports_positions() {
        let mut f = Fuzzy::new("agpk");
        let (_, idx) = f.matches("agent picker").unwrap();
        assert_eq!(idx, vec![0, 1, 6, 9]);
        assert!(f.matches("gateway").is_none());
    }

    #[test]
    fn fzf_syntax_negation_and_exact() {
        let mut f = Fuzzy::new("'rdp !wip");
        assert!(f.matches("ironrdp fix").is_some());
        assert!(f.matches("ironrdp wip").is_none());
        assert!(f.matches("ironrxdxp").is_none());
    }

    #[test]
    fn better_match_scores_higher() {
        let mut f = Fuzzy::new("picker");
        let (tight, _) = f.matches("agent picker").unwrap();
        let (loose, _) = f.matches("p-i-c-k-e-r").unwrap();
        assert!(tight > loose);
    }
}
