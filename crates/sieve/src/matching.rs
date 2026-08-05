use crate::instruction::{Comparator, MatchType};

pub(crate) fn value_matches(
    match_type: MatchType,
    comparator: Comparator,
    key: &str,
    value: &str,
) -> bool {
    match match_type {
        MatchType::Is => match comparator {
            Comparator::Octet => value == key,
            Comparator::AsciiCaseMap => value.eq_ignore_ascii_case(key),
        },
        MatchType::Contains => {
            if key.is_empty() {
                return true;
            }
            match comparator {
                Comparator::Octet => value.contains(key),
                Comparator::AsciiCaseMap => value
                    .to_ascii_lowercase()
                    .contains(&key.to_ascii_lowercase()),
            }
        }
        MatchType::Matches => match comparator {
            Comparator::Octet => glob_matches(key, value),
            Comparator::AsciiCaseMap => {
                glob_matches(&key.to_ascii_lowercase(), &value.to_ascii_lowercase())
            }
        },
    }
}

enum Pat {
    Many,
    Single,
    Char(char),
}

fn compile_glob(pattern: &str) -> Vec<Pat> {
    let mut compiled = Vec::new();
    let mut chars = pattern.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '*' => {
                if !matches!(compiled.last(), Some(Pat::Many)) {
                    compiled.push(Pat::Many);
                }
            }
            '?' => compiled.push(Pat::Single),
            '\\' => compiled.push(Pat::Char(chars.next().unwrap_or('\\'))),
            _ => compiled.push(Pat::Char(ch)),
        }
    }
    compiled
}

// Two-pointer glob with a single backtrack point: linear, no recursion (research.swtch.com/glob).
fn glob_matches(pattern: &str, value: &str) -> bool {
    let pattern = compile_glob(pattern);
    let value: Vec<char> = value.chars().collect();
    let (mut px, mut nx, mut next_px, mut next_nx) = (0usize, 0usize, 0usize, 0usize);
    while px < pattern.len() || nx < value.len() {
        if px < pattern.len() {
            match pattern[px] {
                Pat::Char(ch) if nx < value.len() && value[nx] == ch => {
                    px += 1;
                    nx += 1;
                    continue;
                }
                Pat::Single if nx < value.len() => {
                    px += 1;
                    nx += 1;
                    continue;
                }
                Pat::Many => {
                    next_px = px;
                    next_nx = nx + 1;
                    px += 1;
                    continue;
                }
                _ => {}
            }
        }
        if next_nx > 0 && next_nx <= value.len() {
            px = next_px;
            nx = next_nx;
            continue;
        }
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_compares_exactly_under_octet_and_folds_under_casemap() {
        assert!(value_matches(
            MatchType::Is,
            Comparator::Octet,
            "abc",
            "abc"
        ));
        assert!(!value_matches(
            MatchType::Is,
            Comparator::Octet,
            "ABC",
            "abc"
        ));
        assert!(value_matches(
            MatchType::Is,
            Comparator::AsciiCaseMap,
            "ABC",
            "abc"
        ));
    }

    #[test]
    fn contains_finds_substrings_case_insensitively_by_default() {
        assert!(value_matches(
            MatchType::Contains,
            Comparator::AsciiCaseMap,
            "Deal",
            "weekly DEALS inside"
        ));
        assert!(!value_matches(
            MatchType::Contains,
            Comparator::Octet,
            "Deal",
            "weekly DEALS inside"
        ));
    }

    #[test]
    fn contains_with_an_empty_key_is_always_true() {
        assert!(value_matches(
            MatchType::Contains,
            Comparator::Octet,
            "",
            "anything"
        ));
        assert!(value_matches(
            MatchType::Contains,
            Comparator::Octet,
            "",
            ""
        ));
    }

    #[test]
    fn casemap_folding_is_ascii_only() {
        assert!(!value_matches(
            MatchType::Is,
            Comparator::AsciiCaseMap,
            "STRASSE",
            "straße"
        ));
    }

    #[test]
    fn glob_star_matches_any_run_including_empty() {
        assert!(glob_matches("a*c", "ac"));
        assert!(glob_matches("a*c", "abbbc"));
        assert!(glob_matches("*", ""));
        assert!(glob_matches("*deals*", "weekly deals inside"));
        assert!(!glob_matches("a*c", "abd"));
    }

    #[test]
    fn glob_question_mark_matches_exactly_one_character() {
        assert!(glob_matches("a?c", "abc"));
        assert!(!glob_matches("a?c", "ac"));
        assert!(!glob_matches("a?c", "abbc"));
    }

    #[test]
    fn glob_escapes_make_wildcards_literal() {
        assert!(glob_matches("a\\*c", "a*c"));
        assert!(!glob_matches("a\\*c", "abc"));
        assert!(glob_matches("a\\\\c", "a\\c"));
        assert!(glob_matches("tail\\", "tail\\"));
    }

    #[test]
    fn glob_consecutive_stars_collapse() {
        assert!(glob_matches("a**b", "ab"));
        assert!(glob_matches("a***b", "axyzb"));
    }

    #[test]
    fn glob_backtracks_to_the_latest_star() {
        assert!(glob_matches("*ab", "aab"));
        assert!(glob_matches("a*b*c", "aXbYbZc"));
        assert!(!glob_matches("a*b*c", "aXbYb"));
    }

    #[test]
    fn glob_handles_pathological_repeats_quickly() {
        let value = "a".repeat(2000);
        assert!(!glob_matches("*a*a*a*a*a*a*a*a*b", &value));
    }

    #[test]
    fn glob_compares_whole_values() {
        assert!(!glob_matches("deals", "weekly deals inside"));
        assert!(glob_matches("weekly deals inside", "weekly deals inside"));
    }

    #[test]
    fn glob_matches_multibyte_characters_as_single_units() {
        assert!(glob_matches("?", "ß"));
        assert!(glob_matches("gr?ße", "größe"));
    }

    #[test]
    fn matches_folds_case_under_the_default_comparator() {
        assert!(value_matches(
            MatchType::Matches,
            Comparator::AsciiCaseMap,
            "*DEALS*",
            "weekly deals inside"
        ));
        assert!(!value_matches(
            MatchType::Matches,
            Comparator::Octet,
            "*DEALS*",
            "weekly deals inside"
        ));
    }
}
