//! Minimal vector math. Engine convention: +Y up, -Z forward, yaw about +Y.

use serde::{Deserialize, Serialize};
use std::f32::consts::{PI, TAU};
use std::ops::{Add, AddAssign, Mul, Neg, Sub, SubAssign};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct V3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl V3 {
    pub const ZERO: V3 = V3::new(0.0, 0.0, 0.0);

    pub const fn new(x: f32, y: f32, z: f32) -> V3 {
        V3 { x, y, z }
    }

    /// A point on the water plane (y = 0).
    pub const fn xz(x: f32, z: f32) -> V3 {
        V3::new(x, 0.0, z)
    }

    pub fn with_y(self, y: f32) -> V3 {
        V3::new(self.x, y, self.z)
    }

    pub fn dot(self, o: V3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }

    /// Length ignoring height.
    pub fn length_xz(self) -> f32 {
        (self.x * self.x + self.z * self.z).sqrt()
    }

    /// Unit vector, or zero for a (near) zero vector.
    pub fn normalize(self) -> V3 {
        let l = self.length();
        if l > 1e-6 {
            self * (1.0 / l)
        } else {
            V3::ZERO
        }
    }

    pub fn dist(self, o: V3) -> f32 {
        (self - o).length()
    }

    /// Horizontal distance (ignores y). Use it for interaction ranges.
    pub fn dist_xz(self, o: V3) -> f32 {
        (self - o).length_xz()
    }

    pub fn lerp(self, o: V3, t: f32) -> V3 {
        self + (o - self) * t
    }

    /// Moves toward `target` by at most `step`, never overshooting.
    pub fn move_toward(self, target: V3, step: f32) -> V3 {
        let d = target - self;
        let l = d.length();
        if l <= step || l < 1e-6 {
            target
        } else {
            self + d * (step / l)
        }
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    pub fn to_array(self) -> [f32; 3] {
        [self.x, self.y, self.z]
    }

    pub fn from_array(a: [f32; 3]) -> V3 {
        V3::new(a[0], a[1], a[2])
    }
}

impl Add for V3 {
    type Output = V3;
    fn add(self, o: V3) -> V3 {
        V3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl Sub for V3 {
    type Output = V3;
    fn sub(self, o: V3) -> V3 {
        V3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl Mul<f32> for V3 {
    type Output = V3;
    fn mul(self, s: f32) -> V3 {
        V3::new(self.x * s, self.y * s, self.z * s)
    }
}

impl Neg for V3 {
    type Output = V3;
    fn neg(self) -> V3 {
        V3::new(-self.x, -self.y, -self.z)
    }
}

impl AddAssign for V3 {
    fn add_assign(&mut self, o: V3) {
        *self = *self + o;
    }
}

impl SubAssign for V3 {
    fn sub_assign(&mut self, o: V3) {
        *self = *self - o;
    }
}

/// Horizontal unit direction a yaw faces: (-sin yaw, 0, -cos yaw).
pub fn facing(yaw: f32) -> V3 {
    V3::new(-yaw.sin(), 0.0, -yaw.cos())
}

/// Yaw that faces the horizontal direction (dx, dz).
pub fn yaw_toward(dx: f32, dz: f32) -> f32 {
    (-dx).atan2(-dz)
}

/// Wraps an angle into (-PI, PI].
pub fn wrap_angle(a: f32) -> f32 {
    let mut a = (a + PI).rem_euclid(TAU) - PI;
    if a <= -PI {
        a += TAU;
    }
    a
}

/// Turns `yaw` toward `target` by at most `max_step` radians along the short way.
pub fn turn_toward(yaw: f32, target: f32, max_step: f32) -> f32 {
    let d = wrap_angle(target - yaw);
    wrap_angle(yaw + d.clamp(-max_step, max_step))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn yaw_conventions() {
        let f = facing(0.0);
        assert!(close(f.x, 0.0) && close(f.z, -1.0), "yaw 0 faces -Z");
        for (dx, dz) in [(1.0, 0.0), (0.0, 1.0), (-3.0, 2.0), (0.5, -4.0)] {
            let d = facing(yaw_toward(dx, dz));
            let n = V3::xz(dx, dz).normalize();
            assert!(close(d.x, n.x) && close(d.z, n.z), "{dx},{dz}");
        }
    }

    #[test]
    fn angles() {
        assert!(close(wrap_angle(3.0 * PI), PI));
        assert!(close(wrap_angle(-0.5), -0.5));
        assert!(close(turn_toward(0.0, 1.0, 0.25), 0.25));
        assert!(close(
            turn_toward(3.0, -3.0, 1.0),
            wrap_angle(3.0 + (TAU - 6.0))
        ));
    }

    #[test]
    fn vector_ops() {
        let a = V3::new(1.0, 2.0, 3.0);
        assert_eq!(a + a - a, a);
        assert_eq!(a * 2.0, V3::new(2.0, 4.0, 6.0));
        assert_eq!(V3::ZERO.normalize(), V3::ZERO);
        assert!(close(V3::new(3.0, 9.0, 4.0).dist_xz(V3::ZERO), 5.0));
        assert_eq!(V3::ZERO.lerp(a, 0.5), V3::new(0.5, 1.0, 1.5));
        assert_eq!(
            V3::ZERO.move_toward(V3::xz(10.0, 0.0), 3.0),
            V3::xz(3.0, 0.0)
        );
        assert_eq!(
            V3::ZERO.move_toward(V3::xz(1.0, 0.0), 3.0),
            V3::xz(1.0, 0.0)
        );
    }
}
