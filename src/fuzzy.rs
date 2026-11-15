//! Loose text matching for bluff answers: a fake "equals" the truth when it reads the same once
//! case, accents, punctuation, leading articles and number forms are folded away, give or take a typo.

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// Fold `text` to lowercase ASCII-ish words: no accents or punctuation, digits written out as
/// words ("1,000" -> "one thousand"), no leading "the", "a" or "an".
pub fn normalize(text: &str) -> String {
    let folded: String = text
        .nfd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .collect();
    // Thousands separators inside numbers go before punctuation turns them into spaces
    let chars: Vec<char> = folded.chars().collect();
    let mut cleaned = String::new();
    for (at, c) in chars.iter().enumerate() {
        let between_digits = at > 0
            && chars.get(at + 1).is_some_and(char::is_ascii_digit)
            && chars[at - 1].is_ascii_digit();
        if matches!(c, ',' | '_') && between_digits {
            continue;
        }
        cleaned.push(if c.is_alphanumeric() { *c } else { ' ' });
    }
    let mut words = Vec::new();
    for word in cleaned.split_whitespace() {
        match word.parse::<u64>() {
            Ok(n) => words.extend(number_words(n).split(' ').map(str::to_string)),
            Err(_) => words.push(word.to_string()),
        }
    }
    if words.len() > 1 && matches!(words[0].as_str(), "the" | "a" | "an") {
        words.remove(0);
    }
    words.join(" ")
}

/// Whether two answers are the same answer: equal after [`normalize`], or within one edit for
/// short answers and two for longer ones (spaces ignored, so "new york" = "newyork").
pub fn same(
    a: &str,
    b: &str,
) -> bool {
    let a: String = normalize(a).split(' ').collect();
    let b: String = normalize(b).split(' ').collect();
    if a.is_empty() || b.is_empty() {
        return false;
    }
    let limit = if a.chars().count().max(b.chars().count()) <= 6 {
        1
    } else {
        2
    };
    strsim::levenshtein(&a, &b) <= limit
}

const ONES: [&str; 20] = [
    "zero",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
];
const TENS: [&str; 10] = [
    "", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
];

/// English words for a number, without "and" or hyphens: 1,204 -> "one thousand two hundred four".
pub fn number_words(n: u64) -> String {
    if n < 20 {
        return ONES[n as usize].to_string();
    }
    if n < 100 {
        let tens = TENS[(n / 10) as usize];
        return if n.is_multiple_of(10) {
            tens.to_string()
        } else {
            format!("{tens} {}", ONES[(n % 10) as usize])
        };
    }
    let (unit, name) = if n < 1_000 {
        (100, "hundred")
    } else if n < 1_000_000 {
        (1_000, "thousand")
    } else if n < 1_000_000_000 {
        (1_000_000, "million")
    } else {
        (1_000_000_000, "billion")
    };
    let head = format!("{} {name}", number_words(n / unit));
    if n.is_multiple_of(unit) {
        head
    } else {
        format!("{head} {}", number_words(n % unit))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_case_accents_articles_and_numbers() {
        assert_eq!(normalize("  The Asunción!  "), "asuncion");
        assert_eq!(normalize("1,000 cats"), "one thousand cats");
        assert_eq!(normalize("7"), "seven");
        assert_eq!(normalize("A"), "a");
        assert_eq!(number_words(1204), "one thousand two hundred four");
        assert_eq!(number_words(90), "ninety");
    }

    #[test]
    fn matches_typos_and_forms_but_not_other_answers() {
        assert!(same("Bishkek", "bishkek"));
        assert!(same("Bishkek", "Bishkec"));
        assert!(same("Ulaanbaatar", "Ulan Baatar"));
        assert!(same("seven", "7"));
        assert!(same("the Moon", "moon"));
        assert!(!same("Oslo", "Lima"));
        assert!(!same("Rome", "Rabat"));
        assert!(!same("Mars", "Mercury"));
        assert!(!same("", ""));
        assert!(!same("!!!", "truth"));
    }
}
