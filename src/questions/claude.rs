//! Questions from Claude, when the host sets `ANTHROPIC_API_KEY`.
//!
//! A worker thread asks for a batch
//! per category (fast model), has a stronger model fact-check it, and hands back only questions that
//! pass [`super::validate`]. Any failure leaves the slot to the offline generator. The key is read
//! from the environment and never logged or stored.

use std::collections::HashSet;
use std::sync::mpsc;
use std::time::Duration;

use rand::Rng;
use rand::SeedableRng;
use rand::rngs::StdRng;
use rand::seq::IndexedRandom;
use serde::Deserialize;
use serde_json::json;

use super::Category;
use super::Kind;
use super::Question;
use super::Source;
use super::Wanted;
use super::validate;
use crate::fuzzy;

const URL: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";
/// Writes the questions: the fast tier.
pub const GENERATE_MODEL: &str = "claude-haiku-5-5";
/// Checks them: the stronger model.
pub const CHECK_MODEL: &str = "claude-sonnet-5-5";
/// Requests per session, generation and fact-checks together. After that, offline only.
pub const MAX_REQUESTS: u32 = 12;
const TIMEOUT: Duration = Duration::from_secs(20);
const BATCH: usize = 8;
/// Angles that vary batches for the same category.
const ANGLES: [&str; 8] = [
    "history",
    "geography",
    "records",
    "everyday life",
    "science",
    "oddities",
    "firsts",
    "names",
];

const SYSTEM: &str = "You write questions for a party trivia game that friends play on a TV. \
Use only well-established facts that have been true for years and will stay true: nothing that \
changes often (no populations, records, current office holders or prices) and nothing after 2024. \
No opinions, no trick wording, no ambiguity: exactly one answer is right. Keep every prompt under \
120 characters and every answer under 28 characters. Write in plain English.";

struct Job {
    category: Category,
    wanted: Wanted,
}

struct Done {
    category: Category,
    wanted: Wanted,
    questions: Vec<Question>,
}

pub struct Claude {
    jobs: mpsc::Sender<Job>,
    done: mpsc::Receiver<Done>,
    ready: Vec<Question>,
    pending: HashSet<(Category, Wanted)>,
    requests: u32,
}
impl Claude {
    /// A generator when `ANTHROPIC_API_KEY` is set and not empty.
    pub fn from_env(seed: u64) -> Option<Self> {
        let key = std::env::var("ANTHROPIC_API_KEY")
            .ok()
            .filter(|k| !k.trim().is_empty())?;
        let (jobs, inbox) = mpsc::channel::<Job>();
        let (outbox, done) = mpsc::channel();
        std::thread::spawn(move || {
            let agent: ureq::Agent = ureq::Agent::config_builder()
                .timeout_global(Some(TIMEOUT))
                .build()
                .into();
            let mut rng = StdRng::seed_from_u64(seed);
            for job in inbox {
                let questions = match batch(&agent, &key, &job, &mut rng) {
                    Ok(questions) => questions,
                    Err(err) => {
                        eprintln!(
                            "hunch: Claude questions for {} failed, offline instead: {err}",
                            job.category.name()
                        );
                        Vec::new()
                    }
                };
                let reply = Done {
                    category: job.category,
                    wanted: job.wanted,
                    questions,
                };
                if outbox.send(reply).is_err() {
                    break;
                }
            }
        });
        Some(Self {
            jobs,
            done,
            ready: Vec::new(),
            pending: HashSet::new(),
            requests: 0,
        })
    }

    pub const fn requests(&self) -> u32 {
        self.requests
    }

    /// Ask for a batch for this slot, unless one is on its way, enough are ready or the budget is spent.
    pub fn request(
        &mut self,
        category: Category,
        wanted: Wanted,
    ) {
        self.collect();
        let ready = self
            .ready
            .iter()
            .filter(|q| q.category == category && q.wanted() == wanted)
            .count();
        if self.pending.contains(&(category, wanted))
            || ready >= 3
            || self.requests + 2 > MAX_REQUESTS
        {
            return;
        }
        self.requests += 2;
        if self.jobs.send(Job { category, wanted }).is_ok() {
            self.pending.insert((category, wanted));
        }
    }

    /// A ready question for the slot that has not been used.
    pub fn take(
        &mut self,
        category: Category,
        wanted: Wanted,
        used: &HashSet<String>,
    ) -> Option<Question> {
        self.collect();
        let at = self.ready.iter().position(|q| {
            q.category == category
                && q.wanted() == wanted
                && !used.contains(&format!("prompt:{}", fuzzy::normalize(&q.prompt)))
                && validate(q, used).is_ok()
        })?;
        Some(self.ready.remove(at))
    }

    fn collect(&mut self) {
        for done in self.done.try_iter() {
            self.pending.remove(&(done.category, done.wanted));
            self.ready.extend(done.questions);
        }
    }
}

/// One batch: generate, then fact-check.
fn batch(
    agent: &ureq::Agent,
    key: &str,
    job: &Job,
    rng: &mut StdRng,
) -> anyhow::Result<Vec<Question>> {
    let angle = ANGLES.choose(rng).copied().unwrap_or("history");
    let reply = call(
        agent,
        key,
        &generate_request(job.category, job.wanted, angle),
    )?;
    let questions = parse_questions(&reply, job.category, job.wanted, rng)?;
    if questions.is_empty() {
        anyhow::bail!("no valid questions in the reply");
    }
    let reply = call(agent, key, &check_request(&questions))?;
    let ok = parse_verdicts(&reply)?;
    Ok(questions
        .into_iter()
        .enumerate()
        .filter(|(at, _)| ok.contains(at))
        .map(|(_, q)| q)
        .collect())
}

fn call(
    agent: &ureq::Agent,
    key: &str,
    body: &serde_json::Value,
) -> anyhow::Result<String> {
    let mut response = agent
        .post(URL)
        .header("x-api-key", key)
        .header("anthropic-version", API_VERSION)
        .header("content-type", "application/json")
        .send(body.to_string())?;
    Ok(response.body_mut().read_to_string()?)
}

fn schema() -> serde_json::Value {
    let text = json!({ "type": "string" });
    let list = json!({ "type": "array", "items": { "type": "string" } });
    json!({
        "type": "object",
        "properties": {
            "questions": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "prompt": text, "answer": text, "wrong": list, "also_accept": list, "note": text,
                    },
                    "required": ["prompt", "answer", "wrong", "also_accept", "note"],
                    "additionalProperties": false,
                },
            },
        },
        "required": ["questions"],
        "additionalProperties": false,
    })
}

pub fn generate_request(
    category: Category,
    wanted: Wanted,
    angle: &str,
) -> serde_json::Value {
    let topic = category.topic();
    let ask = match wanted {
        Wanted::Choice => format!(
            "Write {BATCH} multiple-choice questions about {topic}, leaning towards {angle}. Mix easy and \
             hard. Give the right `answer` and three `wrong` answers that are the same kind of thing, \
             plausible and clearly wrong. `also_accept` may stay empty. `note` is one short surprising \
             fact about the answer."
        ),
        Wanted::Blank => format!(
            "Write {BATCH} fill-in-the-blank statements about {topic}, leaning towards {angle}, for a \
             bluffing game: players invent fake answers and the others must spot the truth. Each \
             `prompt` is a true statement with the gap written as ____ where a short answer (1 to 3 \
             words) goes, and the gap is not at the start. Pick facts with a surprising answer. \
             `answer` is the real one, `also_accept` other spellings of it, `wrong` three convincing \
             fakes, `note` one short line about it."
        ),
    };
    json!({
        "model": GENERATE_MODEL,
        "max_tokens": 6000,
        "system": SYSTEM,
        "output_config": { "effort": "low", "format": { "type": "json_schema", "schema": schema() } },
        "messages": [{ "role": "user", "content": ask }],
    })
}

pub fn check_request(questions: &[Question]) -> serde_json::Value {
    let items: Vec<serde_json::Value> = questions
        .iter()
        .enumerate()
        .map(|(at, q)| {
            let wrong: Vec<&str> = match &q.kind {
                Kind::Choice { choices, answer } => choices
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| i != answer)
                    .map(|(_, c)| c.as_str())
                    .collect(),
                Kind::Blank { decoys, .. } => decoys.iter().map(String::as_str).collect(),
            };
            json!({ "i": at, "prompt": q.prompt, "answer": q.truth(), "wrong": wrong })
        })
        .collect();
    let ask = format!(
        "Fact-check these trivia items. Mark ok=true only if the answer is definitely correct, every \
         wrong answer is definitely wrong, and the item has exactly one right answer as worded. Give \
         a verdict for every i.\n{}",
        serde_json::Value::Array(items)
    );
    let verdict = json!({
        "type": "object",
        "properties": { "i": { "type": "integer" }, "ok": { "type": "boolean" }, "why": { "type": "string" } },
        "required": ["i", "ok", "why"],
        "additionalProperties": false,
    });
    json!({
        "model": CHECK_MODEL,
        "max_tokens": 4000,
        "system": "You are a careful fact-checker for a trivia game. When unsure, say not ok.",
        "output_config": {
            "effort": "medium",
            "format": {
                "type": "json_schema",
                "schema": {
                    "type": "object",
                    "properties": { "results": { "type": "array", "items": verdict } },
                    "required": ["results"],
                    "additionalProperties": false,
                },
            },
        },
        "messages": [{ "role": "user", "content": ask }],
    })
}

#[derive(Deserialize)]
struct Reply {
    content: Vec<Block>,
    stop_reason: Option<String>,
}

#[derive(Deserialize)]
struct Block {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: String,
}

/// The JSON text of a finished reply.
fn text(reply: &str) -> anyhow::Result<String> {
    let reply: Reply = serde_json::from_str(reply)?;
    match reply.stop_reason.as_deref() {
        Some("end_turn") => {}
        other => anyhow::bail!("reply stopped early: {other:?}"),
    }
    Ok(reply
        .content
        .into_iter()
        .filter(|b| b.kind == "text")
        .map(|b| b.text)
        .collect())
}

#[derive(Deserialize)]
struct Batch {
    questions: Vec<Raw>,
}

#[derive(Deserialize)]
struct Raw {
    prompt: String,
    answer: String,
    wrong: Vec<String>,
    #[serde(default)]
    also_accept: Vec<String>,
    #[serde(default)]
    note: String,
}

/// Questions from a generation reply that pass validation; the rest are dropped.
pub fn parse_questions(
    reply: &str,
    category: Category,
    wanted: Wanted,
    rng: &mut StdRng,
) -> anyhow::Result<Vec<Question>> {
    let batch: Batch = serde_json::from_str(&text(reply)?)?;
    let none = HashSet::new();
    Ok(batch
        .questions
        .into_iter()
        .filter_map(|raw| {
            let trim = |s: &String| s.trim().to_string();
            let kind = match wanted {
                Wanted::Choice => {
                    let wrong: Vec<String> = raw.wrong.iter().map(trim).take(3).collect();
                    if wrong.len() < 3 {
                        return None;
                    }
                    let answer = rng.random_range(0..4);
                    let mut choices = wrong;
                    choices.insert(answer, trim(&raw.answer));
                    Kind::Choice {
                        choices: [
                            choices[0].clone(),
                            choices[1].clone(),
                            choices[2].clone(),
                            choices[3].clone(),
                        ],
                        answer,
                    }
                }
                Wanted::Blank => Kind::Blank {
                    truth: trim(&raw.answer),
                    accept: raw.also_accept.iter().map(trim).collect(),
                    decoys: raw.wrong.iter().map(trim).collect(),
                },
            };
            let question = Question {
                key: format!("claude:{}", fuzzy::normalize(&raw.prompt)),
                prompt: raw.prompt.trim().to_string(),
                kind,
                category,
                source: Source::Claude,
                note: Some(raw.note.trim().to_string()).filter(|n| !n.is_empty()),
            };
            validate(&question, &none).is_ok().then_some(question)
        })
        .collect())
}

#[derive(Deserialize)]
struct Verdicts {
    results: Vec<Verdict>,
}

#[derive(Deserialize)]
struct Verdict {
    i: usize,
    ok: bool,
}

/// Indices the fact-check passed.
pub fn parse_verdicts(reply: &str) -> anyhow::Result<HashSet<usize>> {
    let verdicts: Verdicts = serde_json::from_str(&text(reply)?)?;
    Ok(verdicts
        .results
        .into_iter()
        .filter(|v| v.ok)
        .map(|v| v.i)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(json: &str) -> String {
        serde_json::to_string(&json!({
            "content": [{ "type": "thinking", "thinking": "" }, { "type": "text", "text": json }],
            "stop_reason": "end_turn",
        }))
        .unwrap()
    }

    #[test]
    fn parses_good_questions_and_drops_bad_ones() {
        let body = json!({ "questions": [
            { "prompt": "Which planet has the moon Triton?", "answer": "Neptune",
              "wrong": ["Uranus", "Saturn", "Jupiter"], "also_accept": [], "note": "It orbits backwards." },
            { "prompt": "Duplicate wrongs?", "answer": "A", "wrong": ["A", "B", "C"], "also_accept": [], "note": "" },
            { "prompt": "Too few wrongs?", "answer": "A", "wrong": ["B"], "also_accept": [], "note": "" },
        ]});
        let mut rng = StdRng::seed_from_u64(1);
        let questions = parse_questions(
            &reply(&body.to_string()),
            Category::Space,
            Wanted::Choice,
            &mut rng,
        )
        .unwrap();
        assert_eq!(questions.len(), 1);
        assert_eq!(questions[0].truth(), "Neptune");
        assert_eq!(questions[0].source, Source::Claude);
        assert_eq!(questions[0].note.as_deref(), Some("It orbits backwards."));
    }

    #[test]
    fn blanks_need_a_gap() {
        let body = json!({ "questions": [
            { "prompt": "The tallest moon mountain is on ____.", "answer": "Iapetus",
              "wrong": ["Rhea", "Io", "Titan"], "also_accept": [], "note": "" },
            { "prompt": "No gap here", "answer": "X", "wrong": ["Y", "Z"], "also_accept": [], "note": "" },
        ]});
        let mut rng = StdRng::seed_from_u64(1);
        let questions = parse_questions(
            &reply(&body.to_string()),
            Category::Space,
            Wanted::Blank,
            &mut rng,
        )
        .unwrap();
        assert_eq!(questions.len(), 1);
    }

    #[test]
    fn broken_replies_are_errors() {
        let mut rng = StdRng::seed_from_u64(1);
        let truncated = serde_json::to_string(&json!({
            "content": [{ "type": "text", "text": "{\"questions\": [{\"prompt\": \"Wh" }],
            "stop_reason": "max_tokens",
        }))
        .unwrap();
        assert!(parse_questions(&truncated, Category::Space, Wanted::Choice, &mut rng).is_err());
        assert!(parse_questions("not json", Category::Space, Wanted::Choice, &mut rng).is_err());
        assert!(
            parse_questions(
                &reply("{\"nope\": 1}"),
                Category::Space,
                Wanted::Choice,
                &mut rng
            )
            .is_err()
        );
        let refused = r#"{"content": [], "stop_reason": "refusal"}"#;
        assert!(parse_verdicts(refused).is_err());
    }

    #[test]
    fn verdicts_keep_only_ok() {
        let body = json!({ "results": [
            { "i": 0, "ok": true, "why": "" }, { "i": 1, "ok": false, "why": "two answers" }, { "i": 2, "ok": true, "why": "" },
        ]});
        assert_eq!(
            parse_verdicts(&reply(&body.to_string())).unwrap(),
            HashSet::from([0, 2])
        );
    }

    #[test]
    fn requests_use_structured_output_and_no_temperature() {
        let request = generate_request(Category::Capitals, Wanted::Choice, "history");
        assert_eq!(request["model"], GENERATE_MODEL);
        assert!(request.get("temperature").is_none());
        assert_eq!(request["output_config"]["format"]["type"], "json_schema");
        assert_eq!(check_request(&[])["model"], CHECK_MODEL);
    }
}
