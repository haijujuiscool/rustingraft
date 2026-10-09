//! The reel-in timing bar after a bite: a marker sweeps the bar back and forth, a click
//! inside the green zone lands the fish (spec 8).

use raft_sim::content::Fish;

pub struct Reel {
    /// 0..1 along the bar.
    pub marker: f32,
    /// +1 or -1.
    dir: f32,
    /// Bar widths per second.
    speed: f32,
    pub zone: [f32; 2],
}

impl Reel {
    /// `roll` in 0..1 places the zone, away from where the marker starts.
    pub fn new(fish: Fish, roll: f32) -> Reel {
        let def = fish.def();
        let start = 0.2 + roll.clamp(0.0, 1.0) * (0.75 - def.zone);
        Reel {
            marker: 0.0,
            dir: 1.0,
            speed: def.marker_speed,
            zone: [start, start + def.zone],
        }
    }

    pub fn step(&mut self, dt: f32) {
        self.marker += self.dir * self.speed * dt;
        if self.marker > 1.0 {
            self.marker = 2.0 - self.marker;
            self.dir = -1.0;
        } else if self.marker < 0.0 {
            self.marker = -self.marker;
            self.dir = 1.0;
        }
        self.marker = self.marker.clamp(0.0, 1.0);
    }

    pub fn hit(&self) -> bool {
        (self.zone[0]..=self.zone[1]).contains(&self.marker)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_bounces_and_zone_hits() {
        for fish in [Fish::ALL[0], Fish::ALL[4]] {
            for roll in [0.0, 0.5, 1.0] {
                let mut r = Reel::new(fish, roll);
                assert!(r.zone[0] > 0.1 && r.zone[1] <= 0.96, "{:?}", r.zone);
                assert!(!r.hit(), "starts outside the zone");
                let mut hits = 0;
                for _ in 0..600 {
                    r.step(1.0 / 60.0);
                    assert!((0.0..=1.0).contains(&r.marker));
                    hits += r.hit() as u32;
                }
                assert!(hits > 0, "{fish:?} {roll}: the marker never crossed the zone");
            }
        }
    }
}
