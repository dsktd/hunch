# Hunch: design

Hunch is a party trivia game. Questions are on the TV and players answer on their phones.
It is an original game: own name, look, round formats and copy. It does not use any names,
characters, art, music or game formats from other party-game series.

Status: approved with review changes (2026-11-14). The non-visual core is implemented first; the
visual sections (9, 10, 14) wait for the UI research (`~/claude/trivia-research/persona-ui.md`)
and the Penpot design.

## 1. Shape

- **Host**: one Rust binary on the big screen (Steam Deck in Game Mode, the settop, a desktop).
- **Phones**: one web page. It is served by the host on the LAN over HTTPS/WebSocket, or from GitHub
  Pages through iroh relays from any network. Phones join by QR code, like Gobble Grand Prix (GGP).
- **Players**: 3 to 8 (1 to 2 allowed for testing, with a warning on the lobby screen).
- **Offline**: the game works fully offline. Claude only adds questions, and only when the host
  sets `ANTHROPIC_API_KEY`.

## 2. Host engine: Bevy 0.20 (2D + UI only)

Chosen over macroquad, egui and a web-tech host:

- The Deck path is already proven in GGP: Game Mode fullscreen (`SteamDeck=1`), gilrs gamepads,
  Wayland, glibc 2.35 via cargo-zigbuild. Reusing it means no new risk tonight.
- GGP's `phone.rs` is written against Bevy resources and systems. It can be copied with small changes.
- `bevy_ui` flexbox covers every screen here: cards, grids, bars and podium. `UiTransform` scale and
  rotation give the pop, shake and bounce animations without a custom renderer.
- Cost: compile time. It is kept down with `default-features = false` and features
  `2d, ui, audio, wav`: no 3D renderer, no glTF.

Everything that has no rendering in it (game rules, scoring, question generation, the phone protocol)
lives in plain-Rust modules. Their unit tests need neither Bevy nor a GPU.

## 3. Crate and module layout

```
Cargo.toml            hunch (bin + lib), lints as strict as GGP's (pedantic, nursery, cargo)
src/main.rs           window setup (fullscreen on Deck), adds hunch::plugin
src/lib.rs            plugin(app), prelude, GameState
src/game.rs           pure state machine: Game, Phase, Event, tick(now) -> no Bevy
src/score.rs          pure scoring: points, streaks, multipliers
src/questions.rs      Question, Kind, Category, Generator trait, validate()
src/questions/offline.rs   procedural generator over bundled data (seeded, no repeats)
src/questions/claude.rs    Claude Messages API generator + fact-check, background thread
src/questions/data.rs      include_str! of data/*.tsv, parsed once
src/phone.rs          LAN HTTPS/WS + iroh endpoint, QR, reconnect (from GGP, attributed)
src/protocol.rs       wire types both ways (serde), version constant, view building
src/screens.rs        TV screens: despawn/respawn on phase change, per-frame timer + animations
src/input.rs          host input: keyboard + gamepad -> MenuInput (up/down/left/right/ok/back)
src/sfx.rs            tiny synthesised sounds (no audio assets): tick, ding, buzz, fanfare
src/shots.rs          `hunch shots`: scripted fake players, saves a screenshot per screen
data/*.tsv            bundled facts (see section 8)
assets/fonts/         Lilita One (titles), Atkinson Hyperlegible (body, legible from the sofa), OFL
assets/phone/index.html    the phone page (one file, no framework)
pad/                  wasm iroh client for the Pages page (from GGP's pad/, attributed), pad-host stand-in
tools/pad_join_test.py     headless Chromium + WebKit join + layout test (from GGP, adapted)
deck-build.sh, deck/  Deck builds (Tally deploys)
```

Copied code (`phone.rs`, `pad/`, `tools/pad_join_test.py`, `deck-build.sh`) carries a header line
`Adapted from Gobble Grand Prix (dsktd/gobble-grand-prix, Unlicense)`. It is also listed in CREDITS.md.

## 4. Game state machine (`src/game.rs`)

`Game` is plain data. It is driven by `Event`s and `tick(now: f64)`. Each phase has a deadline.
A phase ends early once every connected player has acted. A host `Ok` press also skips ahead.

```
Lobby ──start──▶ RoundIntro ──▶ CategoryVote ──▶ Question ──▶ Reveal ──┐
  ▲                 ▲  (MC rounds only)              ▲                 │ more questions
  │                 │                                └─────────────────┘
  │                 │                                                  │ round done
  │                 └──────── Scores ◀─────────────────────────────────┘
  │                              │ (bluff round: RoundIntro ▶ Write ▶ Vote ▶ BluffReveal ▶ … ▶ Scores)
  │                              │ last round done
  └──── play again ◀──────── Podium
```

| Phase         | Time limit                 | Ends early when                    | Host keys                           |
| ------------- | -------------------------- | ---------------------------------- | ----------------------------------- |
| Lobby         | none                       | host presses Start (≥1 player)     | ok start, left/right options        |
| RoundIntro    | 4 s                        | host ok                            | ok skip                             |
| CategoryVote  | 12 s                       | all voted                          | ok skip (ties are broken at random) |
| Question      | 20 s (15 s in Lightning)   | all answered                       | ok skip                             |
| Reveal        | 6 s                        | host ok                            | ok next                             |
| Write (bluff) | 45 s                       | all wrote                          | ok skip                             |
| Vote (bluff)  | 25 s                       | all voted                          | ok skip                             |
| BluffReveal   | 3 s per option + 4 s truth | host ok                            | ok next                             |
| Scores        | 7 s                        | host ok                            | ok next                             |
| Podium        | none                       | host ok = play again; back = lobby | ok, back                            |

Default match (the Lobby can switch it between Short, Normal and Long):

1. **Warm-up**: 5 multiple-choice questions, ×1.
2. **Bluff**: 3 fill-in-the-blank prompts. Players write fake answers, then everyone picks the truth. ×1.
3. **Lightning**: 5 multiple-choice questions, shorter timer, ×2.

Players are held as `Vec<Player { id, name, colour, score, streak, connected }>`, keyed by phone id.
Colours come from a fixed 8-colour palette, first free colour on join. Names are sanitised to
12 characters, and duplicates get a suffix (` 2`).

Reconnect: a phone keeps its seat for 60 s after a disconnect. The seat is keyed by the token the page
stores in localStorage (same mechanism as GGP). A player who left mid-question simply misses that
question. On rejoin, the phone gets the current view straight away. Late joiners are allowed between
questions, and they start at 0 points.

## 5. Scoring (`src/score.rs`)

- Correct multiple-choice answer: `500 + 500 × time_left / time_limit`, rounded to 10.
- **Streak**: the nth correct answer in a row adds `100 × (n−1)`, capped at +400. A wrong answer or
  no answer resets the streak. The TV shows a flame chip from 2 in a row.
- Bluff: finding the truth = 1000. Every player fooled by your fake = 500 to you. If your fake
  is too close to the truth, the phone asks for another one (no points for that). "Too close" is fuzzy
  (`src/fuzzy.rs`): both sides are normalised (case, accents, punctuation, leading articles, spaces),
  numbers are written out and words read as numbers ("7" = "seven", "1,000" = "one thousand"), and
  then an edit distance ≤ 1 counts as a match for up to 6 characters, ≤ 2 beyond that. Any entry in
  the question's `accept` list matches too. Two players' identical fakes merge into one option, and
  both authors score from it.
- Fakes are shown on the TV, so the host rejects any that are empty, whitespace-only or have no
  letter or digit (emoji only, for example). The optional lobby option **Family filter** (off by
  default) masks words from a bundled local word list (`data/filter.txt`) as `★★★`. It works offline.
- The round multiplier applies to all of the above.
- Ties on the podium share a place.

Pure functions: `points(correct, time_left, limit, streak, multiplier) -> u32` and
`bluff_points(...)`. Unit tests cover the boundaries: 0 s left, full time, the streak cap and ties.

## 6. Phone protocol (`src/protocol.rs`)

The transport is unchanged from GGP. LAN: `wss://<ip>:<port>/ws?pin=&token=&name=`. Anywhere:
iroh ALPN `hunch/pad/1`, newline-delimited JSON on one bidirectional stream, and the first line is
`{"t":"join",...}`. The host binds port 7777, falls back through 7778 to 7787, then takes any free port.
Relay URLs drop the trailing dot so iOS WebKit accepts them.

**Versioning**: every host→phone `hello` carries `"v":1`. The page also declares its own
`PROTOCOL = 1`. If they differ, the page shows "Update needed, reload" instead of guessing. Pages
are cache-busted with `?v=<crate version>` in the QR link. The ALPN carries the version too.

Phone → host:

| t        | fields                                 | when                                                                          |
| -------- | -------------------------------------- | ----------------------------------------------------------------------------- |
| `join`   | `pin, token, name, v`                  | first line on iroh (LAN puts these in the query)                              |
| `name`   | `name`                                 | edit name in the lobby                                                        |
| `answer` | `q` (question seq), `i` (choice index) | Question                                                                      |
| `write`  | `q, text` (≤ 40 chars)                 | Write                                                                         |
| `vote`   | `q, i`                                 | Vote, CategoryVote (`q` = vote seq)                                           |
| `start`  | none                                   | lobby, first player only (the "party lead"), when the host is across the room |
| `ping`   | `ms`                                   | every 5 s, for the connection chip                                            |

The host ignores stale `q` values, so a late tap from the previous question never counts.

Host → phone (one view message per phone, sent only when it changes, and again on reconnect):

| t        | fields                                               |
| -------- | ---------------------------------------------------- |
| `hello`  | `id, v`                                              |
| `denied` | none (wrong PIN)                                     |
| `pong`   | `ms`                                                 |
| `view`   | `id, seq, screen, …` (screen-specific fields, below) |
| `buzz`   | `ms` (vibrate on correct/fooled)                     |

`view.screen` is one of `lobby`, `wait`, `category`, `choice`, `answered`, `write`, `written`,
`vote`, `voted`, `result`, `scores`, `podium`. The fields are listed with the phone screens in
section 10. The host builds the views in one pure function, `view_for(game, player) -> View`.
The function is unit-tested, so phone screens never drift from game state.

## 7. Question generators (`src/questions.rs`)

```rust
pub struct Question {
    pub kind: Kind,          // Choice { choices: [String; 4], answer: usize } | Blank { truth: String, accept: Vec<String> }
    pub prompt: String,      // "Which of these has the most people?" / "The chemical symbol for tungsten is ____."
    pub category: Category,
    pub key: String,         // dedupe key, e.g. "capital:France"
    pub source: Source,      // Offline | Claude
    pub note: Option<String>,// one-line fun fact shown on Reveal
}

pub trait Generator: Send {
    fn categories(&self) -> Vec<Category>;
    /// Next question for this category and kind. None when it has run dry.
    fn next(&mut self, category: Category, kind: KindWanted, used: &HashSet<String>) -> Option<Question>;
}
```

Categories (a vote offers 3, drawn at random): Capitals, Big & Small (population/area
comparisons), Elements, When Was That (years), Space, Animals, Languages & Money, Mixed Bag.
The Claude generator adds no categories of its own. It produces questions for the same categories,
so offline fallback always has a match.

**`validate(&Question) -> Result<(), Reject>`** runs on every question from every generator:

- 4 non-empty choices, all different after normalisation (case, accents, punctuation, articles).
- `answer < 4`.
- Prompt and choices within length limits: prompt ≤ 140 chars, choice ≤ 32, so they fit the TV card
  and the phone button.
- The answer text does not appear in the prompt.
- `key` not already used this session.
- Blank: a non-empty truth, and the prompt contains `____`.

**Offline** (`offline.rs`): `StdRng` seeded per session from the clock (`shots` and the tests use a
fixed seed). Templates over the data tables, for example:

- "What is the capital of {country}?" with 3 distractors drawn from capitals on the same continent.
- "Which of these has the most people?" with 4 countries whose populations differ pairwise by ≥ 25%,
  so the answer is never close.
- "Which element has the symbol {sym}?" with distractors whose symbols share a letter.
- "In which year {event}?" with distractors at ±3 to 40 years, rounded to look plausible, never in the future.
- Bluff blanks: "The capital of {obscure country} is ____." and "{element}'s symbol is ____."
- "Which planet / moon …", "Which animal …" from small hand-checked tables.

Same seed and same picks give the same questions (tested). Every generated question passes
`validate` (tested over 10 000 draws per category). Nothing repeats within a session (tested).

**Claude** (`claude.rs`), only if `ANTHROPIC_API_KEY` is set and the lobby option "Questions: Claude"
is on:

- It runs on its own thread with blocking `ureq` (rustls), so the frame loop never waits on it.
- When a category is picked, it asks for a batch of 8 questions for that category and kind: Messages API
  `POST /v1/messages` with `claude-haiku-5-5` (the fast tier, per the claude-api skill, 2026-11).
  Structured output (`output_config.format`, a JSON schema) keeps the reply parseable. There is no
  `temperature`, because current models reject it: variety comes from the prompt (a random sub-topic
  seed plus the list of keys already used). `max_tokens` is bounded.
- JSON output is parsed with serde into the same `Question` shape. Then each question gets
  `validate` plus a **fact-check pass**: a second request to `claude-sonnet-5-5` (the stronger model)
  returns `{"ok":bool,"why":...}` per question, and anything not `ok` is dropped.
- **Cost bound**: at most 12 requests per session (generation plus fact-check), counted in the
  generator. After that it stays offline for the rest of the session. The TV shows a "Claude
  questions" chip whenever the current question came from Claude. Prompts ask for
  well-established facts only: nothing after the model's training data, no opinions, no
  "trick" phrasing.
- **Fallback**: on a timeout (8 s per request), an HTTP error, a parse failure or too few valid
  questions, the offline generator fills the gap for that slot. The TV shows a small "offline
  questions" chip, and the game never stalls. The API key is read from the environment only. It is
  never logged, written, committed or shipped.
- A questions mixer serves Claude's questions first, then offline ones, deduped by `key` and by
  normalised prompt.

## 8. Data sources and licences

- `data/countries.tsv`: country, capital, continent, population (millions, rounded), area km²,
  currency, main language.
- `data/elements.tsv`: number, symbol, name.
- `data/events.tsv`: year, event phrase ("the first person walked on the Moon").
- `data/space.tsv`, `data/animals.tsv`: small hand-checked fact tables.

The tables are not hand-typed: `tools/refresh-data.py` (committed) generates them from the Wikidata
SPARQL endpoint (https://query.wikidata.org/sparql). Each TSV starts with a header comment that holds
the query and the retrieval date, and each row keeps its Q-id. Wikidata is CC0, and the tables are
public domain with the code. After every refresh, 20 rows per table are spot-checked by hand
against their Wikidata pages, and the result is noted in `data/CHECKED.md`. The game uses only rows
that have every field present. Ambiguous entities (disputed capitals, multiple capitals) are dropped
by the script. CREDITS.md
lists Wikidata (CC0), Lilita One and Atkinson Hyperlegible (SIL OFL 1.1), and the GGP-derived code.
No other assets: sounds are synthesised in code.

## 9. TV screens (1280×800 base, scales to 1080p/4K)

Look (Cal): inspired by the Persona 3/4/5 menus. A bold, high-contrast palette, skewed and jagged
polygons, diagonal composition, ransom-note mixed typography, snappy overshoot motion, wipe transitions,
cut-in stamps and staggered entries. All of it is original: own palette, shapes and fonts, and no copied
assets. The palette and visual details are pending the research in `~/claude/trivia-research/persona-ui.md`
and the Penpot design. The screen *contents* below are fixed. Every answer slot has a shape as well as a
colour, so it reads for colour-blind players.

1. **Title / Lobby**: logo "Hunch" with a wobbling question-mark mascot (a simple shape, original),
   QR code (big, left), join URL + PIN, a "Same Wi-Fi? <lan url>" line, and the player grid: up to
   8 colour chips with name and connection dot, empty slots "waiting…". Options column, navigable
   by controller: Start, Length (Short/Normal/Long), Questions (Offline / Claude, greyed without key),
   Quit. Hint bar with controller glyphs. Warning line when under 3 players.
2. **Round intro**: round name (Warm-up / Bluff / Lightning) slams in, a one-line rule explanation, multiplier badge.
3. **Category vote**: 3 big category cards with icons. Players' colour dots stack onto the card they
   voted for. Countdown ring. The winner card zooms in.
4. **Question**: category chip + "Q 3/5", the prompt in a large card, 4 answer tiles
   (shape + letter + text), timer bar across the bottom (turns red in the last 5 s, ticks),
   a row of player chips that flip to "locked in" when each answers. Answers stay secret.
5. **Reveal**: wrong tiles shake and fade, the right tile bounces and glows. Player chips fly onto
   the tile they picked. "+650" floats up from each correct chip, with a flame for streaks. A random
   quip ("Nobody? Really?", "Unanimous!", "Big brain hours"). Fun-fact line.
6. **Bluff write**: the blank prompt, "Write a fake answer that sounds true", chips flip when written, timer.
7. **Bluff vote**: the prompt plus all options shuffled: the fakes, the truth, and a house fake when
   there are fewer than 3 options. Chips flip on vote.
8. **Bluff reveal**: one option at a time. Who picked it, then "FAKE, by <name>" stamped with
   +500 per fooled player. The truth last, with a golden "TRUTH" stamp and +1000 per finder.
9. **Scores**: animated bar race. Bars grow and re-sort, with the rank change arrows and streak flames.
10. **Podium**: three blocks rise. 1st gets confetti (2D particles), a name for every place, then
    full standings beneath. Options: Play again / Back to lobby.
11. **Overlays**: "Paused" (Start button), "Player disconnected" toast, "offline questions" chip,
    "Loading questions…" spinner when Claude is still working.

Controller navigation on the host: D-pad/left stick moves focus, A confirms, B goes back, Start pauses.
Keyboard: arrows, Enter, Esc, Space.

## 10. Phone screens (portrait first, works in landscape)

The page shares the GGP page's structure (cover, safe-area insets, 56 px targets, connection chip,
the iPad layout checked by the GGP layout test). It gets the same Persona-inspired treatment as the TV:
CSS `clip-path` polygons, skew transforms and keyframe wipes and stamps. Every target stays ≥ 56 px and
inside the safe area. Visuals are pending the research and Penpot, as for the TV. Every screen shows a top bar: the player's colour, name, score and the
connection chip.

| screen                                                | contents                                                                                                                                    |
| ----------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| cover/join                                            | logo, name field (remembered), "Join" button, PIN prompt if missing, status text, LAN link when iroh is slow                                |
| connecting / not found / disconnected / update needed | status text, retry/cancel, LAN link                                                                                                         |
| `lobby`                                               | "You're in!", your colour swatch, editable name, list of joined players, "Start game" for the party lead, "Waiting for the host…" otherwise |
| `wait`                                                | big glyph + line ("Eyes on the TV") for intros, reveals and pauses                                                                          |
| `category`                                            | 3 big category buttons; after tapping: "Voted: Space"                                                                                       |
| `choice`                                              | 4 full-width buttons, shape + letter + text, in the slot colours; timer bar                                                                 |
| `answered`                                            | "Locked in: B" + "Waiting for others"                                                                                                       |
| `write`                                               | prompt, text field (40 chars), "Send fake" button, char counter; "Too close to the truth, try another" error                                |
| `written`                                             | "Your fake: …" + waiting                                                                                                                    |
| `vote`                                                | list of options as buttons (your own fake hidden), timer bar                                                                                |
| `voted`                                               | "You picked: …" + waiting                                                                                                                   |
| `result`                                              | right/wrong banner, points gained, streak count, rank; vibrates                                                                             |
| `scores`                                              | your rank and score, the gap to the player above                                                                                            |
| `podium`                                              | your place in large type, final score, "Play again?" (lead only)                                                                            |

## 14. Rendering (TV)

`bevy_ui` flex handles layout boxes only. Skewed and torn shapes and per-letter ransom text need more:

- **Shapes**: `Mesh2d` polygons (skewed panels, jagged bursts and stamp outlines) built by a small
  `shapes.rs` from point lists, plus one custom `Material2d` (`hunch.wgsl`). It provides a halftone dot
  fill, an outline width, and a wipe mask (a diagonal threshold that animates 0 to 1 for wipe
  transitions).
- **Ransom titles**: `ransom.rs` spawns one `Text2d` entity per glyph. Each glyph gets its own font
  (Lilita One or Atkinson Bold), size, colour block, rotation jitter (±8°) and baseline offset, all from
  a seeded rng so screenshots are stable.
- **Motion**: `tween.rs`, a small own easing library. `bevy_tweening` was checked on crates.io at
  implementation time and is used only if it supports Bevy 0.20. Named curves: `snap` (back-out
  overshoot), `slam` (fast in, small bounce), `wipe` (expo in-out), `jitter` (step noise).
  A `Tween<T>` component animates a field over time, and stagger is a per-child delay.
- **Layout helper**: `layout.rs` places the text boxes for each screen at 1280×800 base units, and
  ports GGP's `fit_text`, which measures text with `ttf-parser` and shrinks it until it fits its box.
  GGP's headless layout overflow test is ported too: no text overflows on any screen, using the longest
  data strings and 12-character names.

## 11. Steam Deck

Same as GGP: `deck-build.sh` (cargo-zigbuild, `x86_64-unknown-linux-gnu.2.35`, `-C target-cpu=znver2`,
its own target dir, `deck-dev` profile), fullscreen 1280×800 when `SteamDeck` is set, and gamepad
navigation through gilrs. Deck deployment is Tally's job: this repo only builds the binary.

## 12. Testing plan

- **Unit** (`cargo test --lib`, no GPU):
  - Generator determinism: same seed gives the same questions.
  - Validation: 10 000 offline draws per category all pass `validate`, with no repeats in a session.
  - `validate` rejects duplicate choices, an answer in the prompt and overlong text.
  - Claude JSON parsing against recorded good, bad and truncated responses, with fallback whenever it
    fails. No network in the tests.
  - Scoring boundaries, the streak cap and ties.
  - State machine: scripted event sequences reach every phase. Early end once all players answered,
    a stale `q` is ignored, a reconnect keeps the seat, and a late joiner enters at 0.
  - `view_for` per phase.
  - Phone port fallback (from GGP).
- **Pad join** (`tools/pad_join_test.py`): `pad-host` stand-in, and the Pages site joins from Chromium and
  WebKit over iroh. Then every phone screen is laid out on the GGP device matrix, checking the safe
  area, 56 px targets and no clipped text, with screenshots as a CI artifact.
- **TV screenshots**: `hunch shots` drives fake players through every screen. Rendered headless under
  cage with RADV, under `flock ~/claude/.services/locks/gpu.lock`, skipped while
  `~/claude/.services/locks/games-running` says desktop. Published with `claude-view publish`.
- **CI** (GitHub Actions, frugal, path-filtered): `fmt --check`, `clippy --all-targets -D warnings`
  with GGP's strict lint set, `cargo test --lib`, and the pad join workflow on pad/page changes. Pages
  publishing goes through `pad/publish.sh`, as in GGP.

## 13. Out of scope for v1 (later)

Music, an audience mode (more than 8 phones voting), custom question packs, extra round types,
Wikidata refresh tooling, localisation.
