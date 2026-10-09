//! Day/night clock (spec 5). One cycle = 360 s: dawn 0-15, day 15-280, dusk 280-300,
//! night 300-360. The day number increments at dawn.

use crate::content::{CYCLE_SECONDS, DAWN_SECONDS, DAY_SECONDS, DUSK_SECONDS};
use crate::msg::Event;
use crate::world::World;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Dawn,
    Day,
    Dusk,
    Night,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Clock {
    /// Seconds into the current cycle, [0, CYCLE_SECONDS).
    pub time: f32,
    /// Day number, starting at 1.
    pub day: u32,
    /// Total simulated seconds since the world was created. Use it for timestamps
    /// and cooldowns (`World::now`).
    pub elapsed: f64,
}

impl Default for Clock {
    fn default() -> Clock {
        Clock {
            time: 0.0,
            day: 1,
            elapsed: 0.0,
        }
    }
}

pub fn phase_at(time: f32) -> Phase {
    if time < DAWN_SECONDS {
        Phase::Dawn
    } else if time < DAY_SECONDS - DUSK_SECONDS {
        Phase::Day
    } else if time < DAY_SECONDS {
        Phase::Dusk
    } else {
        Phase::Night
    }
}

impl Clock {
    pub fn phase(&self) -> Phase {
        phase_at(self.time)
    }

    pub fn is_night(&self) -> bool {
        self.phase() == Phase::Night
    }

    /// Jumps forward (without events) to the next moment the time of day is `time`.
    /// `time` and `day` derive from `elapsed` on every step, so tests must use this
    /// instead of writing `time` directly.
    pub fn skip_to(&mut self, time: f32) {
        let cycle = CYCLE_SECONDS as f64;
        let start = (self.elapsed / cycle).floor() * cycle;
        let mut t = start + time as f64;
        if t < self.elapsed {
            t += cycle;
        }
        self.elapsed = t;
        self.day = 1 + (t / cycle) as u32;
        self.time = (t % cycle) as f32;
    }

    /// 0 at night, 1 in full day, blending through dawn and dusk.
    pub fn daylight(&self) -> f32 {
        match self.phase() {
            Phase::Dawn => self.time / DAWN_SECONDS,
            Phase::Day => 1.0,
            Phase::Dusk => (DAY_SECONDS - self.time) / DUSK_SECONDS,
            Phase::Night => 0.0,
        }
    }
}

/// True if the clock entered `phase` during this tick. Systems run after the clock,
/// so they can react to phase edges with this.
pub fn phase_entered(ev: &[Event], phase: Phase) -> bool {
    ev.contains(&Event::PhaseChanged(phase))
}

pub(crate) fn step(w: &mut World, dt: f32, ev: &mut Vec<Event>) {
    // Sub-steps no longer than the shortest phase, so a big dt cannot skip a phase.
    let mut left = dt;
    while left > 0.0 {
        let d = left.min(DAWN_SECONDS * 0.5);
        left -= d;
        let c = &mut w.clock;
        let before = c.phase();
        // `elapsed` (f64) is the truth; time and day derive from it so f32 rounding
        // never drifts.
        c.elapsed += d as f64;
        let cycle = CYCLE_SECONDS as f64;
        c.day = 1 + (c.elapsed / cycle) as u32;
        c.time = (c.elapsed % cycle) as f32;
        let after = c.phase();
        if after != before {
            ev.push(Event::PhaseChanged(after));
            match after {
                Phase::Dawn => ev.push(Event::DayStarted(c.day)),
                Phase::Night => ev.push(Event::NightStarted),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phases() {
        assert_eq!(phase_at(0.0), Phase::Dawn);
        assert_eq!(phase_at(15.0), Phase::Day);
        assert_eq!(phase_at(279.9), Phase::Day);
        assert_eq!(phase_at(280.0), Phase::Dusk);
        assert_eq!(phase_at(300.0), Phase::Night);
        assert_eq!(phase_at(359.9), Phase::Night);
        let c = Clock {
            time: 290.0,
            ..Clock::default()
        };
        assert!((c.daylight() - 0.5).abs() < 1e-5);
    }

    #[test]
    fn one_cycle_emits_edges_and_counts_days() {
        let mut w = World::new(1);
        let mut ev = Vec::new();
        for _ in 0..(360 * 60) {
            step(&mut w, 1.0 / 60.0, &mut ev);
        }
        assert_eq!(w.clock.day, 2);
        assert!((w.clock.elapsed - 360.0).abs() < 0.01);
        let phases: Vec<_> = ev
            .iter()
            .filter_map(|e| {
                if let Event::PhaseChanged(p) = e {
                    Some(*p)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            phases,
            vec![Phase::Day, Phase::Dusk, Phase::Night, Phase::Dawn]
        );
        assert!(ev.contains(&Event::NightStarted));
        assert!(ev.contains(&Event::DayStarted(2)));
        assert!(phase_entered(&ev, Phase::Dusk));
    }

    #[test]
    fn skip_to_then_step_into_night() {
        let mut w = World::new(1);
        w.clock.skip_to(299.0);
        assert_eq!((w.clock.day, w.clock.phase()), (1, Phase::Dusk));
        let mut ev = Vec::new();
        step(&mut w, 2.0, &mut ev);
        assert!(w.clock.is_night() && ev.contains(&Event::NightStarted));
        w.clock.skip_to(10.0);
        assert_eq!((w.clock.day, w.clock.phase()), (2, Phase::Dawn));
    }

    #[test]
    fn big_dt_does_not_skip_phases() {
        let mut w = World::new(1);
        let mut ev = Vec::new();
        step(&mut w, 400.0, &mut ev);
        assert_eq!(w.clock.day, 2);
        assert!(ev.contains(&Event::NightStarted) && ev.contains(&Event::DayStarted(2)));
    }
}
