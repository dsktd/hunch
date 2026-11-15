# Hunch

A party trivia game for the big screen. Questions show on the TV, 3 to 8 players answer on their phones
(scan the QR code, no app), and the questions are made on the fly: offline from bundled facts, or by
Claude when `ANTHROPIC_API_KEY` is set.

Work in progress: the rules, questions, phone link and phone page work. The TV is a web prototype for
now (see DESIGN.md): `cargo run --release`, then open the printed `TV: http://127.0.0.1:7880/tv`
link. Steer with the arrow keys, Enter and Esc (or a gamepad) on that page, or w/s/a/d, Enter and b on
stdin; the console also prints the QR code and each phase.

- `cargo test --lib`: rules, scoring, question generators, protocol.
- `pad/build.sh` then `tools/pad-test.sh`: the phone page joins over iroh and fits every screen;
  `tools/pad-test.sh play`: a whole match with the real host and three headless phones;
  `tools/pad-test.sh tv`: one round driven from the TV page with two headless phones.
- `tools/refresh-data.py`: regenerate the fact tables from Wikidata (see `data/CHECKED.md`).
- `ANTHROPIC_API_KEY` (optional): Claude writes questions too; the game is complete without it.

Public domain (UNLICENSE).
