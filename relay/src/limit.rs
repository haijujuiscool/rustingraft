//! Bandwidth and per-IP limits.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr};
use std::time::{Duration, Instant};

pub const RATE_BYTES_PER_SEC: usize = 256 * 1024;
pub const BURST_BYTES: usize = 512 * 1024;
pub const JOIN_ATTEMPTS: u32 = 10;
pub const JOIN_WINDOW: Duration = Duration::from_secs(60);
pub const JOIN_BLOCK: Duration = Duration::from_secs(60);
/// IPs an [`AttemptLimiter`] tracks at most. Past that, under a flood from many addresses, new
/// addresses are refused until old windows expire: memory and lock time stay bounded.
pub const MAX_TRACKED: usize = 16_384;
const PRUNE_EVERY: Duration = Duration::from_secs(1);

/// Per-connection token bucket: [`RATE_BYTES_PER_SEC`] sustained, [`BURST_BYTES`] burst.
pub struct TokenBucket {
    tokens: f64,
    last: Instant,
}

impl TokenBucket {
    pub fn new(now: Instant) -> Self {
        Self { tokens: BURST_BYTES as f64, last: now }
    }

    /// Takes `bytes` tokens, or returns false (taking nothing) if there are not enough.
    pub fn take(&mut self, bytes: usize, now: Instant) -> bool {
        let refill = now.duration_since(self.last).as_secs_f64() * RATE_BYTES_PER_SEC as f64;
        self.tokens = (self.tokens + refill).min(BURST_BYTES as f64);
        self.last = now;
        if self.tokens < bytes as f64 {
            return false;
        }
        self.tokens -= bytes as f64;
        true
    }
}

struct Window {
    start: Instant,
    count: u32,
    blocked_until: Option<Instant>,
}

impl Window {
    fn new(now: Instant) -> Self {
        Self { start: now, count: 0, blocked_until: None }
    }

    fn is_live(&self, now: Instant) -> bool {
        self.blocked_until.is_some_and(|until| now < until)
            || now.duration_since(self.start) < JOIN_WINDOW
    }
}

/// Open connections per IP. IPv6 is keyed by /48, the usual site allocation: a single machine
/// often owns a whole /64 or more (a free tunnel broker hands out a /48).
pub struct IpLimiter {
    max_conns_per_ip: usize,
    conns: HashMap<IpAddr, usize>,
}

impl IpLimiter {
    pub fn new(max_conns_per_ip: usize) -> Self {
        Self { max_conns_per_ip, conns: HashMap::new() }
    }

    /// Counts a new connection, or returns false if the IP is at its limit.
    pub fn connect(&mut self, ip: IpAddr) -> bool {
        let open = self.conns.entry(ip_key(ip)).or_default();
        if *open >= self.max_conns_per_ip {
            return false;
        }
        *open += 1;
        true
    }

    pub fn disconnect(&mut self, ip: IpAddr) {
        let key = ip_key(ip);
        if let Some(open) = self.conns.get_mut(&key) {
            *open = open.saturating_sub(1);
            if *open == 0 {
                self.conns.remove(&key);
            }
        }
    }
}

/// Attempts per IP (joins, or room opens). More than [`JOIN_ATTEMPTS`] within
/// [`JOIN_WINDOW`] block the IP for [`JOIN_BLOCK`].
pub struct AttemptLimiter {
    windows: HashMap<IpAddr, Window>,
    last_prune: Instant,
}

impl AttemptLimiter {
    pub fn new(now: Instant) -> Self {
        Self { windows: HashMap::new(), last_prune: now }
    }

    /// Counts an attempt; returns false while the IP is blocked, or when [`MAX_TRACKED`]
    /// other IPs are live and this one is new.
    pub fn attempt(&mut self, ip: IpAddr, now: Instant) -> bool {
        let key = ip_key(ip);
        if self.windows.len() >= MAX_TRACKED && !self.windows.contains_key(&key) {
            if now.duration_since(self.last_prune) >= PRUNE_EVERY {
                self.last_prune = now;
                self.windows.retain(|_, w| w.is_live(now));
            }
            if self.windows.len() >= MAX_TRACKED {
                return false;
            }
        }
        let w = self.windows.entry(key).or_insert_with(|| Window::new(now));
        if !w.is_live(now) {
            *w = Window::new(now);
        } else if w.blocked_until.is_some() {
            return false;
        }
        w.count += 1;
        if w.count > JOIN_ATTEMPTS {
            w.blocked_until = Some(now + JOIN_BLOCK);
            return false;
        }
        true
    }
}

fn ip_key(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => {
                let s = v6.segments();
                IpAddr::V6(Ipv6Addr::new(s[0], s[1], s[2], 0, 0, 0, 0, 0))
            }
        },
        v4 => v4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn bucket_bursts_then_refills_at_the_sustained_rate() {
        let t0 = Instant::now();
        let mut b = TokenBucket::new(t0);
        assert!(b.take(BURST_BYTES, t0));
        assert!(!b.take(1, t0));
        let t1 = t0 + Duration::from_secs(1);
        assert!(b.take(RATE_BYTES_PER_SEC, t1));
        assert!(!b.take(1024, t1));
        let t2 = t1 + Duration::from_secs(60);
        assert!(!b.take(BURST_BYTES + 1, t2), "refill is capped at the burst");
        assert!(b.take(BURST_BYTES, t2));
    }

    #[test]
    fn connections_per_ip() {
        let mut l = IpLimiter::new(2);
        let a = ip("203.0.113.5");
        assert!(l.connect(a) && l.connect(a));
        assert!(!l.connect(a));
        assert!(l.connect(ip("203.0.113.6")));
        l.disconnect(a);
        assert!(l.connect(a));
        assert!(!l.connect(ip("::ffff:203.0.113.5")), "v4-mapped v6 is the same IP");
    }

    #[test]
    fn ipv6_shares_a_slash_48() {
        let mut l = IpLimiter::new(1);
        assert!(l.connect(ip("2001:db8:1:2::1")));
        assert!(!l.connect(ip("2001:db8:1:ffff:ffff::9")), "another /64 of the same /48");
        assert!(l.connect(ip("2001:db8:2::1")));
    }

    #[test]
    fn join_attempts_block_for_a_minute() {
        let t0 = Instant::now();
        let mut l = AttemptLimiter::new(t0);
        let a = ip("198.51.100.7");
        for i in 0..JOIN_ATTEMPTS {
            assert!(l.attempt(a, t0 + Duration::from_secs(i.into())), "attempt {i}");
        }
        let t_block = t0 + Duration::from_secs(20);
        assert!(!l.attempt(a, t_block));
        assert!(l.attempt(ip("198.51.100.8"), t_block), "other IPs unaffected");
        assert!(!l.attempt(a, t_block + Duration::from_secs(59)));
        assert!(l.attempt(a, t_block + JOIN_BLOCK));
    }

    #[test]
    fn join_window_resets() {
        let t0 = Instant::now();
        let mut l = AttemptLimiter::new(t0);
        let a = ip("198.51.100.7");
        for _ in 0..JOIN_ATTEMPTS {
            assert!(l.attempt(a, t0));
        }
        assert!(l.attempt(a, t0 + JOIN_WINDOW));
    }

    #[test]
    fn tracked_ips_are_capped_under_a_flood() {
        let t0 = Instant::now();
        let mut l = AttemptLimiter::new(t0);
        for i in 0..MAX_TRACKED as u32 {
            assert!(l.attempt(IpAddr::from((0x0A00_0000 | i).to_be_bytes()), t0));
        }
        assert!(!l.attempt(ip("198.51.100.7"), t0), "a new IP is refused at the cap");
        assert!(l.attempt(ip("10.0.0.1"), t0), "known IPs keep their own window");
        let later = t0 + JOIN_WINDOW;
        assert!(l.attempt(ip("198.51.100.7"), later), "expired windows are pruned");
        assert_eq!(l.windows.len(), 1);
    }
}
