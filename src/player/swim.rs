//! Swimming: the controller ignores water, so the body's vertical speed is steered
//! toward a float height under the waves every frame. [`Pose`] lays an avatar down to
//! stroke or stands it up to tread, for the local and the remote players alike.

use super::FEET_OFFSET;
use raft_sim::content::DIVE_DEPTH;

/// Feet this far under the surface while floating: head and shoulders out.
pub const FLOAT_DEPTH: f32 = 1.2;
/// Space lifts the float this much (to look over the deck edge).
const RISE: f32 = 0.4;
/// Fastest vertical swim, m/s.
const MAX_VERTICAL: f32 = 2.5;

/// Horizontal speed (m/s) above which a swimmer strokes ("swim" clip) instead of
/// treading water ("tread").
pub const STROKE_SPEED: f32 = 0.5;
/// Stroking: the body leans this far forward (rad, about 77 deg) and rises this much,
/// so the back and the recovering arms break the surface.
const STROKE_TILT: f32 = 1.35;
const STROKE_LIFT: f32 = 0.25;
/// Treading stays upright and rises this much: head and shoulders out.
const TREAD_LIFT: f32 = 0.45;

/// How far a body is into the water poses (0..1 each), eased so tread and stroke blend.
#[derive(Clone, Copy, Debug, Default)]
pub struct Pose {
    water: f32,
    stroke: f32,
}

impl Pose {
    pub fn step(&mut self, in_water: bool, stroking: bool, dt: f32) {
        let k = (5.0 * dt).min(1.0);
        self.water += (f32::from(u8::from(in_water)) - self.water) * k;
        self.stroke += (f32::from(u8::from(in_water && stroking)) - self.stroke) * k;
    }

    /// (forward lean in rad, feet slid back, feet raised) for a body whose origin is its
    /// feet, at clock `t` seconds. The lean pivots on the middle of the body, not the feet.
    pub fn offsets(&self, t: f32) -> (f32, f32, f32) {
        let tilt = STROKE_TILT * self.stroke;
        let lift = TREAD_LIFT + (STROKE_LIFT - TREAD_LIFT) * self.stroke;
        let up = FEET_OFFSET * (1.0 - tilt.cos()) + self.water * (lift + 0.05 * (t * 2.4).sin());
        (tilt, FEET_OFFSET * tilt.sin(), up)
    }
}

/// The new vertical speed for feet at `feet_y` under a sea surface at `surface`.
/// `rise` (Space) floats higher, `dive` (Ctrl) sinks to DIVE_DEPTH below the float.
pub fn vertical_speed(feet_y: f32, surface: f32, rise: bool, dive: bool, speed: f32, dt: f32) -> f32 {
    let float = surface - FLOAT_DEPTH;
    let target = if dive {
        float - DIVE_DEPTH
    } else if rise {
        float + RISE
    } else {
        float
    };
    let want = ((target - feet_y) * 3.0).clamp(-MAX_VERTICAL, MAX_VERTICAL);
    // Water drag: blend quickly, so a jump in from the deck stops within a metre.
    speed + (want - speed) * (1.0 - (-8.0 * dt).exp())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs 10 s of frames, returns (lowest feet, final feet).
    fn swim(mut feet: f32, mut speed: f32, dive: bool) -> (f32, f32) {
        let dt = 1.0 / 60.0;
        let mut low = feet;
        for _ in 0..600 {
            speed = vertical_speed(feet, 0.0, false, dive, speed, dt);
            feet += speed * dt;
            low = low.min(feet);
        }
        (low, feet)
    }

    #[test]
    fn jumping_in_floats_back_up() {
        // Off the deck (0.45) falling at 6 m/s.
        let (low, end) = swim(0.45, -6.0, false);
        assert!(low > -FLOAT_DEPTH - 1.2, "sank to {low}");
        assert!((end + FLOAT_DEPTH).abs() < 0.05, "settled at {end}");
    }

    #[test]
    fn pose_is_flat_on_land_and_eases_in_water() {
        let mut p = Pose::default();
        assert_eq!(p.offsets(1.0), (0.0, 0.0, 0.0));
        for _ in 0..120 {
            p.step(true, true, 1.0 / 60.0);
        }
        let (tilt, back, up) = p.offsets(0.0);
        assert!((tilt - STROKE_TILT).abs() < 0.01 && back > 0.8 && up > STROKE_LIFT, "{tilt} {back} {up}");
        for _ in 0..120 {
            p.step(true, false, 1.0 / 60.0);
        }
        let (tilt, _, up) = p.offsets(0.0);
        assert!(tilt < 0.01 && (up - TREAD_LIFT).abs() < 0.01, "{tilt} {up}");
    }

    #[test]
    fn dive_stops_at_dive_depth() {
        let (low, end) = swim(-FLOAT_DEPTH, 0.0, true);
        let floor = -FLOAT_DEPTH - DIVE_DEPTH;
        assert!(low > floor - 0.1, "dove to {low}");
        assert!((end - floor).abs() < 0.05, "settled at {end}");
    }
}
