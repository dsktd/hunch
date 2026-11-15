//! What phones and the host say to each other.
//!
//! Phones send small actions; the host sends each phone one `view` describing its whole screen,
//! built from the game by [`view_for`] and sent only when it changes (and again after a reconnect).

use std::collections::HashMap;

use serde::Deserialize;
use serde::Serialize;

use crate::game::Event;
use crate::game::Game;
use crate::game::Notice;
use crate::game::Phase;
use crate::questions::Kind;

/// Bumped on any incompatible message change; the page refuses a host with another version.
pub const VERSION: u32 = 1;

/// Player colours, by `Player::colour`. Placeholder until the UI design settles the palette.
pub const PALETTE: [&str; 8] = [
    "#ff2d55", "#ffd60a", "#0a84ff", "#30d158", "#ff9f0a", "#bf5af2", "#f2f2f7", "#64d2ff",
];

/// Phone to host.
#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum Wire {
    Name { name: String },
    Answer { q: u64, i: usize },
    Write { q: u64, text: String },
    Vote { q: u64, i: usize },
    Start,
    Ping { ms: f64 },
}

/// The game event for a phone message; pings are answered by the transport and never get here.
pub fn event(
    id: u32,
    wire: Wire,
) -> Option<Event> {
    Some(match wire {
        Wire::Name { name } => Event::Rename { id, name },
        Wire::Answer { q, i } => Event::Answer { id, q, i },
        Wire::Write { q, text } => Event::Write { id, q, text },
        Wire::Vote { q, i } => Event::Vote { id, q, i },
        Wire::Start => Event::Start { id },
        Wire::Ping { .. } => return None,
    })
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub t: &'static str,
    pub id: u32,
    /// The phase sequence; actions echo it back as `q`.
    pub seq: u64,
    pub me: Me,
    pub error: Option<&'static str>,
    #[serde(flatten)]
    pub screen: Screen,
    pub timer: Option<Timer>,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct Me {
    pub name: String,
    pub colour: &'static str,
    pub score: u32,
    pub place: usize,
    pub lead: bool,
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timer {
    pub left_ms: u64,
    pub limit_ms: u64,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct Chip {
    pub name: String,
    pub colour: &'static str,
    pub connected: bool,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct Pick {
    pub i: usize,
    pub text: String,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "screen", rename_all = "lowercase")]
pub enum Screen {
    Lobby {
        players: Vec<Chip>,
        few: bool,
    },
    Wait {
        line: String,
    },
    Category {
        options: Vec<&'static str>,
        voted: Option<usize>,
    },
    Choice {
        prompt: String,
        choices: Vec<String>,
    },
    Answered {
        choice: usize,
        text: String,
    },
    Write {
        prompt: String,
    },
    Written {
        text: String,
    },
    Vote {
        prompt: String,
        options: Vec<Pick>,
    },
    Voted {
        text: String,
    },
    Result {
        correct: Option<bool>,
        gained: u32,
        streak: u32,
        fooled: u32,
        bluff: bool,
    },
    Scores {
        gap: Option<u32>,
        above: Option<String>,
    },
    Podium,
}

const fn notice(notice: Notice) -> &'static str {
    match notice {
        Notice::TooClose => "Too close to the truth! Try another fake.",
        Notice::Rejected => "Write a real word or two.",
        Notice::OwnFake => "That one is yours! Pick another.",
    }
}

fn wait(line: &str) -> Screen {
    Screen::Wait {
        line: line.to_string(),
    }
}

/// The screen for one player's phone right now, or None for an unknown id.
pub fn view_for(
    game: &Game,
    id: u32,
    now: f64,
) -> Option<View> {
    let at = game.players.iter().position(|p| p.id == id)?;
    let player = &game.players[at];
    let places = game.places();
    let me = Me {
        name: player.name.clone(),
        colour: PALETTE[player.colour % PALETTE.len()],
        score: player.score,
        place: places[at],
        lead: game.lead() == Some(id),
    };
    let screen = match &game.phase {
        _ if game.paused.is_some() => wait("Paused"),
        Phase::Lobby => Screen::Lobby {
            players: game
                .players
                .iter()
                .map(|p| Chip {
                    name: p.name.clone(),
                    colour: PALETTE[p.colour % PALETTE.len()],
                    connected: p.connected,
                })
                .collect(),
            few: game.players.len() < crate::game::FEW_PLAYERS,
        },
        Phase::RoundIntro => wait(game.current_round().kind.name()),
        Phase::CategoryVote { options, votes } => Screen::Category {
            options: options.iter().map(|c| c.name()).collect(),
            voted: votes.get(&id).copied(),
        },
        Phase::Question { question, answers } => match (&question.kind, answers.get(&id)) {
            (Kind::Choice { choices, .. }, Some((choice, _))) => Screen::Answered {
                choice: *choice,
                text: choices[*choice].clone(),
            },
            (Kind::Choice { choices, .. }, None) => Screen::Choice {
                prompt: question.prompt.clone(),
                choices: choices.to_vec(),
            },
            (Kind::Blank { .. }, _) => wait("Eyes on the TV"),
        },
        Phase::Write { question, fakes } => fakes.get(&id).map_or_else(
            || Screen::Write {
                prompt: question.prompt.clone(),
            },
            |text| Screen::Written { text: text.clone() },
        ),
        Phase::Vote {
            question,
            options,
            votes,
        } => votes.get(&id).map_or_else(
            || Screen::Vote {
                prompt: question.prompt.clone(),
                options: options
                    .iter()
                    .enumerate()
                    .filter(|(_, o)| !o.authors.contains(&id))
                    .map(|(i, o)| Pick {
                        i,
                        text: o.text.clone(),
                    })
                    .collect(),
            },
            |at| Screen::Voted {
                text: options[*at].text.clone(),
            },
        ),
        Phase::BluffReveal { order, step, .. } if step + 1 < order.len() => wait("Eyes on the TV"),
        Phase::Reveal { .. } | Phase::BluffReveal { .. } => Screen::Result {
            correct: player.correct,
            gained: player.gained,
            streak: player.streak,
            fooled: player.fooled,
            bluff: matches!(game.phase, Phase::BluffReveal { .. }),
        },
        Phase::Scores => {
            let above = game
                .players
                .iter()
                .filter(|p| p.score > player.score)
                .min_by_key(|p| p.score);
            Screen::Scores {
                gap: above.map(|p| p.score - player.score),
                above: above.map(|p| p.name.clone()),
            }
        }
        Phase::Podium => Screen::Podium,
    };
    let timer = match &game.phase {
        Phase::CategoryVote { .. }
        | Phase::Question { .. }
        | Phase::Write { .. }
        | Phase::Vote { .. } => game
            .left(now)
            .zip(game.deadline.map(|d| d - game.since))
            .map(|(left, limit)| Timer {
                left_ms: (left * 1000.0) as u64,
                limit_ms: (limit * 1000.0) as u64,
            }),
        _ => None,
    };
    Some(View {
        t: "view",
        id,
        seq: game.seq,
        me,
        error: player.notice.map(notice),
        screen,
        timer,
    })
}

/// Remembers what each phone was last sent, so a view goes out only when it changed.
#[derive(Default)]
pub struct Sync {
    sent: HashMap<u32, String>,
}
impl Sync {
    /// Forget what a phone has, so it gets its view again (after a reconnect).
    pub fn reset(
        &mut self,
        id: u32,
    ) {
        self.sent.remove(&id);
    }

    /// Messages for every phone whose view changed. The timer alone never counts as a change.
    pub fn views(
        &mut self,
        game: &Game,
        now: f64,
    ) -> Vec<String> {
        let mut messages = Vec::new();
        for player in game.players.iter().filter(|p| p.connected) {
            let Some(view) = view_for(game, player.id, now) else {
                continue;
            };
            let key = serde_json::to_string(&View {
                timer: None,
                ..view.clone()
            })
            .unwrap_or_default();
            if self.sent.get(&player.id) != Some(&key) {
                self.sent.insert(player.id, key);
                messages.push(serde_json::to_string(&view).unwrap_or_default());
            }
        }
        messages
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::Key;

    fn lobby() -> Game {
        let mut game = Game::new(5, None);
        for id in 1..=3 {
            game.handle(
                Event::Join {
                    id,
                    name: format!("P{id}"),
                },
                0.0,
            );
        }
        game
    }

    #[test]
    fn parses_phone_messages() {
        let wire: Wire = serde_json::from_str(r#"{"t":"answer","q":4,"i":2}"#).unwrap();
        assert_eq!(event(7, wire), Some(Event::Answer { id: 7, q: 4, i: 2 }));
        let wire: Wire = serde_json::from_str(r#"{"t":"ping","ms":12.5}"#).unwrap();
        assert_eq!(event(7, wire), None);
        assert!(serde_json::from_str::<Wire>(r#"{"t":"hack"}"#).is_err());
    }

    #[test]
    fn views_follow_the_game() {
        let mut game = lobby();
        let view = view_for(&game, 1, 0.0).unwrap();
        assert!(view.me.lead);
        assert!(
            matches!(view.screen, Screen::Lobby { ref players, few: false } if players.len() == 3)
        );
        assert!(!view_for(&game, 2, 0.0).unwrap().me.lead);
        assert_eq!(view_for(&game, 99, 0.0), None);
        let json = serde_json::to_value(&view).unwrap();
        assert_eq!(json["screen"], "lobby");
        assert_eq!(json["t"], "view");

        game.handle(Event::Host(Key::Ok), 0.0);
        assert!(matches!(
            view_for(&game, 1, 0.0).unwrap().screen,
            Screen::Wait { .. }
        ));
        game.tick(10.0);
        let view = view_for(&game, 1, 11.0).unwrap();
        assert!(matches!(view.screen, Screen::Category { voted: None, .. }));
        assert_eq!(view.timer.unwrap().left_ms, 11_000);
        let q = game.seq;
        game.handle(Event::Vote { id: 1, q, i: 2 }, 11.0);
        assert!(matches!(
            view_for(&game, 1, 11.0).unwrap().screen,
            Screen::Category { voted: Some(2), .. }
        ));
        game.tick(30.0);
        let Screen::Choice { choices, .. } = view_for(&game, 1, 30.0).unwrap().screen else {
            panic!()
        };
        assert_eq!(choices.len(), 4);
        let q = game.seq;
        game.handle(Event::Answer { id: 1, q, i: 0 }, 30.0);
        assert!(matches!(
            view_for(&game, 1, 30.0).unwrap().screen,
            Screen::Answered { choice: 0, .. }
        ));
        game.tick(100.0);
        assert!(matches!(
            view_for(&game, 2, 100.0).unwrap().screen,
            Screen::Result { correct: None, .. }
        ));
    }

    #[test]
    fn sync_sends_changes_only() {
        let mut game = lobby();
        let mut sync = Sync::default();
        assert_eq!(sync.views(&game, 0.0).len(), 3);
        assert_eq!(sync.views(&game, 1.0), Vec::<String>::new());
        sync.reset(2);
        assert_eq!(sync.views(&game, 1.0).len(), 1);
        game.handle(Event::Host(Key::Ok), 1.0);
        game.tick(10.0);
        assert_eq!(sync.views(&game, 10.0).len(), 3);
        // The timer running down is not a change
        assert_eq!(sync.views(&game, 12.0), Vec::<String>::new());
    }
}
