//! Brute-force and flood limits for the phone link: PIN lockouts, the seat table and a message rate.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Duration;
use std::time::Instant;

/// Wrong PINs one address may send before it is locked out.
const IP_FAILS: u32 = 5;
/// Wrong PINs from everywhere together before every address is locked out.
const GLOBAL_FAILS: u32 = 20;
const FIRST_LOCKOUT: Duration = Duration::from_secs(30);
const LONGEST_LOCKOUT: Duration = Duration::from_secs(600);
/// Addresses remembered at once; the rest of the table is swept when it fills.
const MAX_ADDRS: usize = 1024;

/// Failures against one limit: past the threshold every further failure doubles the lockout.
struct Strikes {
    fails: u32,
    threshold: u32,
    until: Option<Instant>,
    last: Instant,
}

impl Strikes {
    const fn new(
        threshold: u32,
        now: Instant,
    ) -> Self {
        Self {
            fails: 0,
            threshold,
            until: None,
            last: now,
        }
    }

    fn locked(
        &self,
        now: Instant,
    ) -> bool {
        self.until.is_some_and(|until| until > now)
    }

    fn fail(
        &mut self,
        now: Instant,
    ) {
        self.fails = self.fails.saturating_add(1);
        self.last = now;
        if self.fails >= self.threshold {
            let doublings = (self.fails - self.threshold).min(16);
            let lockout = FIRST_LOCKOUT
                .saturating_mul(1 << doublings)
                .min(LONGEST_LOCKOUT);
            self.until = Some(now + lockout);
        }
    }

    /// Nothing locked and nothing recent enough to matter.
    fn stale(
        &self,
        now: Instant,
    ) -> bool {
        !self.locked(now) && now.duration_since(self.last) > LONGEST_LOCKOUT
    }
}

/// Failed-PIN counting per address and overall.
pub struct PinGuard {
    addrs: HashMap<IpAddr, Strikes>,
    global: Strikes,
}

impl PinGuard {
    pub fn new(now: Instant) -> Self {
        Self {
            addrs: HashMap::new(),
            global: Strikes::new(GLOBAL_FAILS, now),
        }
    }

    /// May this address try a PIN right now? Iroh phones have no address worth trusting: `None`
    /// is held to the global limit only.
    pub fn allowed(
        &self,
        addr: Option<IpAddr>,
        now: Instant,
    ) -> bool {
        !self.global.locked(now)
            && addr.is_none_or(|a| self.addrs.get(&a).is_none_or(|s| !s.locked(now)))
    }

    pub fn failed(
        &mut self,
        addr: Option<IpAddr>,
        now: Instant,
    ) {
        self.global.fail(now);
        let Some(addr) = addr else { return };
        if self.addrs.len() >= MAX_ADDRS {
            self.addrs.retain(|_, s| !s.stale(now));
        }
        if self.addrs.len() < MAX_ADDRS || self.addrs.contains_key(&addr) {
            self.addrs
                .entry(addr)
                .or_insert_with(|| Strikes::new(IP_FAILS, now))
                .fail(now);
        }
    }

    pub fn succeeded(
        &mut self,
        addr: Option<IpAddr>,
    ) {
        if let Some(addr) = addr {
            self.addrs.remove(&addr);
        }
    }
}

struct Seat {
    id: u32,
    live: u32,
    last: u64,
}

/// Phone token to connection id, so a reload gets its old seat back; bounded.
#[derive(Default)]
pub struct Seats {
    map: HashMap<String, Seat>,
    clock: u64,
}

impl Seats {
    /// The id for a token (the same one again after a reconnect), or `None` when every seat is in
    /// use. A new token past `cap` evicts the least recently seen disconnected one.
    pub fn claim(
        &mut self,
        token: &str,
        cap: usize,
        fresh: impl FnOnce() -> u32,
    ) -> Option<u32> {
        if token.is_empty() {
            return Some(fresh());
        }
        self.clock += 1;
        let key: String = token.chars().take(64).collect();
        if let Some(seat) = self.map.get_mut(&key) {
            seat.live += 1;
            seat.last = self.clock;
            return Some(seat.id);
        }
        if self.map.len() >= cap {
            let oldest = self
                .map
                .iter()
                .filter(|(_, s)| s.live == 0)
                .min_by_key(|(_, s)| s.last)
                .map(|(k, _)| k.clone())?;
            self.map.remove(&oldest);
        }
        let id = fresh();
        self.map.insert(
            key,
            Seat {
                id,
                live: 1,
                last: self.clock,
            },
        );
        Some(id)
    }

    /// A connection for this id ended.
    pub fn release(
        &mut self,
        id: u32,
    ) {
        self.clock += 1;
        if let Some(seat) = self.map.values_mut().find(|s| s.id == id) {
            seat.live = seat.live.saturating_sub(1);
            seat.last = self.clock;
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.map.len()
    }
}

/// The largest websocket message or frame the LAN side accepts, the same as the iroh stream's.
pub const MAX_MESSAGE: usize = 65_536;

/// Messages one connection may send per second before it is dropped.
pub const MAX_PER_SECOND: u32 = 60;

/// A one-second window message counter.
pub struct Rate {
    start: Instant,
    count: u32,
}

impl Rate {
    pub const fn new(now: Instant) -> Self {
        Self {
            start: now,
            count: 0,
        }
    }

    /// Counts a message; false once the connection is over its limit.
    pub fn allow(
        &mut self,
        now: Instant,
    ) -> bool {
        if now.duration_since(self.start) >= Duration::from_secs(1) {
            self.start = now;
            self.count = 0;
        }
        self.count += 1;
        self.count <= MAX_PER_SECOND
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(10, 0, 0, 2));
    const B: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(10, 0, 0, 3));

    #[test]
    fn wrong_pins_lock_an_address_out_with_doubling_up_to_a_cap() {
        let t0 = Instant::now();
        let mut guard = PinGuard::new(t0);
        for _ in 0..IP_FAILS - 1 {
            assert!(guard.allowed(Some(A), t0));
            guard.failed(Some(A), t0);
        }
        assert!(guard.allowed(Some(A), t0));
        guard.failed(Some(A), t0);
        assert!(!guard.allowed(Some(A), t0));
        assert!(guard.allowed(Some(B), t0));
        let t1 = t0 + FIRST_LOCKOUT;
        assert!(guard.allowed(Some(A), t1));
        guard.failed(Some(A), t1);
        assert!(!guard.allowed(Some(A), t1 + FIRST_LOCKOUT));
        assert!(guard.allowed(Some(A), t1 + FIRST_LOCKOUT * 2));
        let mut now = t1;
        for _ in 0..30 {
            now += LONGEST_LOCKOUT;
            guard.failed(Some(A), now);
        }
        assert!(!guard.allowed(Some(A), now + LONGEST_LOCKOUT / 2));
        assert!(guard.allowed(Some(A), now + LONGEST_LOCKOUT));
    }

    #[test]
    fn a_right_pin_clears_the_address() {
        let t0 = Instant::now();
        let mut guard = PinGuard::new(t0);
        for _ in 0..IP_FAILS - 1 {
            guard.failed(Some(A), t0);
        }
        guard.succeeded(Some(A));
        guard.failed(Some(A), t0);
        assert!(guard.allowed(Some(A), t0));
    }

    #[test]
    fn many_addresses_hit_the_global_limit() {
        let t0 = Instant::now();
        let mut guard = PinGuard::new(t0);
        for n in 0..GLOBAL_FAILS {
            let addr = IpAddr::from([10, 1, 0, u8::try_from(n).unwrap()]);
            assert!(guard.allowed(Some(addr), t0));
            guard.failed(Some(addr), t0);
        }
        assert!(!guard.allowed(Some(B), t0));
        assert!(!guard.allowed(None, t0));
        assert!(guard.allowed(None, t0 + FIRST_LOCKOUT));
    }

    #[test]
    fn the_address_table_stays_bounded() {
        let t0 = Instant::now();
        let mut guard = PinGuard::new(t0);
        for n in 0..MAX_ADDRS * 2 {
            let n = u32::try_from(n).unwrap();
            guard.failed(Some(IpAddr::from(n.to_be_bytes())), t0);
        }
        assert!(guard.addrs.len() <= MAX_ADDRS);
    }

    #[test]
    fn the_seat_table_evicts_the_oldest_disconnected_token() {
        let mut seats = Seats::default();
        let mut next = 0;
        let mut fresh = || {
            next += 1;
            next
        };
        let a = seats.claim("a", 2, &mut fresh).unwrap();
        let b = seats.claim("b", 2, &mut fresh).unwrap();
        assert_eq!(seats.claim("a", 2, &mut fresh), Some(a));
        // Everyone is connected: no seat for a newcomer.
        assert_eq!(seats.claim("c", 2, &mut fresh), None);
        seats.release(a);
        seats.release(a);
        seats.release(b);
        // "a" was released before "b", so "a" goes.
        let c = seats.claim("c", 2, &mut fresh).unwrap();
        assert_eq!(seats.len(), 2);
        assert_eq!(seats.claim("b", 2, &mut fresh), Some(b));
        assert_ne!(c, a);
        for n in 0..50 {
            let _ = seats.claim(&format!("t{n}"), 2, &mut fresh);
        }
        assert!(seats.len() <= 2);
    }

    #[test]
    fn a_connection_over_the_message_rate_is_refused() {
        let t0 = Instant::now();
        let mut rate = Rate::new(t0);
        for _ in 0..MAX_PER_SECOND {
            assert!(rate.allow(t0));
        }
        assert!(!rate.allow(t0));
        assert!(rate.allow(t0 + Duration::from_secs(1)));
    }
}
