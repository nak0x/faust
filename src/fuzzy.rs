//! Thin wrapper over `nucleo-matcher`, reusing its scratch buffer.

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

pub struct Fuzzy {
    matcher: Matcher,
    /// Reused across calls; scoring a list otherwise allocates per candidate.
    buf: Vec<char>,
}

impl Default for Fuzzy {
    fn default() -> Self {
        Self::new()
    }
}

impl Fuzzy {
    pub fn new() -> Self {
        Self {
            // `match_paths` biases towards the last path component, which is
            // what you mean when you type a note name.
            matcher: Matcher::new(Config::DEFAULT.match_paths()),
            buf: Vec::new(),
        }
    }

    /// Compiles a query. Space-separated terms must all match.
    pub fn pattern(query: &str) -> Pattern {
        Pattern::parse(query, CaseMatching::Smart, Normalization::Smart)
    }

    /// Score of `haystack` against `pattern`; `None` means no match.
    pub fn score(&mut self, pattern: &Pattern, haystack: &str) -> Option<u32> {
        pattern.score(Utf32Str::new(haystack, &mut self.buf), &mut self.matcher)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subsequences_match_and_rank_sensibly() {
        let mut fuzzy = Fuzzy::new();
        let pattern = Fuzzy::pattern("dnote");
        let exact = fuzzy.score(&pattern, "notes/daily/dnote.md");
        let spread = fuzzy.score(&pattern, "d/other/n/o/t/e.md");
        assert!(exact.is_some());
        assert!(exact > spread);
        assert!(fuzzy.score(&pattern, "unrelated.md").is_none());
    }

    #[test]
    fn an_empty_query_matches_everything() {
        let mut fuzzy = Fuzzy::new();
        let pattern = Fuzzy::pattern("");
        assert!(fuzzy.score(&pattern, "anything").is_some());
    }
}
