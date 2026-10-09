//! Driving a boat from seat 0: the driver owns the hull's transform and sends it as
//! `BoatTransform` (the sim clamps speed and pushes it out of islands the same way).
//! The hull is also kept out of the raft's tiles; the sim does not check that.

use raft_sim::boats::Boat;
use raft_sim::content::{BoatKind, ISLANDS, TILE};
use raft_sim::math::{facing, wrap_angle};
use raft_sim::raft::{cell_center, Raft};
use raft_sim::{EntityId, V3};

/// Throttle response, m/s per second.
const ACCEL: f32 = 3.0;
/// Turn rate at full speed, rad/s.
const TURN: f32 = 1.2;
/// Reverse is this fraction of the top speed.
const REVERSE: f32 = 0.4;

pub struct Drive {
    pub boat: EntityId,
    pub kind: BoatKind,
    pub pos: V3,
    pub yaw: f32,
    pub speed: f32,
}

impl Drive {
    pub fn new(boat: EntityId, b: &Boat) -> Drive {
        Drive {
            boat,
            kind: b.kind,
            pos: b.pos,
            yaw: b.yaw,
            speed: b.speed,
        }
    }

    /// `throttle` and `turn` in -1..1 (W/S, D/A).
    pub fn step(&mut self, throttle: f32, turn: f32, dt: f32, raft: &Raft) {
        let def = self.kind.def();
        let want = if throttle > 0.0 {
            def.speed
        } else if throttle < 0.0 {
            -def.speed * REVERSE
        } else {
            0.0
        };
        self.speed += (want - self.speed).clamp(-ACCEL * dt, ACCEL * dt);
        // Rudder: barely turns at a standstill.
        let grip = (self.speed.abs() / def.speed).max(0.25);
        self.yaw = wrap_angle(self.yaw - turn * TURN * grip * dt);
        self.pos += facing(self.yaw) * self.speed * dt;
        for isl in &ISLANDS {
            let min = isl.radius + def.radius;
            let away = (self.pos - isl.center()).with_y(0.0);
            if away.length_xz() < min {
                self.pos = (isl.center() + away.normalize() * min).with_y(self.pos.y);
                self.speed = 0.0;
            }
        }
        if let Some(pos) = off_raft(raft, self.pos, def.radius) {
            self.pos = pos;
            self.speed = 0.0;
        }
    }

    /// The hull as the sim sees it, for `boats::seat_pos`.
    pub fn hull(&self) -> Boat {
        let mut b = Boat::new(self.kind, self.pos);
        b.yaw = self.yaw;
        b.speed = self.speed;
        b
    }
}

/// `pos` pushed out of the raft's tiles grown by `r` (rounded corners), if inside.
fn off_raft(raft: &Raft, pos: V3, r: f32) -> Option<V3> {
    let h = TILE / 2.0;
    let mut p = pos;
    // In a corner between tiles one push can land in another tile's margin.
    for _ in 0..4 {
        let near = raft
            .tiles
            .iter()
            .map(|&cell| {
                let c = cell_center(cell);
                V3::new(p.x.clamp(c.x - h, c.x + h), p.y, p.z.clamp(c.z - h, c.z + h))
            })
            .min_by(|a, b| a.dist_xz(p).total_cmp(&b.dist_xz(p)))?;
        let away = (p - near).with_y(0.0);
        let d = away.length_xz();
        if d >= r - 1e-4 {
            break;
        }
        p = if d > 1e-4 {
            near + away * (r / d)
        } else {
            // Centre over the deck (a fast or spawned hull): out past the nearest side
            // of the raft's bounds.
            let (lo, hi) = raft.bounds()?;
            let (x0, z0) = (lo.0 as f32 * TILE - r, lo.1 as f32 * TILE - r);
            let (x1, z1) = ((hi.0 + 1) as f32 * TILE + r, (hi.1 + 1) as f32 * TILE + r);
            let sides = [
                (p.x - x0, V3::new(x0, p.y, p.z)),
                (x1 - p.x, V3::new(x1, p.y, p.z)),
                (p.z - z0, V3::new(p.x, p.y, z0)),
                (z1 - p.z, V3::new(p.x, p.y, z1)),
            ];
            sides.into_iter().min_by(|a, b| a.0.total_cmp(&b.0))?.1
        };
    }
    (p != pos).then_some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drives_forward_and_stops_at_the_shore() {
        // Hook Isle lies straight ahead (-Z) of the start, centre z -38, radius 11.
        let b = Boat::new(BoatKind::Dinghy, V3::xz(0.0, 0.0));
        let mut d = Drive::new(1, &b);
        for _ in 0..60 * 30 {
            d.step(1.0, 0.0, 1.0 / 60.0, &Raft::default());
        }
        let shore = 38.0 - 11.0 - BoatKind::Dinghy.def().radius;
        assert!((d.pos.z + shore).abs() < 0.01, "stopped at z {}", d.pos.z);
        assert_eq!(d.speed, 0.0);
        // D turns right (clockwise from above: yaw decreases).
        d.step(0.0, 1.0, 0.5, &Raft::default());
        assert!(d.yaw < 0.0);
    }

    #[test]
    fn never_sails_through_the_raft() {
        // The 4x4 start raft spans -4..4; the dinghy comes at it from the south (+Z).
        let raft = Raft {
            tiles: (-2..2).flat_map(|i| (-2..2).map(move |j| (i, j))).collect(),
            ..Raft::default()
        };
        let r = BoatKind::Dinghy.def().radius;
        let mut d = Drive::new(1, &Boat::new(BoatKind::Dinghy, V3::xz(1.0, 12.0)));
        for _ in 0..60 * 10 {
            d.step(1.0, 0.0, 1.0 / 60.0, &raft);
            assert!(d.pos.z >= 4.0 + r - 1e-3, "in the raft at z {}", d.pos.z);
        }
        assert!((d.pos.z - (4.0 + r)).abs() < 0.01 && d.speed == 0.0);
        // Round corner: off the south-east corner diagonally.
        let p = off_raft(&raft, V3::xz(4.5, 4.5), r).unwrap();
        assert!((p.dist(V3::xz(4.0, 4.0)) - r).abs() < 1e-3);
        // A hull spawned over the deck goes out the nearest side.
        let p = off_raft(&raft, V3::xz(3.0, 0.5), r).unwrap();
        assert!((p.x - (4.0 + r)).abs() < 1e-3 && p.z == 0.5);
        assert!(off_raft(&raft, V3::xz(0.0, 8.0), r).is_none());
    }
}
