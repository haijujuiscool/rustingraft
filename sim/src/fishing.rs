//! Fishing (spec 7). Two phases so the host rolls the species and the client runs the
//! minigame for that fish:
//! 1. CastLine { at }: `at` on open water (no ground, not over the raft) within
//!    CAST_RANGE of the player. Uses one Bait from the sac if any. The bite comes after
//!    BITE_MIN..BITE_MAX seconds (world RNG).
//! 2. `step`: at the bite the host rolls the species (`content::roll_fish`: rod tier,
//!    bait, night, Lucky Lure) and emits FishBite { fish }. The client shows the timing
//!    bar with `fish.def().zone` / `marker_speed` (rarer = smaller zone, faster marker).
//! 3. ReelIn { success }: only while hooked. Success puts the fish in the sac (a full
//!    sac is denied and the fish stays hooked until HOOK_SECONDS run out); failure lets
//!    it escape. CancelFishing ends fishing at any time.
//!
//! The line breaks when the player gets farther than CAST_RANGE + LINE_SLACK from the
//! bobber, and a hooked fish escapes after HOOK_SECONDS without a ReelIn.

use crate::content::{
    roll_fish, Fish, Goods, ItemKind, BITE_MAX, BITE_MIN, CAST_RANGE, HOOK_SECONDS, LINE_SLACK,
};
use crate::math::V3;
use crate::msg::{deny, sound, toast, Event, Request, SoundKind};
use crate::world::{Ground, PlayerId, World};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fishing {
    /// Bobber position on the water.
    pub at: V3,
    /// One bait was used for this cast.
    pub bait: bool,
    /// `World::now` when the fish bites.
    pub bite_at: f64,
    /// Rolled at the bite; the minigame is running.
    pub hooked: Option<Fish>,
}

pub(crate) fn step(w: &mut World, _dt: f32, _g: &dyn Ground, ev: &mut Vec<Event>) {
    let now = w.now();
    let night = w.clock.is_night();
    let ids: Vec<PlayerId> = w
        .connected()
        .filter(|(_, p)| p.fishing.is_some())
        .map(|(id, _)| id)
        .collect();
    for pid in ids {
        let p = w.players.get_mut(&pid).expect("listed above");
        let Some(f) = p.fishing.as_mut() else {
            continue;
        };
        if p.downed {
            p.fishing = None;
        } else if p.pos.dist_xz(f.at) > CAST_RANGE + LINE_SLACK {
            p.fishing = None;
            toast(ev, Some(pid), "The line snapped");
        } else if let Some(fish) = f.hooked {
            if now >= f.bite_at + HOOK_SECONDS as f64 {
                let at = f.at;
                p.fishing = None;
                toast(ev, Some(pid), format!("The {} got away", fish.def().name));
                sound(ev, SoundKind::FishEscape, at);
            }
        } else if now >= f.bite_at {
            let fish = roll_fish(&mut w.rng, p.gear.rod, f.bait, night, p.gear.lucky_lure);
            f.hooked = Some(fish);
            ev.push(Event::FishBite { player: pid, fish });
            sound(ev, SoundKind::FishBite, f.at);
        }
    }
}

pub(crate) fn handle(
    w: &mut World,
    pid: PlayerId,
    req: Request,
    g: &dyn Ground,
    ev: &mut Vec<Event>,
) {
    let Some(p) = w.players.get(&pid) else { return };
    match req {
        Request::CastLine { at } => {
            if p.fishing.is_some() {
                return deny(ev, pid, "Reel in first");
            }
            if !at.is_finite() || p.pos.dist_xz(at) > CAST_RANGE {
                return deny(ev, pid, "Too far to cast");
            }
            if g.height_at(at.x, at.z).is_some() || w.raft.covers_xz(at.x, at.z) {
                return deny(ev, pid, "Cast into the water");
            }
            let at = at.with_y(0.0);
            let delay = w.rng.range(BITE_MIN, BITE_MAX) as f64;
            let bite_at = w.now() + delay;
            let p = w.players.get_mut(&pid).expect("checked above");
            let bait = p.sac.remove(ItemKind::Bait, 1);
            p.fishing = Some(Fishing {
                at,
                bait,
                bite_at,
                hooked: None,
            });
            sound(ev, SoundKind::Splash, at);
        }
        Request::ReelIn { success } => {
            let Some(f) = &p.fishing else {
                return deny(ev, pid, "You are not fishing");
            };
            let (Some(fish), at) = (f.hooked, f.at) else {
                return deny(ev, pid, "Nothing is biting yet");
            };
            if success {
                if !w.give(pid, Goods::Item(ItemKind::Fish(fish), 1), ev) {
                    return deny(ev, pid, "Your sac is full");
                }
                sound(ev, SoundKind::FishCatch, at);
            } else {
                toast(ev, Some(pid), format!("The {} got away", fish.def().name));
                sound(ev, SoundKind::FishEscape, at);
            }
            w.players.get_mut(&pid).expect("checked above").fishing = None;
        }
        Request::CancelFishing => {
            if p.fishing.is_none() {
                return deny(ev, pid, "You are not fishing");
            }
            w.players.get_mut(&pid).expect("checked above").fishing = None;
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::fish_weights;
    use crate::world::{FlatGround, IslandGround};

    const WATER: FlatGround = FlatGround::WATER;

    /// A player at the raft's east edge, casting east.
    fn setup() -> (World, PlayerId, V3) {
        let mut w = World::new(5);
        let a = w.join("Ana").unwrap();
        w.player_mut(a).unwrap().pos = V3::new(3.5, 0.45, 0.0);
        (w, a, V3::xz(10.0, 0.0))
    }

    fn denied(ev: &[Event]) -> bool {
        matches!(ev, [Event::Denied { .. }])
    }

    fn step_until_bite(w: &mut World, g: &dyn Ground) -> Fish {
        for _ in 0..(BITE_MAX as usize * 60 + 10) {
            for e in w.step(1.0 / 60.0, g) {
                if let Event::FishBite { fish, .. } = e {
                    return fish;
                }
            }
        }
        panic!("no bite");
    }

    #[test]
    fn cast_bite_catch() {
        let (mut w, a, at) = setup();
        let ev = w.apply(a, Request::CastLine { at }, &WATER);
        assert!(!denied(&ev), "{ev:?}");
        let f = w.player(a).unwrap().fishing.clone().unwrap();
        assert!(!f.bait && f.hooked.is_none());
        let wait = f.bite_at - w.now();
        assert!((BITE_MIN as f64..BITE_MAX as f64).contains(&wait), "{wait}");
        assert!(denied(&w.apply(a, Request::CastLine { at }, &WATER)));
        assert!(denied(&w.apply(
            a,
            Request::ReelIn { success: true },
            &WATER
        )));

        let fish = step_until_bite(&mut w, &WATER);
        assert!(w.now() >= f.bite_at);
        assert_eq!(
            w.player(a).unwrap().fishing.as_ref().unwrap().hooked,
            Some(fish)
        );
        let ev = w.apply(a, Request::ReelIn { success: true }, &WATER);
        assert!(ev.contains(&Event::Gained {
            player: a,
            goods: Goods::Item(ItemKind::Fish(fish), 1)
        }));
        let p = w.player(a).unwrap();
        assert_eq!(p.sac.count(ItemKind::Fish(fish)), 1);
        assert!(p.fishing.is_none());
    }

    #[test]
    fn reel_failure_and_cancel() {
        let (mut w, a, at) = setup();
        assert!(denied(&w.apply(a, Request::CancelFishing, &WATER)));
        w.apply(a, Request::CastLine { at }, &WATER);
        step_until_bite(&mut w, &WATER);
        let ev = w.apply(a, Request::ReelIn { success: false }, &WATER);
        assert!(!denied(&ev));
        assert!(w.player(a).unwrap().fishing.is_none());
        assert!(w.player(a).unwrap().sac.is_empty());

        w.apply(a, Request::CastLine { at }, &WATER);
        assert!(w.apply(a, Request::CancelFishing, &WATER).is_empty());
        assert!(w.player(a).unwrap().fishing.is_none());
    }

    #[test]
    fn bad_casts_are_denied_and_change_nothing() {
        let (mut w, a, _) = setup();
        let before = w.clone();
        for at in [
            V3::xz(f32::NAN, 0.0),
            V3::xz(3.5 + CAST_RANGE + 0.5, 0.0),
            V3::xz(1.0, 1.0), // over the raft
        ] {
            assert!(
                denied(&w.apply(a, Request::CastLine { at }, &WATER)),
                "{at:?}"
            );
        }
        assert!(denied(&w.apply(
            a,
            Request::CastLine {
                at: V3::xz(10.0, 0.0)
            },
            &FlatGround::LAND
        )));
        // Hook Isle is 38 m north; from its shore, casting onto it is refused.
        w.player_mut(a).unwrap().pos = V3::xz(0.0, -24.0);
        assert!(denied(&w.apply(
            a,
            Request::CastLine {
                at: V3::xz(0.0, -30.0)
            },
            &IslandGround
        )));
        w.player_mut(a).unwrap().pos = before.player(a).unwrap().pos;
        assert_eq!(w, before);
    }

    #[test]
    fn bait_is_used_once_per_cast() {
        let (mut w, a, at) = setup();
        w.player_mut(a).unwrap().sac.add(ItemKind::Bait, 2);
        w.apply(a, Request::CastLine { at }, &WATER);
        assert!(w.player(a).unwrap().fishing.as_ref().unwrap().bait);
        assert_eq!(w.player(a).unwrap().sac.count(ItemKind::Bait), 1);
        w.apply(a, Request::CancelFishing, &WATER);
        w.apply(a, Request::CastLine { at }, &WATER);
        w.apply(a, Request::CancelFishing, &WATER);
        w.apply(a, Request::CastLine { at }, &WATER);
        assert!(!w.player(a).unwrap().fishing.as_ref().unwrap().bait);
        assert_eq!(w.player(a).unwrap().sac.count(ItemKind::Bait), 0);
    }

    #[test]
    fn full_sac_keeps_the_fish_hooked_until_it_escapes() {
        let (mut w, a, at) = setup();
        let p = w.player_mut(a).unwrap();
        for _ in 0..p.sac.capacity() {
            p.sac.add(ItemKind::Flare, 1);
        }
        w.apply(a, Request::CastLine { at }, &WATER);
        step_until_bite(&mut w, &WATER);
        let before = w.clone();
        assert!(denied(&w.apply(
            a,
            Request::ReelIn { success: true },
            &WATER
        )));
        assert_eq!(w, before);
        for _ in 0..(HOOK_SECONDS as usize * 60 + 5) {
            w.step(1.0 / 60.0, &WATER);
        }
        assert!(w.player(a).unwrap().fishing.is_none(), "it got away");
    }

    #[test]
    fn walking_away_snaps_the_line() {
        let (mut w, a, at) = setup();
        w.apply(a, Request::CastLine { at }, &WATER);
        w.player_mut(a).unwrap().pos = V3::new(-8.0, 0.45, 0.0);
        let ev = w.step(1.0 / 60.0, &WATER);
        assert!(w.player(a).unwrap().fishing.is_none());
        assert!(ev.iter().any(|e| matches!(e, Event::Toast { .. })));
    }

    #[test]
    fn odds_follow_rod_bait_lure_and_night() {
        // Empirical share of rare fish over many bites, per setup.
        fn rare_share(rod: u8, bait: bool, lure: bool, night: bool) -> f64 {
            let (mut w, a, at) = setup();
            let g = &WATER;
            let p = w.player_mut(a).unwrap();
            p.gear.rod = rod;
            p.gear.lucky_lure = lure;
            if night {
                w.clock.skip_to(305.0);
            }
            let mut rare = 0;
            let n = 3000;
            for _ in 0..n {
                if bait {
                    w.player_mut(a).unwrap().sac.add(ItemKind::Bait, 1);
                }
                w.apply(a, Request::CastLine { at }, g);
                w.player_mut(a).unwrap().fishing.as_mut().unwrap().bite_at = w.now();
                let ev = w.step(1e-3, g);
                let fish = ev
                    .iter()
                    .find_map(|e| match e {
                        Event::FishBite { fish, .. } => Some(*fish),
                        _ => None,
                    })
                    .unwrap();
                rare += fish.def().rare as u32;
                w.apply(a, Request::CancelFishing, g);
            }
            rare as f64 / n as f64
        }
        let expect = |rod, bait, lure, night| {
            let wt = fish_weights(rod, bait, night, lure);
            (wt[2] + wt[3] + wt[4]) as f64 / wt.iter().sum::<f32>() as f64
        };
        for (rod, bait, lure, night) in [
            (1, false, false, false),
            (2, false, false, false),
            (3, true, true, false),
            (1, false, false, true),
        ] {
            let got = rare_share(rod, bait, lure, night);
            let want = expect(rod, bait, lure, night);
            assert!(
                (got - want).abs() < 0.035,
                "{rod} {bait} {lure} {night}: {got} vs {want}"
            );
        }
        assert!(expect(3, true, true, false) > 2.0 * expect(1, false, false, false));
    }

    #[test]
    fn downed_players_stop_fishing() {
        let (mut w, a, at) = setup();
        w.apply(a, Request::CastLine { at }, &WATER);
        w.player_mut(a).unwrap().downed = true;
        w.step(1.0 / 60.0, &WATER);
        assert!(w.player(a).unwrap().fishing.is_none());
    }
}
