//! Checks on player-written text before the TV shows it: no empty or symbol-only fakes, and an
//! optional family filter that masks listed words. Local word list only, nothing leaves the host.

use std::sync::LazyLock;

use crate::fuzzy;

static WORDS: LazyLock<Vec<String>> = LazyLock::new(|| {
    include_str!("../data/filter.txt")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(fuzzy::normalize)
        .collect()
});

/// Longest fake a player may write.
pub const MAX_LEN: usize = 40;

/// Trimmed, length-capped text, or None when it has no letter or digit at all.
pub fn accept(text: &str) -> Option<String> {
    let text: String = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_LEN)
        .collect();
    text.chars().any(char::is_alphanumeric).then_some(text)
}

/// `text` with every listed word replaced by stars.
pub fn mask(text: &str) -> String {
    text.split(' ')
        .map(|word| {
            let plain = fuzzy::normalize(word);
            let listed =
                !plain.is_empty() && WORDS.iter().any(|w| plain.split(' ').any(|p| p == w));
            if listed {
                "★★★".to_string()
            } else {
                word.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_and_symbol_only() {
        assert_eq!(accept("   "), None);
        assert_eq!(accept("🎉🎉"), None);
        assert_eq!(accept("?!"), None);
        assert_eq!(accept("  Little   Rock "), Some("Little Rock".to_string()));
        assert_eq!(accept(&"x".repeat(99)).unwrap().len(), MAX_LEN);
    }

    #[test]
    fn masks_listed_words_only() {
        assert_eq!(mask("Shitville"), "Shitville");
        assert_eq!(mask("holy shit"), "holy ★★★");
        assert_eq!(mask("Scunthorpe"), "Scunthorpe");
        assert_eq!(mask("Lake Titicaca"), "Lake Titicaca");
    }
}
