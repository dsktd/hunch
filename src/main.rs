//! Hunch host. Until the TV screens land this is a console host: it serves phones, prints the
//! join link, a QR code and each phase, and takes host keys on stdin (w/s/a/d, Enter = ok, b = back).

use std::io::BufRead;
use std::io::Write;
use std::sync::mpsc;
use std::time::Duration;
use std::time::Instant;

use hunch::game::Event;
use hunch::game::Game;
use hunch::game::Key;
use hunch::game::MENU;
use hunch::game::Phase;
use hunch::phone::Inbound;
use hunch::phone::Server;
use hunch::protocol;
use hunch::questions::claude::Claude;

fn main() {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let mut game = Game::new(seed, Claude::from_env(seed));
    let Some(mut server) = Server::start() else {
        eprintln!("hunch: phones are needed to play");
        return;
    };
    let keys = stdin_keys();
    let mut sync = protocol::Sync::default();
    let start = Instant::now();
    let mut shown = (u64::MAX, None);
    let mut qr_for = String::new();
    while !game.quit {
        let now = start.elapsed().as_secs_f64();
        if qr_for != server.join.qr_link() {
            qr_for = server.join.qr_link().to_string();
            print_join(&server, &qr_for);
        }
        for inbound in server.poll() {
            match inbound {
                Inbound::Joined { id, name } => {
                    sync.reset(id);
                    if !game.handle(Event::Join { id, name }, now) {
                        server.send(format!(r#"{{"t":"full","id":{id}}}"#));
                    }
                }
                Inbound::Left(id) => {
                    game.handle(Event::Leave { id }, now);
                }
                Inbound::Wire(id, wire) => {
                    if let Some(event) = protocol::event(id, wire) {
                        game.handle(event, now);
                    }
                }
            }
        }
        for key in keys.try_iter() {
            game.handle(Event::Host(key), now);
        }
        game.tick(now);
        for message in sync.views(&game, now) {
            server.send(message);
        }
        let state = (game.seq, game.paused.map(|_| ()));
        if state != shown {
            shown = state;
            print_phase(&game);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn stdin_keys() -> mpsc::Receiver<Key> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            let key = match line.trim() {
                "w" => Key::Up,
                "s" => Key::Down,
                "a" => Key::Left,
                "d" => Key::Right,
                "b" => Key::Back,
                _ => Key::Ok,
            };
            if tx.send(key).is_err() {
                break;
            }
        }
    });
    rx
}

fn print_join(
    server: &Server,
    link: &str,
) {
    let mut out = std::io::stdout().lock();
    if let Some((side, dark)) = hunch::phone::qr(link) {
        // Two modules per character cell, with a quiet zone
        let at = |x: isize, y: isize| {
            x >= 0
                && y >= 0
                && (x as usize) < side
                && (y as usize) < side
                && dark[y as usize * side + x as usize]
        };
        for y in (-2..side as isize + 2).step_by(2) {
            let row: String = (-2..side as isize + 2)
                .map(|x| match (at(x, y), at(x, y + 1)) {
                    (true, true) => ' ',
                    (true, false) => '▄',
                    (false, true) => '▀',
                    (false, false) => '█',
                })
                .collect();
            let _ = writeln!(out, "{row}");
        }
    }
    let _ = writeln!(
        out,
        "Join: {link}\nPIN {}",
        server.join.pin
    );
}

fn print_phase(game: &Game) {
    let players: Vec<String> = game
        .players
        .iter()
        .map(|p| {
            format!(
                "{} {}{}",
                p.name,
                p.score,
                if p.connected { "" } else { " (away)" }
            )
        })
        .collect();
    let line = match &game.phase {
        _ if game.paused.is_some() => "Paused (Enter or b to resume)".to_string(),
        Phase::Lobby => format!(
            "Lobby: {} | menu: {:?} | length {} claude {} filter {}",
            players.join(", "),
            MENU[game.menu],
            game.settings.length.name(),
            game.settings.claude,
            game.settings.family
        ),
        Phase::RoundIntro => {
            let round = game.current_round();
            format!("Round: {} ({})", round.kind.name(), round.kind.rule())
        }
        Phase::CategoryVote { options, .. } => {
            let names: Vec<&str> = options.iter().map(|c| c.name()).collect();
            format!("Vote a category: {}", names.join(" / "))
        }
        Phase::Question { question, .. } | Phase::Write { question, .. } => {
            format!("[{}] {}", question.category.name(), question.prompt)
        }
        Phase::Reveal { question, .. } => format!("Answer: {}", question.truth()),
        Phase::Vote { options, .. } => {
            let texts: Vec<&str> = options.iter().map(|o| o.text.as_str()).collect();
            format!("Which is true? {}", texts.join(" / "))
        }
        Phase::BluffReveal { question, .. } => format!("The truth: {}", question.truth()),
        Phase::Scores | Phase::Podium => format!("Scores: {}", players.join(", ")),
    };
    println!("{line}");
}
