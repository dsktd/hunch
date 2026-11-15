//! The TV prototype: a web page the host serves on localhost, a pure view of the game.
//!
//! The host pushes one [`Tv`] state per change over a websocket; the page only renders it and
//! sends host keys back. All rules stay in [`crate::game`], so a native TV can replace the page.

use std::net::SocketAddr;
use std::sync::mpsc;

use axum::Router;
use axum::extract::State as AxumState;
use axum::extract::ws::Message as WsMessage;
use axum::extract::ws::WebSocket;
use axum::extract::ws::WebSocketUpgrade;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::response::Html;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::get;
use futures_util::SinkExt;
use futures_util::StreamExt;
use serde::Serialize;
use tokio::sync::watch;

use crate::game::Game;
use crate::game::Key;
use crate::game::MENU;
use crate::game::MenuItem;
use crate::game::Phase;
use crate::phone::JoinInfo;
use crate::protocol::PALETTE;
use crate::questions::Kind;

/// Localhost port for the TV page; `HUNCH_TV_PORT` overrides it.
pub const PORT: u16 = 7880;
const PAGE: &str = include_str!("../assets/tv/index.html");

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct Tv {
    pub t: &'static str,
    pub seq: u64,
    pub paused: bool,
    pub players: Vec<Seat>,
    pub join: Join,
    pub round: RoundInfo,
    #[serde(flatten)]
    pub screen: Screen,
    pub timer: Option<crate::protocol::Timer>,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct Seat {
    pub id: u32,
    pub name: String,
    pub colour: &'static str,
    pub score: u32,
    pub streak: u32,
    pub connected: bool,
    pub place: usize,
    /// Answered, wrote or voted in this phase.
    pub done: bool,
    pub gained: u32,
    pub correct: Option<bool>,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct Join {
    pub link: String,
    pub pin: String,
    pub qr_side: usize,
    /// QR modules row by row, '1' dark.
    pub qr: String,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct RoundInfo {
    pub n: usize,
    pub of: usize,
    pub name: &'static str,
    pub rule: &'static str,
    pub multiplier: u32,
    pub asked: u32,
    pub questions: u32,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct MenuRow {
    pub label: &'static str,
    pub value: Option<String>,
    pub enabled: bool,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    pub text: String,
    pub truth: bool,
    pub authors: Vec<String>,
    pub voters: Vec<String>,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "screen", rename_all = "lowercase")]
pub enum Screen {
    Lobby {
        menu: Vec<MenuRow>,
        focus: usize,
    },
    Intro,
    Category {
        options: Vec<&'static str>,
        votes: Vec<usize>,
    },
    Question {
        category: &'static str,
        prompt: String,
        choices: Vec<String>,
        /// Set on the reveal.
        answer: Option<usize>,
        /// Players per choice, on the reveal.
        picks: Vec<Vec<String>>,
        note: Option<String>,
    },
    Write {
        category: &'static str,
        prompt: String,
    },
    Vote {
        prompt: String,
        options: Vec<String>,
    },
    Bluff {
        prompt: String,
        /// Revealed so far, the truth last.
        shown: Vec<Shown>,
        note: Option<String>,
    },
    Scores,
    Podium,
}

fn names(
    game: &Game,
    ids: impl Iterator<Item = u32>,
) -> Vec<String> {
    ids.filter_map(|id| game.player(id).map(|p| p.name.clone()))
        .collect()
}

fn menu(game: &Game) -> Vec<MenuRow> {
    let s = game.settings;
    MENU.iter()
        .map(|item| match item {
            MenuItem::Start => MenuRow {
                label: "Start",
                value: None,
                enabled: true,
            },
            MenuItem::Length => MenuRow {
                label: "Length",
                value: Some(s.length.name().to_string()),
                enabled: true,
            },
            MenuItem::Questions => MenuRow {
                label: "Questions",
                value: Some(if s.claude { "Claude" } else { "Offline" }.to_string()),
                enabled: game.claude_available(),
            },
            MenuItem::Filter => MenuRow {
                label: "Family filter",
                value: Some(if s.family { "On" } else { "Off" }.to_string()),
                enabled: true,
            },
            MenuItem::Quit => MenuRow {
                label: "Quit",
                value: None,
                enabled: true,
            },
        })
        .collect()
}

/// What the TV shows right now.
pub fn view(
    game: &Game,
    join: &JoinInfo,
    now: f64,
) -> Tv {
    let places = game.places();
    let done = |id: u32| match &game.phase {
        Phase::CategoryVote { votes, .. } | Phase::Vote { votes, .. } => votes.contains_key(&id),
        Phase::Question { answers, .. } => answers.contains_key(&id),
        Phase::Write { fakes, .. } => fakes.contains_key(&id),
        _ => false,
    };
    let players = game
        .players
        .iter()
        .zip(&places)
        .map(|(p, place)| Seat {
            id: p.id,
            name: p.name.clone(),
            colour: PALETTE[p.colour % PALETTE.len()],
            score: p.score,
            streak: p.streak,
            connected: p.connected,
            place: *place,
            done: done(p.id),
            gained: p.gained,
            correct: p.correct,
        })
        .collect();
    let link = join.qr_link().to_string();
    let (qr_side, dark) = crate::phone::qr(&link).unwrap_or_default();
    let round = game.current_round();
    let screen = match &game.phase {
        Phase::Lobby => Screen::Lobby {
            menu: menu(game),
            focus: game.menu,
        },
        Phase::RoundIntro => Screen::Intro,
        Phase::CategoryVote { options, votes } => Screen::Category {
            options: options.iter().map(|c| c.name()).collect(),
            votes: (0..options.len())
                .map(|i| votes.values().filter(|v| **v == i).count())
                .collect(),
        },
        Phase::Question { question, .. } | Phase::Reveal { question, .. } => {
            let reveal = match &game.phase {
                Phase::Reveal { answers, .. } => Some(answers),
                _ => None,
            };
            let (choices, answer) = match &question.kind {
                Kind::Choice { choices, answer } => (choices.to_vec(), *answer),
                Kind::Blank { truth, .. } => (vec![truth.clone()], 0),
            };
            Screen::Question {
                category: question.category.name(),
                prompt: question.prompt.clone(),
                picks: reveal.map_or_else(Vec::new, |answers| {
                    (0..choices.len())
                        .map(|i| {
                            names(
                                game,
                                answers.iter().filter(|(_, a)| a.0 == i).map(|(id, _)| *id),
                            )
                        })
                        .collect()
                }),
                choices,
                answer: reveal.map(|_| answer),
                note: reveal.and_then(|_| question.note.clone()),
            }
        }
        Phase::Write { question, .. } => Screen::Write {
            category: question.category.name(),
            prompt: question.prompt.clone(),
        },
        Phase::Vote {
            question, options, ..
        } => Screen::Vote {
            prompt: question.prompt.clone(),
            options: options.iter().map(|o| o.text.clone()).collect(),
        },
        Phase::BluffReveal {
            question,
            options,
            votes,
            order,
            step,
        } => Screen::Bluff {
            prompt: question.prompt.clone(),
            shown: order
                .iter()
                .take(step + 1)
                .map(|&i| Shown {
                    text: options[i].text.clone(),
                    truth: options[i].truth,
                    authors: names(game, options[i].authors.iter().copied()),
                    voters: names(
                        game,
                        votes.iter().filter(|(_, v)| **v == i).map(|(id, _)| *id),
                    ),
                })
                .collect(),
            note: (step + 1 >= order.len())
                .then(|| question.note.clone())
                .flatten(),
        },
        Phase::Scores => Screen::Scores,
        Phase::Podium => Screen::Podium,
    };
    let timer = match &game.phase {
        Phase::CategoryVote { .. }
        | Phase::Question { .. }
        | Phase::Write { .. }
        | Phase::Vote { .. } => game
            .left(now)
            .zip(game.deadline.map(|d| d - game.since))
            .map(|(left, limit)| crate::protocol::Timer {
                left_ms: (left * 1000.0) as u64,
                limit_ms: (limit * 1000.0) as u64,
            }),
        _ => None,
    };
    Tv {
        t: "tv",
        seq: game.seq,
        paused: game.paused.is_some(),
        players,
        join: Join {
            link,
            pin: join.pin.clone(),
            qr_side,
            qr: dark.iter().map(|d| if *d { '1' } else { '0' }).collect(),
        },
        round: RoundInfo {
            n: game.round + 1,
            of: game.rounds.len(),
            name: round.kind.name(),
            rule: round.kind.rule(),
            multiplier: round.multiplier,
            asked: game.asked,
            questions: round.questions,
        },
        screen,
        timer,
    }
}

/// A host key from the page: w/a/s/d, ok, back.
pub fn key(text: &str) -> Option<Key> {
    Some(match text.trim() {
        "up" => Key::Up,
        "down" => Key::Down,
        "left" => Key::Left,
        "right" => Key::Right,
        "ok" => Key::Ok,
        "back" => Key::Back,
        _ => return None,
    })
}

/// The TV page server on 127.0.0.1, on its own thread.
pub struct Server {
    pub url: String,
    keys: mpsc::Receiver<Key>,
    state: watch::Sender<String>,
    last: Option<Tv>,
}
impl Server {
    pub fn start() -> Option<Self> {
        let port = std::env::var("HUNCH_TV_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(PORT);
        let listener = std::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port)))
            .or_else(|_| std::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))))
            .map_err(|err| eprintln!("hunch: no TV port: {err}"))
            .ok()?;
        listener.set_nonblocking(true).ok()?;
        let addr = listener.local_addr().ok()?;
        let (tx, keys) = mpsc::channel();
        let (state, rx) = watch::channel(String::new());
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            runtime.block_on(serve(listener, Shared { tx, rx }));
        });
        Some(Self {
            url: format!("http://{addr}/tv"),
            keys,
            state,
            last: None,
        })
    }

    /// Host keys pressed on the page since the last call.
    pub fn keys(&self) -> Vec<Key> {
        self.keys.try_iter().collect()
    }

    /// Tell the page the game is over, so it shows "Game closed" and stops reconnecting.
    pub fn close(&self) {
        let _ = self.state.send(r#"{"screen":"closed"}"#.into());
    }

    /// Push the TV state; sent only when something besides the timer changed, or once a second.
    pub fn show(
        &mut self,
        tv: Tv,
    ) {
        let tick = |t: &Tv| t.timer.map(|t| t.left_ms / 1000);
        let same = self.last.as_ref().is_some_and(|last| {
            Tv {
                timer: None,
                ..last.clone()
            } == Tv {
                timer: None,
                ..tv.clone()
            } && tick(last) == tick(&tv)
        });
        if !same {
            let _ = self
                .state
                .send(serde_json::to_string(&tv).unwrap_or_default());
            self.last = Some(tv);
        }
    }
}

#[derive(Clone)]
struct Shared {
    tx: mpsc::Sender<Key>,
    rx: watch::Receiver<String>,
}

async fn serve(
    listener: std::net::TcpListener,
    shared: Shared,
) {
    let mut app = Router::new().route("/tv", get(|| async { Html(PAGE) }));
    for &(path, kind, bytes) in crate::phone::FILES {
        app = app.route(
            &format!("/tv{path}"),
            get(move || async move { ([(axum::http::header::CONTENT_TYPE, kind)], bytes) }),
        );
    }
    let app = app.route("/tv/ws", get(socket)).with_state(shared);
    let Ok(listener) = tokio::net::TcpListener::from_std(listener) else {
        return;
    };
    if let Err(err) = axum::serve(listener, app).await {
        eprintln!("hunch: TV server stopped: {err}");
    }
}

/// Only the page this host serves may drive it: a browser always sends Origin on a websocket, and it
/// must name the same loopback host the request went to (a page on another site, or a rebound DNS
/// name, would differ).
fn own_origin(headers: &HeaderMap) -> bool {
    let text = |name| headers.get(name).and_then(|v| v.to_str().ok());
    let (Some(origin), Some(host)) = (text("origin"), text("host")) else {
        return false;
    };
    let loopback = host.starts_with("127.0.0.1:") || host.starts_with("localhost:");
    loopback && origin.strip_prefix("http://") == Some(host)
}

async fn socket(
    upgrade: WebSocketUpgrade,
    headers: HeaderMap,
    AxumState(shared): AxumState<Shared>,
) -> Response {
    if !own_origin(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    upgrade
        .max_message_size(256)
        .on_upgrade(move |ws| session(ws, shared))
        .into_response()
}

async fn session(
    ws: WebSocket,
    shared: Shared,
) {
    let (mut sink, mut stream) = ws.split();
    let mut rx = shared.rx.clone();
    rx.mark_changed();
    loop {
        tokio::select! {
            changed = rx.changed() => {
                if changed.is_err() { break; }
                let state = rx.borrow_and_update().clone();
                if !state.is_empty() && sink.send(WsMessage::Text(state.into())).await.is_err() {
                    break;
                }
            }
            message = stream.next() => match message {
                Some(Ok(WsMessage::Text(text))) => {
                    if let Some(key) = key(&text) {
                        let _ = shared.tx.send(key);
                    }
                }
                Some(Ok(_)) => {}
                _ => break,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::Event;

    fn join() -> JoinInfo {
        JoinInfo {
            url: "https://10.0.0.2:7777/?pin=123456".into(),
            anywhere: None,
            pin: "123456".into(),
            lan: "10.0.0.2:7777".parse().unwrap(),
        }
    }

    #[test]
    fn the_tv_follows_the_game() {
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
        let tv = view(&game, &join(), 0.0);
        assert!(matches!(tv.screen, Screen::Lobby { focus: 0, .. }));
        assert_eq!(tv.players.len(), 3);
        assert_eq!(tv.join.qr.len(), tv.join.qr_side * tv.join.qr_side);
        let json = serde_json::to_value(&tv).unwrap();
        assert_eq!(json["screen"], "lobby");
        game.handle(Event::Host(Key::Ok), 0.0);
        assert!(matches!(view(&game, &join(), 0.0).screen, Screen::Intro));
        game.tick(10.0);
        let tv = view(&game, &join(), 10.0);
        assert!(matches!(tv.screen, Screen::Category { ref options, .. } if options.len() == 3));
        assert!(tv.timer.is_some());
    }

    #[test]
    fn only_the_own_page_may_connect() {
        let headers = |origin: Option<&str>, host: &str| {
            let mut h = HeaderMap::new();
            h.insert("host", host.parse().unwrap());
            if let Some(origin) = origin {
                h.insert("origin", origin.parse().unwrap());
            }
            h
        };
        assert!(own_origin(&headers(
            Some("http://127.0.0.1:7880"),
            "127.0.0.1:7880"
        )));
        assert!(!own_origin(&headers(
            Some("http://evil.example"),
            "127.0.0.1:7880"
        )));
        assert!(!own_origin(&headers(
            Some("http://evil.example:7880"),
            "evil.example:7880"
        )));
        assert!(!own_origin(&headers(None, "127.0.0.1:7880")));
    }

    #[test]
    fn page_keys_map_to_host_keys() {
        assert_eq!(key("ok"), Some(Key::Ok));
        assert_eq!(key("left"), Some(Key::Left));
        assert_eq!(key("rm -rf"), None);
    }
}
