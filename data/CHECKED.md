# Spot-checks

After each `tools/refresh-data.py` run, 20 random rows per table are compared by hand with general
reference knowledge and their Wikidata pages. Fixes go into the script (`EXCLUDE`, `OVERRIDE`), never
into the TSVs.

## 2026-11-14

- countries (20 rows): 18 fine. Ethiopia currency label "bir" -> birr; Argentina currency
  "Argentine convertible peso" -> Argentine peso. Côte d'Ivoire capital Yamoussoukro is the official
  capital (kept). Populations are Wikidata's latest; questions only compare countries ≥ 25% apart.
- elements (20 rows): all fine (number, symbol, name).
- olympics (all 31 rows): 1916 Berlin and 1944 London were cancelled, excluded. 1940 and 1972 are
  missing from the query result (no single host city in Wikidata); fine, they are just absent.
- moons (20 rows): all fine; duplicate rows from multiple class paths, now deduplicated.
