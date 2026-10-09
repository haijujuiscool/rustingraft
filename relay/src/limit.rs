//! Bandwidth and per-IP limits.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr};
use std::time::{Duration, Instant};

pub const RATE_BYTES_PER_SEC: usize = 256 * 1024;
pub const BURST_BYTES: usize = 512 * 1024;
pub const JOIN_ATTEMPTS: u32 = 10;
pub const JOIN_WINDOW: Duration = Duration::from_secs(60);
pub const JOIN_BLOCK: Duration = Duration::from_secs(60);
// ponytail: pruning is O(n) per attempt once more IPs than this are tracked; only reached
// under a distributed flood, where the home line saturates first.
const PRUNE_ABOVE: usize = 4096;

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

struct JoinWindow {
    start: Instant,
    count: u32,
    blocked_until: Option<Instant>,
}

impl JoinWindow {
    fn new(now: Instant) -> Self {
        Self { start: now, count: 0, blocked_until: None }
    }
}

/// Open connections and join attempts per IP. IPv6 is keyed by /64, since one machine
/// usually owns a whole /64.
pub struct IpLimiter {
    max_conns_per_ip: usize,
    conns: HashMap<IpAddr, usize>,
    joins: HashMap<IpAddr, JoinWindow>,
}

impl IpLimiter {
    pub fn new(max_conns_per_ip: usize) -> Self {
        Self { max_conns_per_ip, conns: HashMap::new(), joins: HashMap::new() }
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

    /// Counts a join attempt. More than [`JOIN_ATTEMPTS`] within [`JOIN_WINDOW`] blocks the
    /// IP for [`JOIN_BLOCK`]; returns false while blocked.
    pub fn join_attempt(&mut self, ip: IpAddr, now: Instant) -> bool {
        if self.joins.len() > PRUNE_ABOVE {
            self.joins.retain(|_, w| {
                w.blocked_until.is_some_and(|until| now < until)
                    || now.duration_since(w.start) < JOIN_WINDOW
            });
        }
        let w = self.joins.entry(ip_key(ip)).or_insert_with(|| JoinWindow::new(now));
        match w.blocked_until {
            Some(until) if now < until => return false,
            Some(_) => *w = JoinWindow::new(now),
            None if now.duration_since(w.start) >= JOIN_WINDOW => *w = JoinWindow::new(now),
            None => {}
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
                IpAddr::V6(Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0))
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
    fn ipv6_shares_a_slash_64() {
        let mut l = IpLimiter::new(1);
        assert!(l.connect(ip("2001:db8:1:2::1")));
        assert!(!l.connect(ip("2001:db8:1:2:ffff::9")));
        assert!(l.connect(ip("2001:db8:1:3::1")));
    }

    #[test]
    fn join_attempts_block_for_a_minute() {
        let mut l = IpLimiter::new(8);
        let a = ip("198.51.100.7");
        let t0 = Instant::now();
        for i in 0..JOIN_ATTEMPTS {
            assert!(l.join_attempt(a, t0 + Duration::from_secs(i.into())), "attempt {i}");
        }
        let t_block = t0 + Duration::from_secs(20);
        assert!(!l.join_attempt(a, t_block));
        assert!(l.join_attempt(ip("198.51.100.8"), t_block), "other IPs unaffected");
        assert!(!l.join_attempt(a, t_block + Duration::from_secs(59)));
        assert!(l.join_attempt(a, t_block + JOIN_BLOCK));
    }

    #[test]
    fn join_window_resets() {
        let mut l = IpLimiter::new(8);
        let a = ip("198.51.100.7");
        let t0 = Instant::now();
        for _ in 0..JOIN_ATTEMPTS {
            assert!(l.join_attempt(a, t0));
        }
        assert!(l.join_attempt(a, t0 + JOIN_WINDOW));
    }
}
