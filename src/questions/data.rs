//! The bundled fact tables in `data/*.tsv`, made by `tools/refresh-data.py` from Wikidata (CC0).

use std::sync::LazyLock;

#[derive(Clone, Debug)]
pub struct Country {
    pub name: String,
    pub capital: String,
    pub continent: String,
    pub population: u64,
    pub area: u64,
    /// Empty when the country uses several.
    pub currency: String,
}

#[derive(Clone, Debug)]
pub struct Element {
    pub number: u32,
    pub symbol: String,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct Games {
    pub year: u32,
    pub city: String,
    pub country: String,
}

#[derive(Clone, Debug)]
pub struct Moon {
    pub name: String,
    pub planet: String,
}

/// Rows of a table, without the query comments, the header and the Q-id column.
fn rows(table: &'static str) -> impl Iterator<Item = Vec<&'static str>> {
    table
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .skip(1)
        .map(|l| l.split('\t').skip(1).collect())
}

pub static COUNTRIES: LazyLock<Vec<Country>> = LazyLock::new(|| {
    rows(include_str!("../../data/countries.tsv"))
        .filter_map(|r| {
            Some(Country {
                name: (*r.first()?).to_string(),
                capital: (*r.get(1)?).to_string(),
                continent: (*r.get(2)?).to_string(),
                population: r.get(3)?.parse().ok()?,
                area: r.get(4)?.parse().ok()?,
                currency: Some(*r.get(5)?)
                    .filter(|c| *c != "-")
                    .unwrap_or_default()
                    .to_string(),
            })
        })
        .collect()
});

pub static ELEMENTS: LazyLock<Vec<Element>> = LazyLock::new(|| {
    rows(include_str!("../../data/elements.tsv"))
        .filter_map(|r| {
            Some(Element {
                number: r.first()?.parse().ok()?,
                symbol: (*r.get(1)?).to_string(),
                name: capitalise(r.get(2)?),
            })
        })
        .collect()
});

pub static GAMES: LazyLock<Vec<Games>> = LazyLock::new(|| {
    rows(include_str!("../../data/olympics.tsv"))
        .filter_map(|r| {
            Some(Games {
                year: r.first()?.parse().ok()?,
                city: (*r.get(1)?).to_string(),
                country: (*r.get(2)?).to_string(),
            })
        })
        .collect()
});

pub static MOONS: LazyLock<Vec<Moon>> = LazyLock::new(|| {
    rows(include_str!("../../data/moons.tsv"))
        .filter_map(|r| {
            Some(Moon {
                name: (*r.first()?).to_string(),
                planet: (*r.get(1)?).to_string(),
            })
        })
        .collect()
});

/// First letter upper case: "euro" -> "Euro".
pub fn capitalise(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_load() {
        assert!(COUNTRIES.len() > 150);
        assert_eq!(ELEMENTS.len(), 118);
        assert!(GAMES.len() > 25);
        assert!(MOONS.len() > 100);
        assert!(
            COUNTRIES
                .iter()
                .any(|c| c.name == "France" && c.capital == "Paris")
        );
        assert!(
            ELEMENTS
                .iter()
                .any(|e| e.symbol == "W" && e.name == "Tungsten")
        );
        assert!(GAMES.iter().all(|g| g.year != 1916 && g.year != 1944));
    }
}
