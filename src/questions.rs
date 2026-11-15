//! Questions and where they come from: the offline generator over bundled Wikidata tables, and
//! optionally Claude. Every question from every source passes [`validate`] before it is asked.

pub mod claude;
pub mod data;
pub mod offline;

use std::collections::HashSet;

use serde::Serialize;

use crate::fuzzy;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub enum Category {
    Capitals,
    BigSmall,
    Elements,
    Olympics,
    Space,
    Money,
}
impl Category {
    pub const ALL: [Self; 6] = [
        Self::Capitals,
        Self::BigSmall,
        Self::Elements,
        Self::Olympics,
        Self::Space,
        Self::Money,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Capitals => "Capitals",
            Self::BigSmall => "Big & Small",
            Self::Elements => "Elements",
            Self::Olympics => "Summer Games",
            Self::Space => "Moons",
            Self::Money => "Money",
        }
    }

    /// What Claude is asked to write about for this category.
    pub const fn topic(self) -> &'static str {
        match self {
            Self::Capitals => "capital cities of countries",
            Self::BigSmall => {
                "comparing sizes, populations, lengths or heights of countries, cities, rivers, mountains, animals or buildings"
            }
            Self::Elements => "chemical elements and their properties",
            Self::Olympics => "the history of the Summer Olympic Games",
            Self::Space => "the planets, moons and spaceflight of the Solar System",
            Self::Money => "currencies and money around the world",
        }
    }

    /// Whether the offline generator can write fill-in-the-blank prompts for it.
    pub const fn has_blanks(self) -> bool {
        !matches!(self, Self::BigSmall)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Four choices, one right.
    Choice { choices: [String; 4], answer: usize },
    /// A prompt with a `____` gap: players write fakes, then hunt for `truth` among them.
    Blank {
        truth: String,
        /// Other spellings that also count as the truth.
        accept: Vec<String>,
        /// Plausible wrong answers the house adds when players wrote too few.
        decoys: Vec<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Wanted {
    Choice,
    Blank,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Source {
    Offline,
    Claude,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    pub prompt: String,
    pub kind: Kind,
    pub category: Category,
    /// Dedupe key: one fact is asked once per session, whatever the wording.
    pub key: String,
    pub source: Source,
    /// A line shown on the reveal.
    pub note: Option<String>,
}
impl Question {
    pub const fn wanted(&self) -> Wanted {
        match self.kind {
            Kind::Choice { .. } => Wanted::Choice,
            Kind::Blank { .. } => Wanted::Blank,
        }
    }

    /// The right answer as text.
    pub fn truth(&self) -> &str {
        match &self.kind {
            Kind::Choice { choices, answer } => &choices[*answer],
            Kind::Blank { truth, .. } => truth,
        }
    }
}

/// The blank in a fill-in prompt.
pub const GAP: &str = "____";
pub const MAX_PROMPT: usize = 140;
pub const MAX_CHOICE: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reject {
    EmptyText,
    TooLong,
    DuplicateChoices,
    AnswerOutOfRange,
    AnswerInPrompt,
    NoGap,
    Used,
}

/// Whether a question may be asked: fits the screens, has one clear answer, and is new this session.
pub fn validate(
    question: &Question,
    used: &HashSet<String>,
) -> Result<(), Reject> {
    let prompt = question.prompt.trim();
    if prompt.is_empty() {
        return Err(Reject::EmptyText);
    }
    if prompt.chars().count() > MAX_PROMPT {
        return Err(Reject::TooLong);
    }
    if used.contains(&question.key) {
        return Err(Reject::Used);
    }
    let shown: Vec<&str> = match &question.kind {
        Kind::Choice { choices, answer } => {
            if *answer >= choices.len() {
                return Err(Reject::AnswerOutOfRange);
            }
            choices.iter().map(String::as_str).collect()
        }
        Kind::Blank { truth, decoys, .. } => {
            if !prompt.contains(GAP) {
                return Err(Reject::NoGap);
            }
            std::iter::once(truth.as_str())
                .chain(decoys.iter().map(String::as_str))
                .collect()
        }
    };
    if shown
        .iter()
        .any(|c| c.trim().is_empty() || !c.chars().any(char::is_alphanumeric))
    {
        return Err(Reject::EmptyText);
    }
    if shown.iter().any(|c| c.chars().count() > MAX_CHOICE) {
        return Err(Reject::TooLong);
    }
    for (at, a) in shown.iter().enumerate() {
        if shown[at + 1..]
            .iter()
            .any(|b| fuzzy::normalize(a) == fuzzy::normalize(b))
        {
            return Err(Reject::DuplicateChoices);
        }
    }
    let truth = fuzzy::normalize(question.truth());
    let words = format!(" {} ", fuzzy::normalize(prompt));
    if words.contains(&format!(" {truth} ")) {
        return Err(Reject::AnswerInPrompt);
    }
    Ok(())
}

/// Something that writes questions.
pub trait Generator {
    /// A new question for this category and kind, or None when it has nothing (left).
    fn next(
        &mut self,
        category: Category,
        wanted: Wanted,
        used: &HashSet<String>,
    ) -> Option<Question>;
}

/// Serves Claude's questions while it has valid ones for the slot and offline ones otherwise,
/// never the same fact or prompt twice in a session.
pub struct Mixer {
    pub offline: offline::Offline,
    pub claude: Option<claude::Claude>,
    /// Ask Claude at all (the lobby option); offline only when false.
    pub use_claude: bool,
    used: HashSet<String>,
}
impl Mixer {
    pub fn new(
        seed: u64,
        claude: Option<claude::Claude>,
    ) -> Self {
        Self {
            offline: offline::Offline::new(seed),
            use_claude: claude.is_some(),
            claude,
            used: HashSet::new(),
        }
    }

    /// Ask Claude for a batch ahead of time; a no-op offline.
    pub fn prepare(
        &mut self,
        category: Category,
        wanted: Wanted,
    ) {
        if let Some(claude) = self.claude.as_mut().filter(|_| self.use_claude) {
            claude.request(category, wanted);
        }
    }

    /// The next question for the slot. Falls back to another category when this one ran dry
    /// (blank prompts for a category without them, for example).
    pub fn next(
        &mut self,
        category: Category,
        wanted: Wanted,
    ) -> Question {
        let from_claude = self
            .claude
            .as_mut()
            .filter(|_| self.use_claude)
            .and_then(|c| c.take(category, wanted, &self.used));
        let question = from_claude
            .or_else(|| self.offline.next(category, wanted, &self.used))
            .or_else(|| {
                Category::ALL
                    .iter()
                    .find_map(|c| self.offline.next(*c, wanted, &self.used))
            })
            .unwrap_or_else(|| {
                // Every table exhausted in one session: start over rather than stall
                self.used.clear();
                self.offline
                    .next(Category::Elements, wanted, &self.used)
                    .expect("elements always yield")
            });
        self.used.insert(question.key.clone());
        self.used
            .insert(format!("prompt:{}", fuzzy::normalize(&question.prompt)));
        question
    }

    pub const fn used(&self) -> &HashSet<String> {
        &self.used
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(
        prompt: &str,
        choices: [&str; 4],
    ) -> Question {
        Question {
            prompt: prompt.to_string(),
            kind: Kind::Choice {
                choices: choices.map(str::to_string),
                answer: 0,
            },
            category: Category::Capitals,
            key: "k".to_string(),
            source: Source::Offline,
            note: None,
        }
    }

    #[test]
    fn validate_rejects_bad_questions() {
        let none = HashSet::new();
        let good = choice(
            "What is the capital of Peru?",
            ["Lima", "Quito", "La Paz", "Bogotá"],
        );
        assert_eq!(validate(&good, &none), Ok(()));
        let dupes = choice("Capital of Peru?", ["Lima", "lima!", "La Paz", "Quito"]);
        assert_eq!(validate(&dupes, &none), Err(Reject::DuplicateChoices));
        let given = choice(
            "Is Lima the capital of Peru?",
            ["Lima", "Quito", "La Paz", "Cusco"],
        );
        assert_eq!(validate(&given, &none), Err(Reject::AnswerInPrompt));
        let long = choice("Capital?", ["Lima", &"x".repeat(40), "La Paz", "Quito"]);
        assert_eq!(validate(&long, &none), Err(Reject::TooLong));
        let empty = choice("Capital?", ["Lima", " ", "La Paz", "Quito"]);
        assert_eq!(validate(&empty, &none), Err(Reject::EmptyText));
        let used = HashSet::from(["k".to_string()]);
        assert_eq!(validate(&good, &used), Err(Reject::Used));
        let mut out_of_range = good;
        if let Kind::Choice { answer, .. } = &mut out_of_range.kind {
            *answer = 4;
        }
        assert_eq!(
            validate(&out_of_range, &none),
            Err(Reject::AnswerOutOfRange)
        );
        let blank = Question {
            prompt: "The capital of Peru is".to_string(),
            kind: Kind::Blank {
                truth: "Lima".to_string(),
                accept: vec![],
                decoys: vec![],
            },
            ..choice("x", ["a", "b", "c", "d"])
        };
        assert_eq!(validate(&blank, &none), Err(Reject::NoGap));
    }

    #[test]
    fn mixer_never_repeats_and_never_runs_dry() {
        let mut mixer = Mixer::new(7, None);
        let mut prompts = HashSet::new();
        for i in 0..600 {
            let category = Category::ALL[i % Category::ALL.len()];
            let wanted = if i % 3 == 0 {
                Wanted::Blank
            } else {
                Wanted::Choice
            };
            let question = mixer.next(category, wanted);
            assert_eq!(question.wanted(), wanted);
            assert!(
                prompts.insert(question.prompt.clone()),
                "repeat: {}",
                question.prompt
            );
        }
    }
}
