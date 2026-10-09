//! Deterministic SplitMix64 RNG. Its state is part of the world, so a saved and
//! reloaded world continues the same sequence.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng { state: seed }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// Uniform in [0, 1).
    pub fn f32(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Uniform in [lo, hi).
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.f32()
    }

    /// Uniform integer in [lo, hi] (inclusive). Returns `lo` when hi <= lo.
    pub fn range_u32(&mut self, lo: u32, hi: u32) -> u32 {
        if hi <= lo {
            return lo;
        }
        lo + (self.next_u64() % (hi - lo + 1) as u64) as u32
    }

    /// True with probability `p`.
    pub fn chance(&mut self, p: f32) -> bool {
        self.f32() < p
    }

    /// Index picked with probability proportional to its weight. Non-positive weights
    /// are never picked (unless every weight is, then 0).
    pub fn weighted(&mut self, weights: &[f32]) -> usize {
        let total: f32 = weights.iter().filter(|w| **w > 0.0).sum();
        if total <= 0.0 {
            return 0;
        }
        let mut r = self.f32() * total;
        let mut last = 0;
        for (i, &w) in weights.iter().enumerate() {
            if w <= 0.0 {
                continue;
            }
            if r < w {
                return i;
            }
            r -= w;
            last = i;
        }
        last
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        if items.is_empty() {
            None
        } else {
            items.get(self.range_u32(0, items.len() as u32 - 1) as usize)
        }
    }

    /// Fisher-Yates shuffle (for drawing without replacement).
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.range_u32(0, i as u32) as usize;
            items.swap(i, j);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_in_range() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..1000 {
            assert_eq!(a.next_u64(), b.next_u64());
            let f = a.f32();
            b.f32();
            assert!((0.0..1.0).contains(&f));
            let r = a.range_u32(2, 4);
            b.range_u32(2, 4);
            assert!((2..=4).contains(&r));
        }
        assert_ne!(Rng::new(1).next_u64(), Rng::new(2).next_u64());
    }

    #[test]
    fn weighted_respects_zero_weights() {
        let mut r = Rng::new(7);
        let mut hits = [0; 3];
        for _ in 0..3000 {
            hits[r.weighted(&[1.0, 0.0, 3.0])] += 1;
        }
        assert_eq!(hits[1], 0);
        assert!(hits[2] > hits[0] * 2, "{hits:?}");
        assert_eq!(r.weighted(&[0.0, 0.0]), 0);
    }

    #[test]
    fn serde_continues_sequence() {
        let mut a = Rng::new(9);
        a.next_u64();
        let mut b: Rng = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
        assert_eq!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn shuffle_is_permutation() {
        let mut v: Vec<u32> = (0..9).collect();
        Rng::new(3).shuffle(&mut v);
        let mut s = v.clone();
        s.sort();
        assert_eq!(s, (0..9).collect::<Vec<_>>());
    }
}
