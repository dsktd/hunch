#!/usr/bin/env python3
"""Regenerate data/*.tsv from the Wikidata SPARQL endpoint (CC0).

    tools/refresh-data.py [out dir]

Each file starts with '#' lines holding the query and the retrieval date; rows keep their Q-id.
Rows with a missing field, or entities with several values where one is needed, are dropped here.
"""

import datetime
import json
import pathlib
import sys
import urllib.parse
import urllib.request

# Rows Wikidata gets wrong for a quiz, with the reason; found in the spot-checks (data/CHECKED.md)
EXCLUDE = {
    "Q8123": "1916 Summer Olympics, Berlin: cancelled (First World War)",
    "Q8376": "1944 Summer Olympics, London: cancelled (Second World War)",
}

# Field fixes, also from the spot-checks: Wikidata label or value wrong for today's answer
OVERRIDE = {
    "Q115": {"currency": "birr"},  # label typo "bir"
    "Q414": {"currency": "Argentine peso"},  # sample picked the 1992-era convertible peso
    "Q419": {"currency": "Peruvian sol"},  # "Nuevo sol" was renamed in 2015
    "Q96": {"currency": "Mexican peso"},  # label is the bare "peso"
    "Q686": {"currency": "vatu"},  # label "Vanuatu vatus"
    "Q717": {"currency": "Venezuelan bolívar"},  # label names the 2018 "sovereign" issue
    "Q874": {"currency": "Turkmen manat"},  # label "Turkmenistan new manat"
}

ENDPOINT = "https://query.wikidata.org/sparql"
AGENT = "hunch-data-refresh/0.1 (https://github.com/dsktd/hunch)"

TABLES = {
    # Sovereign states with exactly one capital; currency "-" when they use several (continent: any one of theirs, it only steers distractors)
    "countries": (
        ["qid", "country", "capital", "continent", "population", "area_km2", "currency"],
        """
SELECT ?c ?cLabel (SAMPLE(?capLabel) AS ?capital) (SAMPLE(?contLabel) AS ?continent)
       (MAX(?pop) AS ?population) (MAX(?area) AS ?area_km2) (SAMPLE(?curLabel) AS ?currency)
       (COUNT(DISTINCT ?cap) AS ?ncap) (COUNT(DISTINCT ?cont) AS ?ncont) (COUNT(DISTINCT ?cur) AS ?ncur)
WHERE {
  ?c wdt:P31 wd:Q3624078 ; wdt:P36 ?cap ; wdt:P30 ?cont ; wdt:P1082 ?pop ; wdt:P2046 ?area ; wdt:P38 ?cur .
  FILTER NOT EXISTS { ?c wdt:P576 ?end }
  ?cap rdfs:label ?capLabel . FILTER(LANG(?capLabel) = "en")
  ?cont rdfs:label ?contLabel . FILTER(LANG(?contLabel) = "en")
  ?cur rdfs:label ?curLabel . FILTER(LANG(?curLabel) = "en")
  SERVICE wikibase:label { bd:serviceParam wikibase:language "en". }
}
GROUP BY ?c ?cLabel
HAVING (?ncap = 1)
""",
        lambda b: [b["cLabel"], b["capital"], b["continent"], str(round(float(b["population"]))),
                   str(round(float(b["area_km2"]))), b["currency"] if b["ncur"] == "1" else "-"],
    ),
    "elements": (
        ["qid", "number", "symbol", "element"],
        """
SELECT ?e ?eLabel ?n ?sym WHERE {
  ?e wdt:P31 wd:Q11344 ; wdt:P1086 ?n ; wdt:P246 ?sym .
  FILTER(?n <= 118)
  SERVICE wikibase:label { bd:serviceParam wikibase:language "en". }
}
ORDER BY ?n
""",
        lambda b: [str(int(float(b["n"]))), b["sym"], b["eLabel"]],
    ),
    # Summer Olympic Games editions, host city = the one location that is not a structure (Q811979: stadiums,
    # venues); year from the label
    "olympics": (
        ["qid", "year", "city", "country"],
        """
SELECT ?g ?gLabel (SAMPLE(?cityLabel) AS ?city) (SAMPLE(?ctryLabel) AS ?country)
       (COUNT(DISTINCT ?cityE) AS ?ncity)
WHERE {
  ?g wdt:P31 ?edition ; wdt:P276 ?cityE .
  ?edition rdfs:label "Summer Olympic Games edition"@en .
  ?cityE wdt:P17 ?ctry .
  FILTER NOT EXISTS { ?cityE wdt:P31/wdt:P279* wd:Q811979 }
  ?cityE rdfs:label ?cityLabel . FILTER(LANG(?cityLabel) = "en")
  ?ctry rdfs:label ?ctryLabel . FILTER(LANG(?ctryLabel) = "en")
  ?g rdfs:label ?gLabel . FILTER(LANG(?gLabel) = "en")
  FILTER(REGEX(?gLabel, "^[0-9]{4} Summer Olympics$"))
}
GROUP BY ?g ?gLabel
HAVING (?ncity = 1)
""",
        lambda b: [b["gLabel"][:4], b["city"], b["country"]] if int(b["gLabel"][:4]) <= datetime.date.today().year - 1 else [""],
    ),
    # Named moons of the planets
    "moons": (
        ["qid", "moon", "planet"],
        """
SELECT ?m ?mLabel ?pLabel WHERE {
  VALUES ?p { wd:Q111 wd:Q319 wd:Q193 wd:Q324 wd:Q332 }
  ?m wdt:P31/wdt:P279* wd:Q2537 ; wdt:P397 ?p .
  SERVICE wikibase:label { bd:serviceParam wikibase:language "en". }
}
""",
        lambda b: [b["mLabel"], b["pLabel"]] if not b["mLabel"].startswith(b["pLabel"]) and not any(ch.isdigit() for ch in b["mLabel"]) else [""],
    ),
}


def fetch(query):
    url = ENDPOINT + "?" + urllib.parse.urlencode({"query": query, "format": "json"})
    request = urllib.request.Request(url, headers={"User-Agent": AGENT, "Accept": "application/sparql-results+json"})
    with urllib.request.urlopen(request, timeout=120) as response:
        return json.load(response)["results"]["bindings"]


def main(out):
    out = pathlib.Path(out)
    today = datetime.date.today().isoformat()
    for name, (header, query, row) in TABLES.items():
        rows = []
        for binding in fetch(query):
            values = {k: v["value"] for k, v in binding.items()}
            subject = next(v for k, v in values.items() if v.startswith("http://www.wikidata.org/entity/"))
            try:
                fields = row(values)
            except (KeyError, ValueError):
                continue
            # No unlabelled entities (a bare Q-id as label), no catalogue names like "S/2004 S 12"
            if any(not f or f.startswith("Q") and f[1:].isdigit() or "/" in f or "\t" in f for f in fields):
                continue
            qid = subject.rsplit("/", 1)[1]
            if qid in EXCLUDE:
                continue
            for column, value in OVERRIDE.get(qid, {}).items():
                fields[header.index(column) - 1] = value
            rows.append([qid, *fields])
        rows = [list(r) for r in sorted({tuple(r) for r in rows}, key=lambda r: r[1:])]
        lines = [f"# Wikidata (CC0), {ENDPOINT}, retrieved {today}"]
        lines += ["# " + l for l in query.strip().splitlines()]
        lines.append("\t".join(header))
        lines += ["\t".join(r) for r in rows]
        (out / f"{name}.tsv").write_text("\n".join(lines) + "\n")
        print(f"{name}: {len(rows)} rows")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else pathlib.Path(__file__).parent.parent / "data")
