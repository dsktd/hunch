//! The match as plain data: players, phases with deadlines, and the rules that move between them.
//! Driven by [`Game::handle`] for phone and host input and [`Game::tick`] for time; no engine here.

use std::collections::BTreeMap;

use rand::Rng;
use rand::SeedableRng;
use rand::rngs::StdRng;
use rand::seq::IndexedRandom;
use rand::seq::SliceRandom;

use crate::filter;
use crate::fuzzy;
use crate::questions::Category;
use crate::questions::Kind;
use crate::questions::Mixer;
use crate::questions::Question;
use crate::questions::Wanted;
use crate::questions::claude::Claude;
use crate::score;

pub const MAX_PLAYERS: usize = 8;
/// Players fewer than this get a warning in the lobby; the game still starts with one.
pub const FEW_PLAYERS: usize = 3;
pub const MAX_NAME: usize = 12;
/// How long a disconnected phone keeps its seat.
pub const REJOIN: f64 = 60.0;
pub const INTRO: f64 = 4.0;
pub const CATEGORY_VOTE: f64 = 12.0;
pub const REVEAL: f64 = 6.0;
pub const WRITE: f64 = 45.0;
pub const VOTE: f64 = 25.0;
pub const BLUFF_STEP: f64 = 3.0;
pub const BLUFF_TRUTH: f64 = 4.0;
pub const SCORES: f64 = 7.0;
/// Fakes plus the truth on a bluff vote; the house tops up with decoys.
pub const MIN_BLUFF_OPTIONS: usize = 3;

#[derive(Clone, Debug, PartialEq)]
pub struct Player {
    pub id: u32,
    pub name: String,
    /// Index into the palette.
    pub colour: usize,
    pub score: u32,
    pub streak: u32,
    pub connected: bool,
    gone_at: Option<f64>,
    /// Points from the last question, for the reveal.
    pub gained: u32,
    /// Right or wrong on the last question; None when they did not answer.
    pub correct: Option<bool>,
    /// Players their last fake fooled.
    pub fooled: u32,
    /// What the phone should tell its player about their last action.
    pub notice: Option<Notice>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notice {
    /// The fake is the truth, or nearly.
    TooClose,
    /// Empty, or no letters at all.
    Rejected,
    /// Voted for their own fake.
    OwnFake,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Length {
    Short,
    Normal,
    Long,
}
impl Length {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Short => "Short",
            Self::Normal => "Normal",
            Self::Long => "Long",
        }
    }

    const fn next(self) -> Self {
        match self {
            Self::Short => Self::Normal,
            Self::Normal => Self::Long,
            Self::Long => Self::Short,
        }
    }

    const fn previous(self) -> Self {
        match self {
            Self::Short => Self::Long,
            Self::Normal => Self::Short,
            Self::Long => Self::Normal,
        }
    }

    pub fn rounds(self) -> Vec<Round> {
        let (choice, bluff) = match self {
            Self::Short => (3, 2),
            Self::Normal => (5, 3),
            Self::Long => (7, 4),
        };
        vec![
            Round {
                kind: RoundKind::Warmup,
                questions: choice,
                multiplier: 1,
                limit: 20.0,
            },
            Round {
                kind: RoundKind::Bluff,
                questions: bluff,
                multiplier: 1,
                limit: WRITE,
            },
            Round {
                kind: RoundKind::Lightning,
                questions: choice,
                multiplier: 2,
                limit: 15.0,
            },
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoundKind {
    Warmup,
    Bluff,
    Lightning,
}
impl RoundKind {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Warmup => "Warm-up",
            Self::Bluff => "Bluff",
            Self::Lightning => "Lightning",
        }
    }

    pub const fn rule(self) -> &'static str {
        match self {
            Self::Warmup => "Pick the right answer. Faster is worth more, streaks add a bonus.",
            Self::Bluff => "Write a fake answer that sounds true, then find the real one.",
            Self::Lightning => "Less time, double points.",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Round {
    pub kind: RoundKind,
    pub questions: u32,
    pub multiplier: u32,
    /// Seconds to answer (to write, for bluffs).
    pub limit: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuItem {
    Start,
    Length,
    Questions,
    Filter,
    Quit,
}
pub const MENU: [MenuItem; 5] = [
    MenuItem::Start,
    MenuItem::Length,
    MenuItem::Questions,
    MenuItem::Filter,
    MenuItem::Quit,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub length: Length,
    /// Questions from Claude; only takes effect when a key is set.
    pub claude: bool,
    /// Mask listed words in players' fakes.
    pub family: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BluffOption {
    pub text: String,
    /// Players who wrote it (two can write the same fake); empty for the truth and house decoys.
    pub authors: Vec<u32>,
    pub truth: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    Lobby,
    RoundIntro,
    CategoryVote {
        options: [Category; 3],
        votes: BTreeMap<u32, usize>,
    },
    Question {
        question: Question,
        /// Choice and the time it came in.
        answers: BTreeMap<u32, (usize, f64)>,
    },
    Reveal {
        question: Question,
        answers: BTreeMap<u32, (usize, f64)>,
    },
    Write {
        question: Question,
        fakes: BTreeMap<u32, String>,
    },
    Vote {
        question: Question,
        options: Vec<BluffOption>,
        votes: BTreeMap<u32, usize>,
    },
    BluffReveal {
        question: Question,
        options: Vec<BluffOption>,
        votes: BTreeMap<u32, usize>,
        /// Options in reveal order, the truth last.
        order: Vec<usize>,
        step: usize,
    },
    Scores,
    Podium,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Ok,
    Back,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Join {
        id: u32,
        name: String,
    },
    Leave {
        id: u32,
    },
    Rename {
        id: u32,
        name: String,
    },
    /// `q` is the phase sequence the phone saw, so a late tap never lands on the next question.
    Answer {
        id: u32,
        q: u64,
        i: usize,
    },
    Write {
        id: u32,
        q: u64,
        text: String,
    },
    Vote {
        id: u32,
        q: u64,
        i: usize,
    },
    /// The party lead starts (or restarts) from their phone.
    Start {
        id: u32,
    },
    Host(Key),
}

pub struct Game {
    pub phase: Phase,
    pub players: Vec<Player>,
    pub settings: Settings,
    /// Focused lobby menu item.
    pub menu: usize,
    pub round: usize,
    /// Questions asked so far in this round.
    pub asked: u32,
    /// Bumps on every phase change.
    pub seq: u64,
    pub since: f64,
    pub deadline: Option<f64>,
    pub category: Category,
    pub paused: Option<f64>,
    /// The host chose Quit.
    pub quit: bool,
    pub rounds: Vec<Round>,
    pub mixer: Mixer,
    claude_available: bool,
    rng: StdRng,
}
impl Game {
    pub fn new(
        seed: u64,
        claude: Option<Claude>,
    ) -> Self {
        let claude_available = claude.is_some();
        let settings = Settings {
            length: Length::Normal,
            claude: claude_available,
            family: false,
        };
        Self {
            phase: Phase::Lobby,
            players: Vec::new(),
            settings,
            menu: 0,
            round: 0,
            asked: 0,
            seq: 0,
            since: 0.0,
            deadline: None,
            category: Category::Capitals,
            paused: None,
            quit: false,
            rounds: settings.length.rounds(),
            mixer: Mixer::new(seed, claude),
            claude_available,
            rng: StdRng::seed_from_u64(seed ^ 0x5eed),
        }
    }

    pub const fn claude_available(&self) -> bool {
        self.claude_available
    }

    pub fn current_round(&self) -> Round {
        self.rounds[self.round.min(self.rounds.len() - 1)]
    }

    pub fn player(
        &self,
        id: u32,
    ) -> Option<&Player> {
        self.players.iter().find(|p| p.id == id)
    }

    fn player_mut(
        &mut self,
        id: u32,
    ) -> Option<&mut Player> {
        self.players.iter_mut().find(|p| p.id == id)
    }

    /// The first connected player, who may start the game from their phone.
    pub fn lead(&self) -> Option<u32> {
        self.players.iter().find(|p| p.connected).map(|p| p.id)
    }

    pub fn connected(&self) -> usize {
        self.players.iter().filter(|p| p.connected).count()
    }

    /// Places by score, ties sharing a place, in `players` order.
    pub fn places(&self) -> Vec<usize> {
        score::places(&self.players.iter().map(|p| p.score).collect::<Vec<_>>())
    }

    /// Seconds left in this phase.
    pub fn left(
        &self,
        now: f64,
    ) -> Option<f64> {
        let now = self.paused.unwrap_or(now);
        self.deadline.map(|d| (d - now).max(0.0))
    }

    /// Phone or host input. Returns false when a join is refused because the game is full.
    pub fn handle(
        &mut self,
        event: Event,
        now: f64,
    ) -> bool {
        match event {
            Event::Join { id, name } => return self.join(id, &name, now),
            Event::Leave { id } => {
                if let Some(player) = self.player_mut(id) {
                    player.connected = false;
                    player.gone_at = Some(now);
                }
            }
            Event::Rename { id, name } => {
                if matches!(self.phase, Phase::Lobby) {
                    let name = self.unique_name(id, &name);
                    if let (Some(name), Some(player)) = (name, self.player_mut(id)) {
                        player.name = name;
                    }
                }
            }
            Event::Answer { id, q, i } => self.answer(id, q, i, now),
            Event::Write { id, q, text } => self.write(id, q, &text),
            Event::Vote { id, q, i } => self.vote(id, q, i),
            Event::Start { id } => {
                if self.lead() == Some(id) && matches!(self.phase, Phase::Lobby | Phase::Podium) {
                    self.start(now);
                }
            }
            Event::Host(key) => self.host(key, now),
        }
        if self.paused.is_none() && self.all_acted() {
            self.next(now);
        }
        true
    }

    /// Time passing: deadlines, and forgetting phones that stayed away.
    pub fn tick(
        &mut self,
        now: f64,
    ) {
        if self.paused.is_some() {
            return;
        }
        if matches!(self.phase, Phase::Lobby | Phase::Podium) {
            self.players
                .retain(|p| p.connected || p.gone_at.is_none_or(|t| now - t <= REJOIN));
        }
        if self.deadline.is_some_and(|d| now >= d) || self.all_acted() {
            self.next(now);
        }
    }

    fn join(
        &mut self,
        id: u32,
        name: &str,
        now: f64,
    ) -> bool {
        let _ = now;
        if let Some(player) = self.player_mut(id) {
            player.connected = true;
            player.gone_at = None;
            return true;
        }
        if self.players.len() >= MAX_PLAYERS {
            return false;
        }
        let name = self
            .unique_name(id, name)
            .unwrap_or_else(|| format!("Player {}", self.players.len() + 1));
        let colour = (0..MAX_PLAYERS)
            .find(|c| !self.players.iter().any(|p| p.colour == *c))
            .unwrap_or(0);
        self.players.push(Player {
            id,
            name,
            colour,
            score: 0,
            streak: 0,
            connected: true,
            gone_at: None,
            gained: 0,
            correct: None,
            fooled: 0,
            notice: None,
        });
        true
    }

    /// A clean name not used by another player: "Sam" becomes "Sam 2" when taken.
    fn unique_name(
        &self,
        id: u32,
        name: &str,
    ) -> Option<String> {
        let base: String = filter::accept(name)?.chars().take(MAX_NAME).collect();
        let base = base.trim().to_string();
        let taken = |n: &str| {
            self.players
                .iter()
                .any(|p| p.id != id && p.name.eq_ignore_ascii_case(n))
        };
        if !taken(&base) {
            return Some(base);
        }
        (2..=MAX_PLAYERS + 1).find_map(|n| {
            let short: String = base.chars().take(MAX_NAME - 2).collect();
            let name = format!("{} {n}", short.trim());
            (!taken(&name)).then_some(name)
        })
    }

    fn host(
        &mut self,
        key: Key,
        now: f64,
    ) {
        if matches!(self.phase, Phase::Lobby) {
            self.lobby_menu(key, now);
            return;
        }
        match (key, &self.phase) {
            (Key::Back, Phase::Podium) => self.reset(),
            (Key::Ok, Phase::Podium) => self.start(now),
            (Key::Back, _) => self.pause(now),
            (Key::Ok, _) if self.paused.is_some() => self.pause(now),
            (Key::Ok, _) => self.next(now),
            _ => {}
        }
    }

    fn lobby_menu(
        &mut self,
        key: Key,
        now: f64,
    ) {
        let item = MENU[self.menu];
        match key {
            Key::Up => self.menu = (self.menu + MENU.len() - 1) % MENU.len(),
            Key::Down => self.menu = (self.menu + 1) % MENU.len(),
            Key::Left | Key::Right | Key::Ok => match item {
                MenuItem::Start if key == Key::Ok => self.start(now),
                MenuItem::Length => {
                    self.settings.length = if key == Key::Left {
                        self.settings.length.previous()
                    } else {
                        self.settings.length.next()
                    };
                }
                MenuItem::Questions => {
                    self.settings.claude = self.claude_available && !self.settings.claude;
                }
                MenuItem::Filter => self.settings.family = !self.settings.family,
                MenuItem::Quit if key == Key::Ok => self.quit = true,
                _ => {}
            },
            Key::Back => {}
        }
    }

    /// Pause or resume; resuming pushes the deadline back by the paused time.
    fn pause(
        &mut self,
        now: f64,
    ) {
        match self.paused.take() {
            Some(at) => {
                if let Some(deadline) = &mut self.deadline {
                    *deadline += now - at;
                }
            }
            None => self.paused = Some(now),
        }
    }

    /// Back to the lobby with scores cleared.
    fn reset(&mut self) {
        for player in &mut self.players {
            player.score = 0;
            player.streak = 0;
            player.gained = 0;
            player.correct = None;
            player.fooled = 0;
        }
        self.round = 0;
        self.asked = 0;
        self.paused = None;
        self.enter(Phase::Lobby, 0.0, None);
    }

    fn start(
        &mut self,
        now: f64,
    ) {
        if self.connected() == 0 {
            return;
        }
        self.reset();
        self.mixer.use_claude = self.settings.claude && self.claude_available;
        self.rounds = self.settings.length.rounds();
        self.enter_round(now);
    }

    fn enter(
        &mut self,
        phase: Phase,
        now: f64,
        duration: Option<f64>,
    ) {
        self.phase = phase;
        self.seq += 1;
        self.since = now;
        self.deadline = duration.map(|d| now + d);
        for player in &mut self.players {
            player.notice = None;
        }
    }

    fn enter_round(
        &mut self,
        now: f64,
    ) {
        self.asked = 0;
        if self.current_round().kind == RoundKind::Bluff {
            let category = self.blank_category();
            self.category = category;
            self.mixer.prepare(category, Wanted::Blank);
        }
        self.enter(Phase::RoundIntro, now, Some(INTRO));
    }

    fn blank_category(&mut self) -> Category {
        let blanks: Vec<Category> = Category::ALL
            .into_iter()
            .filter(|c| c.has_blanks())
            .collect();
        *blanks.choose(&mut self.rng).unwrap_or(&Category::Capitals)
    }

    /// Whether every connected player has done what this phase asks.
    fn all_acted(&self) -> bool {
        let done = |acted: &dyn Fn(u32) -> bool| {
            let mut live = self.players.iter().filter(|p| p.connected).peekable();
            live.peek().is_some() && live.all(|p| acted(p.id))
        };
        match &self.phase {
            Phase::CategoryVote { votes, .. } | Phase::Vote { votes, .. } => {
                done(&|id| votes.contains_key(&id))
            }
            Phase::Question { answers, .. } => done(&|id| answers.contains_key(&id)),
            Phase::Write { fakes, .. } => done(&|id| fakes.contains_key(&id)),
            _ => false,
        }
    }

    /// Leave the current phase for the next one.
    fn next(
        &mut self,
        now: f64,
    ) {
        let phase = std::mem::replace(&mut self.phase, Phase::Lobby);
        match phase {
            Phase::Lobby => self.phase = Phase::Lobby,
            Phase::Podium => self.phase = Phase::Podium,
            Phase::RoundIntro => {
                if self.current_round().kind == RoundKind::Bluff {
                    self.ask(now);
                } else {
                    let mut all = Category::ALL;
                    all.shuffle(&mut self.rng);
                    let options = [all[0], all[1], all[2]];
                    self.enter(
                        Phase::CategoryVote {
                            options,
                            votes: BTreeMap::new(),
                        },
                        now,
                        Some(CATEGORY_VOTE),
                    );
                }
            }
            Phase::CategoryVote { options, votes } => {
                let mut tally = [0usize; 3];
                for choice in votes.values() {
                    tally[*choice] += 1;
                }
                let best = tally.iter().copied().max().unwrap_or(0);
                let top: Vec<usize> = (0..3).filter(|i| tally[*i] == best).collect();
                let pick = *top.choose(&mut self.rng).unwrap_or(&0);
                self.category = options[pick];
                self.mixer.prepare(self.category, Wanted::Choice);
                self.ask(now);
            }
            Phase::Question { question, answers } => {
                self.score_question(&question, &answers);
                self.enter(Phase::Reveal { question, answers }, now, Some(REVEAL));
            }
            Phase::Reveal { .. } => self.after_question(now),
            Phase::Write { question, fakes } => {
                let options = self.bluff_options(&question, &fakes);
                self.enter(
                    Phase::Vote {
                        question,
                        options,
                        votes: BTreeMap::new(),
                    },
                    now,
                    Some(VOTE),
                );
            }
            Phase::Vote {
                question,
                options,
                votes,
            } => {
                self.score_bluff(&options, &votes);
                let votes_on = |at: usize| votes.values().filter(|v| **v == at).count();
                let mut order: Vec<usize> = (0..options.len())
                    .filter(|at| {
                        !options[*at].truth
                            && (!options[*at].authors.is_empty() || votes_on(*at) > 0)
                    })
                    .collect();
                order.extend(options.iter().position(|o| o.truth));
                let step = if order.len() == 1 {
                    BLUFF_TRUTH
                } else {
                    BLUFF_STEP
                };
                self.enter(
                    Phase::BluffReveal {
                        question,
                        options,
                        votes,
                        order,
                        step: 0,
                    },
                    now,
                    Some(step),
                );
            }
            Phase::BluffReveal {
                question,
                options,
                votes,
                order,
                step,
            } => {
                if step + 1 < order.len() {
                    let last = step + 2 == order.len();
                    self.phase = Phase::BluffReveal {
                        question,
                        options,
                        votes,
                        order,
                        step: step + 1,
                    };
                    self.since = now;
                    self.deadline = Some(now + if last { BLUFF_TRUTH } else { BLUFF_STEP });
                } else {
                    self.after_question(now);
                }
            }
            Phase::Scores => {
                if self.round + 1 < self.rounds.len() {
                    self.round += 1;
                    self.enter_round(now);
                } else {
                    self.enter(Phase::Podium, now, None);
                }
            }
        }
    }

    fn ask(
        &mut self,
        now: f64,
    ) {
        let round = self.current_round();
        if round.kind == RoundKind::Bluff {
            if self.asked > 0 {
                self.category = self.blank_category();
            }
            let question = self.mixer.next(self.category, Wanted::Blank);
            self.enter(
                Phase::Write {
                    question,
                    fakes: BTreeMap::new(),
                },
                now,
                Some(round.limit),
            );
        } else {
            let question = self.mixer.next(self.category, Wanted::Choice);
            self.enter(
                Phase::Question {
                    question,
                    answers: BTreeMap::new(),
                },
                now,
                Some(round.limit),
            );
        }
    }

    fn after_question(
        &mut self,
        now: f64,
    ) {
        self.asked += 1;
        if self.asked < self.current_round().questions {
            self.ask(now);
        } else {
            self.enter(Phase::Scores, now, Some(SCORES));
        }
    }

    fn answer(
        &mut self,
        id: u32,
        q: u64,
        i: usize,
        now: f64,
    ) {
        let seq = self.seq;
        let known = self.player(id).is_some();
        if let Phase::Question { answers, .. } = &mut self.phase
            && q == seq
            && i < 4
            && known
        {
            answers.entry(id).or_insert((i, now));
        }
    }

    fn write(
        &mut self,
        id: u32,
        q: u64,
        text: &str,
    ) {
        let seq = self.seq;
        let Phase::Write { question, fakes } = &mut self.phase else {
            return;
        };
        if q != seq || fakes.contains_key(&id) {
            return;
        }
        let notice = match filter::accept(text) {
            None => Some(Notice::Rejected),
            Some(fake) => {
                let close = match &question.kind {
                    Kind::Blank { truth, accept, .. } => {
                        fuzzy::same(&fake, truth) || accept.iter().any(|a| fuzzy::same(&fake, a))
                    }
                    Kind::Choice { .. } => false,
                };
                if close {
                    Some(Notice::TooClose)
                } else {
                    fakes.insert(id, fake);
                    None
                }
            }
        };
        if let Some(player) = self.player_mut(id) {
            player.notice = notice;
        }
    }

    fn vote(
        &mut self,
        id: u32,
        q: u64,
        i: usize,
    ) {
        let seq = self.seq;
        let mut own = false;
        match &mut self.phase {
            Phase::CategoryVote { votes, .. } if q == seq && i < 3 => {
                votes.entry(id).or_insert(i);
            }
            Phase::Vote { options, votes, .. } if q == seq && i < options.len() => {
                if options[i].authors.contains(&id) {
                    own = true;
                } else {
                    votes.entry(id).or_insert(i);
                }
            }
            _ => {}
        }
        if own && let Some(player) = self.player_mut(id) {
            player.notice = Some(Notice::OwnFake);
        }
    }

    fn score_question(
        &mut self,
        question: &Question,
        answers: &BTreeMap<u32, (usize, f64)>,
    ) {
        let Kind::Choice { answer, .. } = question.kind else {
            return;
        };
        let round = self.current_round();
        let since = self.since;
        for player in &mut self.players {
            player.fooled = 0;
            match answers.get(&player.id) {
                Some((choice, at)) if *choice == answer => {
                    player.streak += 1;
                    let left = (round.limit - (at - since)) as f32;
                    player.gained =
                        score::points(left, round.limit as f32, player.streak, round.multiplier);
                    player.correct = Some(true);
                }
                Some(_) => {
                    player.streak = 0;
                    player.gained = 0;
                    player.correct = Some(false);
                }
                None => {
                    player.streak = 0;
                    player.gained = 0;
                    player.correct = None;
                }
            }
            player.score += player.gained;
        }
    }

    /// The truth, every fake (identical ones merged, listed words masked when the filter is on) and
    /// house decoys up to [`MIN_BLUFF_OPTIONS`], shuffled.
    fn bluff_options(
        &mut self,
        question: &Question,
        fakes: &BTreeMap<u32, String>,
    ) -> Vec<BluffOption> {
        let Kind::Blank { truth, decoys, .. } = &question.kind else {
            return Vec::new();
        };
        let mut options = vec![BluffOption {
            text: truth.clone(),
            authors: Vec::new(),
            truth: true,
        }];
        for (id, fake) in fakes {
            let shown = if self.settings.family {
                filter::mask(fake)
            } else {
                fake.clone()
            };
            match options
                .iter_mut()
                .find(|o| !o.truth && fuzzy::same(&o.text, fake))
            {
                Some(option) => option.authors.push(*id),
                None => options.push(BluffOption {
                    text: shown,
                    authors: vec![*id],
                    truth: false,
                }),
            }
        }
        for decoy in decoys {
            if options.len() >= MIN_BLUFF_OPTIONS {
                break;
            }
            if !options.iter().any(|o| fuzzy::same(&o.text, decoy)) {
                options.push(BluffOption {
                    text: decoy.clone(),
                    authors: Vec::new(),
                    truth: false,
                });
            }
        }
        options.shuffle(&mut self.rng);
        options
    }

    fn score_bluff(
        &mut self,
        options: &[BluffOption],
        votes: &BTreeMap<u32, usize>,
    ) {
        let multiplier = self.current_round().multiplier;
        for player in &mut self.players {
            let found = votes.get(&player.id).map(|at| options[*at].truth);
            let fooled = votes
                .iter()
                .filter(|(voter, at)| {
                    **voter != player.id && options[**at].authors.contains(&player.id)
                })
                .count() as u32;
            player.correct = found;
            player.fooled = fooled;
            player.gained = score::bluff(found == Some(true), fooled, multiplier);
            player.score += player.gained;
        }
    }

    /// Random number for screens that want stable variety (quips, jitter seeds).
    pub fn roll(&mut self) -> u32 {
        self.rng.random()
    }
}

/// Drive a game with fake players to a given point; for tests and the screenshot tour.
#[cfg(test)]
mod tests {
    use super::*;

    fn game(players: u32) -> Game {
        let mut game = Game::new(1, None);
        for id in 1..=players {
            assert!(game.handle(
                Event::Join {
                    id,
                    name: format!("P{id}")
                },
                0.0
            ));
        }
        game
    }

    fn ok(
        game: &mut Game,
        now: f64,
    ) {
        game.handle(Event::Host(Key::Ok), now);
    }

    #[test]
    fn joins_names_colours_and_full() {
        let mut game = game(0);
        game.handle(
            Event::Join {
                id: 1,
                name: "Sam".into(),
            },
            0.0,
        );
        game.handle(
            Event::Join {
                id: 2,
                name: "sam".into(),
            },
            0.0,
        );
        game.handle(
            Event::Join {
                id: 3,
                name: "🎉".into(),
            },
            0.0,
        );
        game.handle(
            Event::Join {
                id: 4,
                name: "Wwwwwwwwwwwwwwwwww".into(),
            },
            0.0,
        );
        let names: Vec<&str> = game.players.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Sam", "sam 2", "Player 3", "Wwwwwwwwwwww"]);
        let colours: Vec<usize> = game.players.iter().map(|p| p.colour).collect();
        assert_eq!(colours, [0, 1, 2, 3]);
        for id in 5..=8 {
            assert!(game.handle(
                Event::Join {
                    id,
                    name: String::new()
                },
                0.0
            ));
        }
        assert!(!game.handle(
            Event::Join {
                id: 9,
                name: "late".into()
            },
            0.0
        ));
        assert_eq!(game.players.len(), MAX_PLAYERS);
    }

    #[test]
    fn reconnect_keeps_the_seat_and_lobby_forgets_after_a_while() {
        let mut game = game(2);
        game.handle(Event::Leave { id: 1 }, 1.0);
        game.tick(30.0);
        assert_eq!(game.players.len(), 2);
        game.handle(
            Event::Join {
                id: 1,
                name: "ignored".into(),
            },
            31.0,
        );
        assert!(game.player(1).unwrap().connected);
        assert_eq!(game.player(1).unwrap().name, "P1");
        game.handle(Event::Leave { id: 2 }, 40.0);
        game.tick(40.0 + REJOIN + 1.0);
        assert_eq!(game.players.len(), 1);
    }

    #[test]
    fn plays_a_whole_match() {
        let mut game = game(3);
        game.settings.length = Length::Short;
        let mut now = 0.0;
        ok(&mut game, now);
        assert!(matches!(game.phase, Phase::RoundIntro));
        let mut seen = Vec::new();
        for _ in 0..200 {
            now += 0.5;
            let q = game.seq;
            match &game.phase {
                Phase::CategoryVote { .. } => {
                    for id in 1..=3 {
                        game.handle(Event::Vote { id, q, i: 1 }, now);
                    }
                }
                Phase::Question { question, .. } => {
                    let Kind::Choice { answer, .. } = question.kind else {
                        unreachable!()
                    };
                    game.handle(
                        Event::Answer {
                            id: 1,
                            q,
                            i: answer,
                        },
                        now,
                    );
                    game.handle(
                        Event::Answer {
                            id: 2,
                            q,
                            i: (answer + 1) % 4,
                        },
                        now,
                    );
                    // A stale answer for an old question never counts
                    game.handle(
                        Event::Answer {
                            id: 3,
                            q: q - 1,
                            i: answer,
                        },
                        now,
                    );
                    assert!(matches!(game.phase, Phase::Question { .. }));
                    game.handle(
                        Event::Answer {
                            id: 3,
                            q,
                            i: answer,
                        },
                        now,
                    );
                }
                Phase::Write { question, .. } => {
                    let truth = question.truth().to_string();
                    game.handle(
                        Event::Write {
                            id: 1,
                            q,
                            text: truth.to_uppercase(),
                        },
                        now,
                    );
                    assert_eq!(game.player(1).unwrap().notice, Some(Notice::TooClose));
                    game.handle(
                        Event::Write {
                            id: 1,
                            q,
                            text: "Zzyzx".into(),
                        },
                        now,
                    );
                    game.handle(
                        Event::Write {
                            id: 2,
                            q,
                            text: "  ".into(),
                        },
                        now,
                    );
                    assert_eq!(game.player(2).unwrap().notice, Some(Notice::Rejected));
                    game.handle(
                        Event::Write {
                            id: 2,
                            q,
                            text: "zzyzx".into(),
                        },
                        now,
                    );
                    game.handle(
                        Event::Write {
                            id: 3,
                            q,
                            text: "Qwerty Bay".into(),
                        },
                        now,
                    );
                }
                Phase::Vote { options, .. } => {
                    assert!(options.len() >= MIN_BLUFF_OPTIONS);
                    // Identical fakes merge into one option with both authors
                    assert!(options.iter().any(|o| o.authors == vec![1, 2]));
                    let truth = options.iter().position(|o| o.truth).unwrap();
                    let fake = options
                        .iter()
                        .position(|o| o.authors == vec![1, 2])
                        .unwrap();
                    game.handle(Event::Vote { id: 1, q, i: fake }, now);
                    assert_eq!(game.player(1).unwrap().notice, Some(Notice::OwnFake));
                    game.handle(Event::Vote { id: 1, q, i: truth }, now);
                    game.handle(Event::Vote { id: 2, q, i: truth }, now);
                    game.handle(Event::Vote { id: 3, q, i: fake }, now);
                }
                Phase::Podium => break,
                _ => game.tick(now + 100.0),
            }
            let name = format!("{:?}", game.phase)
                .split([' ', '{'])
                .next()
                .unwrap()
                .to_string();
            if seen.last() != Some(&name) {
                seen.push(name);
            }
        }
        assert!(matches!(game.phase, Phase::Podium));
        for phase in [
            "CategoryVote",
            "Question",
            "Reveal",
            "Write",
            "Vote",
            "BluffReveal",
            "Scores",
            "Podium",
        ] {
            assert!(
                seen.iter().any(|s| s == phase),
                "never saw {phase}: {seen:?}"
            );
        }
        // P1 always right and fooled P3 in bluffs; P2 always wrong but also fooled P3
        let score = |game: &Game, id| game.player(id).unwrap().score;
        assert!(
            score(&game, 1) > score(&game, 3) && score(&game, 3) > score(&game, 2),
            "{:?}",
            game.players
        );
        assert!(score(&game, 2) > 0);
        // Play again from the podium
        ok(&mut game, now);
        assert!(matches!(game.phase, Phase::RoundIntro));
        assert_eq!(score(&game, 1), 0);
    }

    #[test]
    fn deadlines_streaks_and_pause() {
        let mut game = game(1);
        ok(&mut game, 0.0);
        game.tick(INTRO);
        assert!(matches!(game.phase, Phase::CategoryVote { .. }));
        game.tick(INTRO + CATEGORY_VOTE);
        let answer_now = |game: &mut Game, now: f64| {
            let Phase::Question { question, .. } = &game.phase else {
                panic!("{:?}", game.phase)
            };
            let Kind::Choice { answer, .. } = question.kind else {
                unreachable!()
            };
            let q = game.seq;
            game.handle(
                Event::Answer {
                    id: 1,
                    q,
                    i: answer,
                },
                now,
            );
        };
        let start = game.since;
        answer_now(&mut game, start);
        assert!(matches!(game.phase, Phase::Reveal { .. }));
        assert_eq!(game.player(1).unwrap().gained, 1000);
        game.tick(start + REVEAL);
        let start = game.since;
        answer_now(&mut game, start + 10.0);
        // Second in a row: streak bonus, half the speed bonus
        assert_eq!(game.player(1).unwrap().gained, 500 + 250 + 100);
        // Pause holds the deadline
        let at = game.since;
        game.handle(Event::Host(Key::Back), at + 1.0);
        game.tick(at + 100.0);
        assert!(matches!(game.phase, Phase::Reveal { .. }));
        game.handle(Event::Host(Key::Ok), at + 100.0);
        assert_eq!(game.left(at + 100.0), Some(REVEAL - 1.0));
        // Nobody answers: no points, streak broken
        game.tick(at + 200.0);
        let start = game.since;
        game.tick(start + 20.0);
        assert_eq!(game.player(1).unwrap().correct, None);
        assert_eq!(game.player(1).unwrap().streak, 0);
    }

    #[test]
    fn lobby_menu_changes_settings() {
        let mut game = game(1);
        game.handle(Event::Host(Key::Down), 0.0);
        game.handle(Event::Host(Key::Right), 0.0);
        assert_eq!(game.settings.length, Length::Long);
        game.handle(Event::Host(Key::Down), 0.0);
        game.handle(Event::Host(Key::Ok), 0.0);
        assert!(!game.settings.claude, "no key, no Claude");
        game.handle(Event::Host(Key::Down), 0.0);
        game.handle(Event::Host(Key::Ok), 0.0);
        assert!(game.settings.family);
        game.handle(Event::Host(Key::Down), 0.0);
        game.handle(Event::Host(Key::Ok), 0.0);
        assert!(game.quit);
        // Only the lead's phone starts
        game.handle(
            Event::Join {
                id: 2,
                name: "B".into(),
            },
            0.0,
        );
        game.handle(Event::Start { id: 2 }, 0.0);
        assert!(matches!(game.phase, Phase::Lobby));
        game.handle(Event::Start { id: 1 }, 0.0);
        assert!(matches!(game.phase, Phase::RoundIntro));
    }

    #[test]
    fn family_filter_masks_fakes() {
        let mut game = game(2);
        game.settings.family = true;
        let question = game.mixer.next(Category::Capitals, Wanted::Blank);
        let fakes = BTreeMap::from([(1, "holy shit".to_string())]);
        let options = game.bluff_options(&question, &fakes);
        assert!(options.iter().any(|o| o.text == "holy ★★★"));
    }
}
