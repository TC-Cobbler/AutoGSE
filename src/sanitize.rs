use std::sync::OnceLock;

use regex::Regex;

/// PRD §5.3.3's literal noise-word pattern. Matched case-insensitively: the
/// PRD's own casing is inconsistent (`FitGirl`, `GOG`, but lowercase `crack`),
/// which only makes sense as a case-insensitive match in practice.
const NOISE_PATTERN: &str =
    r"(?i)\b(v\d+\.\d+|FitGirl|DODI|Repack|Deluxe|Edition|GOG|MULTi\d+|crack|FLT|TENOKE|RUNE|Goldberg)\b";

fn noise_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(NOISE_PATTERN).expect("noise pattern is a valid regex"))
}

/// Strips known repack/release-group noise words and collapses separators,
/// e.g. `"Cyberpunk 2077 v1.63 MULTi12-FitGirl"` -> `"Cyberpunk 2077"`
/// (PRD §5.3.3).
pub fn sanitize_name(raw: &str) -> String {
    let truncated = if raw.len() > 260 { &raw[..260] } else { raw };
    let stripped = noise_regex().replace_all(truncated, " ");
    let separators_collapsed: String = stripped
        .chars()
        .map(|c| if c == '-' || c == '_' || c == '.' { ' ' } else { c })
        .collect();
    separators_collapsed.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prd_worked_example() {
        assert_eq!(sanitize_name("Cyberpunk 2077 v1.63 MULTi12-FitGirl"), "Cyberpunk 2077");
    }

    #[test]
    fn strips_each_noise_word_in_isolation() {
        let cases = [
            ("Hollow Knight DODI", "Hollow Knight"),
            ("Hollow Knight Repack", "Hollow Knight"),
            ("Hollow Knight Deluxe Edition", "Hollow Knight"),
            ("Hollow Knight GOG", "Hollow Knight"),
            ("Hollow Knight crack", "Hollow Knight"),
            ("Hollow Knight FLT", "Hollow Knight"),
            ("Hollow Knight TENOKE", "Hollow Knight"),
            ("Hollow Knight RUNE", "Hollow Knight"),
            ("Hollow Knight Goldberg", "Hollow Knight"),
            ("Hollow Knight v2.10", "Hollow Knight"),
        ];
        for (input, expected) in cases {
            assert_eq!(sanitize_name(input), expected, "input: {input}");
        }
    }

    #[test]
    fn is_case_insensitive() {
        assert_eq!(sanitize_name("Hollow Knight REPACK"), "Hollow Knight");
        assert_eq!(sanitize_name("Hollow Knight goldberg"), "Hollow Knight");
    }

    #[test]
    fn collapses_whitespace_and_trims() {
        assert_eq!(sanitize_name("  Hollow   Knight  "), "Hollow Knight");
    }

    #[test]
    fn leaves_clean_names_untouched() {
        assert_eq!(sanitize_name("Stray"), "Stray");
    }
}
