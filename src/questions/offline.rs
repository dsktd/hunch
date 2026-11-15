//! Procedural questions over the bundled tables: seeded, so the same seed asks the same questions,
//! with distractors drawn from neighbours (same continent, nearby years, similar symbols).

use std::collections::HashSet;

use rand::Rng;
use rand::SeedableRng;
use rand::rngs::StdRng;
use rand::seq::IndexedRandom;
use rand::seq::SliceRandom;

use super::Category;
use super::GAP;
use super::Generator;
use super::Kind;
use super::Question;
use super::Source;
use super::Wanted;
use super::data;
use super::data::COUNTRIES;
use super::data::Country;
use super::data::ELEMENTS;
use super::data::GAMES;
use super::data::MOONS;
use super::validate;
use crate::fuzzy;

/// Random picks per request before a category counts as run dry.
const ATTEMPTS: usize = 300;

pub struct Offline {
    rng: StdRng,
}
impl Offline {
    pub fn new(seed: u64) -> Self {
        Self {
            rng: StdRng::seed_from_u64(seed),
        }
    }

    fn draw(
        &mut self,
        category: Category,
        wanted: Wanted,
    ) -> Option<Question> {
        let rng = &mut self.rng;
        let variant = rng.random_range(0..3);
        match (category, wanted) {
            (Category::Capitals, Wanted::Choice) => capital(rng, variant % 2 == 0),
            (Category::Capitals, Wanted::Blank) => capital_blank(rng),
            (Category::BigSmall, Wanted::Choice) => big_small(rng, variant),
            (Category::BigSmall, Wanted::Blank) => None,
            (Category::Elements, Wanted::Choice) => element(rng, variant),
            (Category::Elements, Wanted::Blank) => element_blank(rng, variant % 2 == 0),
            (Category::Olympics, Wanted::Choice) => games(rng, variant % 2 == 0),
            (Category::Olympics, Wanted::Blank) => games_blank(rng),
            (Category::Space, Wanted::Choice) => moon(rng, variant % 2 == 0),
            (Category::Space, Wanted::Blank) => moon_blank(rng),
            (Category::Money, Wanted::Choice) => money(rng, variant % 2 == 0),
            (Category::Money, Wanted::Blank) => money_blank(rng),
        }
        .map(|mut question| {
            question.category = category;
            question
        })
    }
}
impl Generator for Offline {
    fn next(
        &mut self,
        category: Category,
        wanted: Wanted,
        used: &HashSet<String>,
    ) -> Option<Question> {
        for _ in 0..ATTEMPTS {
            let Some(question) = self.draw(category, wanted) else {
                continue;
            };
            let seen = used.contains(&format!("prompt:{}", fuzzy::normalize(&question.prompt)));
            if !seen && validate(&question, used).is_ok() {
                return Some(question);
            }
        }
        None
    }
}

/// Four choices with the answer at a random place, or None without three distinct distractors.
fn choice(
    rng: &mut StdRng,
    prompt: String,
    answer: String,
    mut pool: Vec<String>,
    key: String,
    note: Option<String>,
) -> Option<Question> {
    pool.shuffle(rng);
    let mut picked: Vec<String> = Vec::new();
    for candidate in pool {
        let fresh = fuzzy::normalize(&candidate) != fuzzy::normalize(&answer)
            && !picked
                .iter()
                .any(|p| fuzzy::normalize(p) == fuzzy::normalize(&candidate));
        if fresh {
            picked.push(candidate);
        }
        if picked.len() == 3 {
            break;
        }
    }
    if picked.len() < 3 {
        return None;
    }
    let at = rng.random_range(0..4);
    picked.insert(at, answer);
    Some(Question {
        prompt,
        kind: Kind::Choice {
            choices: [
                picked[0].clone(),
                picked[1].clone(),
                picked[2].clone(),
                picked[3].clone(),
            ],
            answer: at,
        },
        category: Category::Capitals,
        key,
        source: Source::Offline,
        note,
    })
}

fn blank(
    rng: &mut StdRng,
    prompt: String,
    truth: String,
    mut decoys: Vec<String>,
    key: String,
) -> Option<Question> {
    decoys.retain(|d| fuzzy::normalize(d) != fuzzy::normalize(&truth));
    decoys.sort();
    decoys.dedup();
    decoys.shuffle(rng);
    decoys.truncate(4);
    (decoys.len() >= 2).then(|| Question {
        prompt,
        kind: Kind::Blank {
            truth,
            accept: Vec::new(),
            decoys,
        },
        category: Category::Capitals,
        key,
        source: Source::Offline,
        note: None,
    })
}

/// Countries other than `country`, from its continent when there are enough.
fn neighbours(country: &Country) -> Vec<&'static Country> {
    let near: Vec<&Country> = COUNTRIES
        .iter()
        .filter(|c| c.continent == country.continent && c.name != country.name)
        .collect();
    if near.len() >= 6 {
        near
    } else {
        COUNTRIES
            .iter()
            .filter(|c| c.name != country.name)
            .collect()
    }
}

fn capital(
    rng: &mut StdRng,
    forward: bool,
) -> Option<Question> {
    let country = COUNTRIES.choose(rng)?;
    let near = neighbours(country);
    let key = format!("capital:{}", country.name);
    if forward {
        let pool = near.iter().map(|c| c.capital.clone()).collect();
        let prompt = format!("What is the capital of {}?", country.name);
        choice(rng, prompt, country.capital.clone(), pool, key, None)
    } else {
        let pool = near.iter().map(|c| c.name.clone()).collect();
        let prompt = format!("{} is the capital of which country?", country.capital);
        choice(rng, prompt, country.name.clone(), pool, key, None)
    }
}

fn capital_blank(rng: &mut StdRng) -> Option<Question> {
    let country = COUNTRIES.choose(rng)?;
    let decoys = neighbours(country)
        .iter()
        .map(|c| c.capital.clone())
        .collect();
    let prompt = format!("The capital of {} is {GAP}.", country.name);
    let key = format!("capital:{}", country.name);
    blank(rng, prompt, country.capital.clone(), decoys, key)
}

/// "1.4 billion", "68 million", "37 thousand".
fn about(n: u64) -> String {
    let (unit, name) = if n >= 1_000_000_000 {
        (1_000_000_000.0, "billion")
    } else if n >= 1_000_000 {
        (1_000_000.0, "million")
    } else {
        (1_000.0, "thousand")
    };
    let value = n as f64 / unit;
    if value >= 10.0 {
        format!("{value:.0} {name}")
    } else {
        format!("{value:.1} {name}")
    }
}

fn big_small(
    rng: &mut StdRng,
    variant: u32,
) -> Option<Question> {
    let by_area = variant == 2 || rng.random_bool(0.3);
    let most = rng.random_bool(0.6);
    let metric = |c: &Country| if by_area { c.area } else { c.population };
    // Four countries, each at least 25% apart from the others, so the answer is never close
    let mut picked: Vec<&Country> = Vec::new();
    let mut pool: Vec<&Country> = COUNTRIES.iter().filter(|c| c.name.len() <= 32).collect();
    pool.shuffle(rng);
    for candidate in pool {
        let apart = picked.iter().all(|p| {
            let (a, b) = (metric(p) as f64, metric(candidate) as f64);
            a.max(b) >= a.min(b) * 1.25
        });
        if apart {
            picked.push(candidate);
        }
        if picked.len() == 4 {
            break;
        }
    }
    if picked.len() < 4 {
        return None;
    }
    let answer = if most {
        picked.iter().max_by_key(|c| metric(c))?
    } else {
        picked.iter().min_by_key(|c| metric(c))?
    };
    let prompt = match (by_area, most) {
        (false, true) => "Which of these countries has the most people?",
        (false, false) => "Which of these countries has the fewest people?",
        (true, true) => "Which of these countries is the largest by area?",
        (true, false) => "Which of these countries is the smallest by area?",
    }
    .to_string();
    let note = if by_area {
        format!("{}: about {} km²", answer.name, about(answer.area))
    } else {
        format!("{}: about {} people", answer.name, about(answer.population))
    };
    let mut names: Vec<&str> = picked.iter().map(|c| c.name.as_str()).collect();
    names.sort_unstable();
    let key = format!("{prompt}:{}", names.join(","));
    let pool = picked
        .iter()
        .filter(|c| c.name != answer.name)
        .map(|c| c.name.clone())
        .collect();
    choice(rng, prompt, answer.name.clone(), pool, key, Some(note))
}

fn element(
    rng: &mut StdRng,
    variant: u32,
) -> Option<Question> {
    let element = ELEMENTS.choose(rng)?;
    let initial = element.symbol.chars().next()?;
    let alike: Vec<&data::Element> = ELEMENTS
        .iter()
        .filter(|e| e.number != element.number && e.symbol.starts_with(initial))
        .collect();
    let others: Vec<&data::Element> = if alike.len() >= 3 {
        alike
    } else {
        ELEMENTS
            .iter()
            .filter(|e| e.number != element.number)
            .collect()
    };
    match variant {
        0 => {
            let pool = others.iter().map(|e| e.name.clone()).collect();
            let prompt = format!("Which element has the chemical symbol {}?", element.symbol);
            let key = format!("element-symbol:{}", element.number);
            let note = format!("{} is element number {}", element.name, element.number);
            choice(rng, prompt, element.name.clone(), pool, key, Some(note))
        }
        1 => {
            let pool = others.iter().map(|e| e.symbol.clone()).collect();
            let prompt = format!("What is the chemical symbol for {}?", element.name);
            let key = format!("element-symbol:{}", element.number);
            choice(rng, prompt, element.symbol.clone(), pool, key, None)
        }
        _ => {
            let pool = ELEMENTS
                .iter()
                .filter(|e| e.number != element.number && e.number.abs_diff(element.number) <= 6)
                .map(|e| e.name.clone())
                .collect();
            let prompt = format!("Which element has atomic number {}?", element.number);
            let key = format!("element-number:{}", element.number);
            let note = format!("Its symbol is {}", element.symbol);
            choice(rng, prompt, element.name.clone(), pool, key, Some(note))
        }
    }
}

fn element_blank(
    rng: &mut StdRng,
    by_number: bool,
) -> Option<Question> {
    let element = ELEMENTS.choose(rng)?;
    let decoys = ELEMENTS
        .iter()
        .filter(|e| e.number.abs_diff(element.number) <= 8 && e.number != element.number)
        .map(|e| e.name.clone())
        .collect();
    let (prompt, key) = if by_number {
        (
            format!(
                "Element number {} on the periodic table is {GAP}.",
                element.number
            ),
            format!("element-number:{}", element.number),
        )
    } else {
        (
            format!("The chemical symbol {} stands for {GAP}.", element.symbol),
            format!("element-symbol:{}", element.number),
        )
    };
    blank(rng, prompt, element.name.clone(), decoys, key)
}

fn host_cities() -> Vec<String> {
    let mut cities: Vec<String> = GAMES.iter().map(|g| g.city.clone()).collect();
    cities.sort();
    cities.dedup();
    cities
}

fn games(
    rng: &mut StdRng,
    by_city: bool,
) -> Option<Question> {
    let pick = GAMES.choose(rng)?;
    if by_city {
        let pool = host_cities()
            .into_iter()
            .filter(|c| *c != pick.city)
            .collect();
        let prompt = format!("Which city hosted the {} Summer Olympics?", pick.year);
        let key = format!("games-city:{}", pick.year);
        let note = format!("{}, {}", pick.city, pick.country);
        return choice(rng, prompt, pick.city.clone(), pool, key, Some(note));
    }
    let hosted: Vec<u32> = GAMES
        .iter()
        .filter(|g| g.city == pick.city)
        .map(|g| g.year)
        .collect();
    let first = *hosted.iter().min()?;
    let pool = GAMES
        .iter()
        .filter(|g| !hosted.contains(&g.year) && g.year.abs_diff(first) <= 28)
        .map(|g| g.year.to_string())
        .collect();
    let prompt = format!(
        "In which year did {} first host the Summer Olympics?",
        pick.city
    );
    let key = format!("games-year:{}", pick.city);
    let note = (hosted.len() > 1).then(|| {
        let years: Vec<String> = hosted.iter().map(u32::to_string).collect();
        format!("{} hosted in {}", pick.city, years.join(", "))
    });
    choice(rng, prompt, first.to_string(), pool, key, note)
}

fn games_blank(rng: &mut StdRng) -> Option<Question> {
    let pick = GAMES.choose(rng)?;
    let decoys = host_cities();
    let prompt = format!("The {} Summer Olympics were held in {GAP}.", pick.year);
    let key = format!("games-city:{}", pick.year);
    blank(rng, prompt, pick.city.clone(), decoys, key)
}

fn planets() -> Vec<String> {
    let mut planets: Vec<String> = MOONS.iter().map(|m| m.planet.clone()).collect();
    planets.sort();
    planets.dedup();
    planets
}

fn moon(
    rng: &mut StdRng,
    which_planet: bool,
) -> Option<Question> {
    let pick = MOONS.choose(rng)?;
    let key = format!("moon:{}", pick.name);
    if which_planet {
        let pool = planets()
            .into_iter()
            .filter(|p| *p != pick.planet)
            .collect();
        let prompt = format!("Which planet does the moon {} orbit?", pick.name);
        return choice(rng, prompt, pick.planet.clone(), pool, key, None);
    }
    let pool = MOONS
        .iter()
        .filter(|m| m.planet != pick.planet)
        .map(|m| m.name.clone())
        .collect();
    let prompt = format!("Which of these is a moon of {}?", pick.planet);
    choice(rng, prompt, pick.name.clone(), pool, key, None)
}

fn moon_blank(rng: &mut StdRng) -> Option<Question> {
    let pick = MOONS.choose(rng)?;
    let decoys = MOONS
        .iter()
        .filter(|m| m.planet != pick.planet)
        .map(|m| m.name.clone())
        .collect();
    let prompt = format!("One of the moons of {} is called {GAP}.", pick.planet);
    let key = format!("moon:{}", pick.name);
    blank(rng, prompt, pick.name.clone(), decoys, key)
}

/// A country with one currency.
fn paying(rng: &mut StdRng) -> Option<&'static Country> {
    COUNTRIES.choose(rng).filter(|c| !c.currency.is_empty())
}

fn money(
    rng: &mut StdRng,
    which_currency: bool,
) -> Option<Question> {
    let country = paying(rng)?;
    let near = neighbours(country);
    let key = format!("currency:{}", country.name);
    if which_currency {
        let pool = near
            .iter()
            .filter(|c| c.currency != country.currency && !c.currency.is_empty())
            .map(|c| data::capitalise(&c.currency))
            .collect();
        let prompt = format!("What is the currency of {}?", country.name);
        let answer = data::capitalise(&country.currency);
        return choice(rng, prompt, answer, pool, key, None);
    }
    // Only currencies one country uses, so the answer is unique
    let users = COUNTRIES
        .iter()
        .filter(|c| c.currency == country.currency)
        .count();
    if users != 1 {
        return None;
    }
    let pool = near.iter().map(|c| c.name.clone()).collect();
    let prompt = format!("Which country uses the {}?", country.currency);
    choice(rng, prompt, country.name.clone(), pool, key, None)
}

fn money_blank(rng: &mut StdRng) -> Option<Question> {
    let country = paying(rng)?;
    let decoys = neighbours(country)
        .iter()
        .filter(|c| !c.currency.is_empty())
        .map(|c| c.currency.clone())
        .collect();
    let prompt = format!("The currency of {} is the {GAP}.", country.name);
    let key = format!("currency:{}", country.name);
    blank(rng, prompt, country.currency.clone(), decoys, key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_questions() {
        let (mut a, mut b) = (Offline::new(42), Offline::new(42));
        let none = HashSet::new();
        for category in Category::ALL {
            for _ in 0..50 {
                assert_eq!(
                    a.next(category, Wanted::Choice, &none),
                    b.next(category, Wanted::Choice, &none)
                );
                assert_eq!(
                    a.next(category, Wanted::Blank, &none),
                    b.next(category, Wanted::Blank, &none)
                );
            }
        }
        assert_ne!(
            Offline::new(1).next(Category::Capitals, Wanted::Choice, &none),
            Offline::new(2).next(Category::Capitals, Wanted::Choice, &none)
        );
    }

    #[test]
    fn every_draw_is_valid() {
        let none = HashSet::new();
        let mut offline = Offline::new(9);
        for category in Category::ALL {
            for _ in 0..10_000 {
                let question = offline.next(category, Wanted::Choice, &none).unwrap();
                assert_eq!(validate(&question, &none), Ok(()), "{question:?}");
                if let Kind::Choice { choices, answer } = &question.kind {
                    assert!(*answer < 4 && !choices[*answer].is_empty());
                }
            }
            if category.has_blanks() {
                for _ in 0..2_000 {
                    let question = offline.next(category, Wanted::Blank, &none).unwrap();
                    assert_eq!(validate(&question, &none), Ok(()), "{question:?}");
                }
            } else {
                assert_eq!(offline.next(category, Wanted::Blank, &none), None);
            }
        }
    }

    #[test]
    fn big_small_answer_is_really_the_extreme() {
        let none = HashSet::new();
        let mut offline = Offline::new(3);
        for _ in 0..500 {
            let q = offline
                .next(Category::BigSmall, Wanted::Choice, &none)
                .unwrap();
            let Kind::Choice { choices, answer } = &q.kind else {
                unreachable!()
            };
            let find = |name: &str| COUNTRIES.iter().find(|c| c.name == name).unwrap();
            let metric = |c: &Country| {
                if q.prompt.contains("area") {
                    c.area
                } else {
                    c.population
                }
            };
            let values: Vec<u64> = choices.iter().map(|c| metric(find(c))).collect();
            let best = if q.prompt.contains("most") || q.prompt.contains("largest") {
                values.iter().max()
            } else {
                values.iter().min()
            };
            assert_eq!(Some(&values[*answer]), best, "{q:?}");
        }
    }
}
