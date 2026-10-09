//! Swimming: the controller ignores water, so the body's vertical speed is steered
//! toward a float height under the waves every frame.

use raft_sim::content::DIVE_DEPTH;

/// Feet this far under the surface while floating: head and shoulders out.
pub const FLOAT_DEPTH: f32 = 1.2;
/// Space lifts the float this much (to look over the deck edge).
const RISE: f32 = 0.4;
/// Fastest vertical swim, m/s.
const MAX_VERTICAL: f32 = 2.5;

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
    fn dive_stops_at_dive_depth() {
        let (low, end) = swim(-FLOAT_DEPTH, 0.0, true);
        let floor = -FLOAT_DEPTH - DIVE_DEPTH;
        assert!(low > floor - 0.1, "dove to {low}");
        assert!((end - floor).abs() < 0.05, "settled at {end}");
    }
}
