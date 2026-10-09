//! Floating debris and carcasses (spec 4, 6, 6.2).
//!
//! - `step`: on average every DEBRIS_SPAWN_INTERVAL (while fewer than DEBRIS_MAX pieces
//!   float) a kind from DEBRIS_WEIGHTS spawns at x = DEBRIS_SPAWN_X, z in
//!   +-DEBRIS_SPAWN_HALF_Z (world coordinates; the raft stays at the origin). Everything
//!   not dragged drifts with CURRENT and despawns when it crosses x = DEBRIS_DESPAWN_X
//!   (or after DEBRIS_MAX_AGE, for pieces dropped far from the current's lane). Dragged
//!   debris moves toward its player at HARPOON_DRAG_SPEED and, within DRAG_ARRIVE_DIST,
//!   goes into the sac if it fits (else it stops being dragged). Heights follow `wave`.
//! - PickUpDebris: within DEBRIS_PICKUP_RANGE; one item into the sac or Denied.
//! - HarpoonDebris: within the player's harpoon range and off cooldown; sets `dragged_by`.
//! - Carcasses (enemies.rs) and dropped items (players.rs) use `World::spawn_debris`.

use crate::content::{
    harpoon, Goods, ItemKind, CURRENT, DEBRIS_DESPAWN_X, DEBRIS_MAX, DEBRIS_PICKUP_RANGE,
    DEBRIS_SPAWN_HALF_Z, DEBRIS_SPAWN_INTERVAL, DEBRIS_SPAWN_X, DEBRIS_WEIGHTS, DRAG_ARRIVE_DIST,
    HARPOON_DRAG_SPEED,
};
use crate::math::V3;
use crate::msg::{deny, sound, toast, Event, Request, SoundKind};
use crate::world::{Ground, PlayerId, World};
use serde::{Deserialize, Serialize};

/// Debris that never crosses the despawn line (dropped near a far island) sinks after
/// this many seconds (tuning).
pub const DEBRIS_MAX_AGE: f64 = 600.0;

/// Sea surface height (m) at (x, z) at time `t` (`World::now`). The view renders its
/// water with the same function so floating things bob in step with it: three sines,
/// |height| <= 0.25.
pub fn wave(x: f32, z: f32, t: f64) -> f32 {
    let (x, z) = (x as f64, z as f64);
    (0.12 * (0.35 * x + 1.1 * t).sin()
        + 0.08 * (0.27 * z + 0.8 * t + 1.3).sin()
        + 0.05 * (0.21 * (x + z) + 1.7 * t + 2.1).sin()) as f32
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Debris {
    pub kind: ItemKind,
    pub pos: V3,
    /// `World::now` at spawn.
    pub spawned_at: f64,
    /// Being pulled by this player's harpoon.
    pub dragged_by: Option<PlayerId>,
}

impl Debris {
    pub fn new(kind: ItemKind, pos: V3, now: f64) -> Debris {
        Debris {
            kind,
            pos,
            spawned_at: now,
            dragged_by: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DebrisSpawn {
    /// Seconds until the next spawn.
    pub timer: f32,
}

impl Default for DebrisSpawn {
    /// A new world gets its first piece one interval in.
    fn default() -> DebrisSpawn {
        DebrisSpawn {
            timer: DEBRIS_SPAWN_INTERVAL,
        }
    }
}

pub(crate) fn step(w: &mut World, dt: f32, _g: &dyn Ground, ev: &mut Vec<Event>) {
    let now = w.now();
    spawn(w, dt);
    let mut arrived = Vec::new();
    let mut gone = Vec::new();
    let players = &w.players;
    for (&id, d) in w.dynamic.debris.iter_mut() {
        let puller = d.dragged_by.and_then(|pid| {
            players
                .get(&pid)
                .filter(|p| p.connected && !p.downed)
                .map(|p| (pid, p.pos))
        });
        if let Some((pid, to)) = puller {
            d.pos = d
                .pos
                .move_toward(to.with_y(d.pos.y), HARPOON_DRAG_SPEED * dt);
            if d.pos.dist_xz(to) <= DRAG_ARRIVE_DIST {
                arrived.push((id, pid));
            }
        } else {
            d.dragged_by = None;
            let x0 = d.pos.x;
            d.pos += CURRENT * dt;
            let crossed = x0 <= DEBRIS_DESPAWN_X && d.pos.x > DEBRIS_DESPAWN_X;
            if crossed || now - d.spawned_at > DEBRIS_MAX_AGE {
                gone.push(id);
            }
        }
        d.pos.y = wave(d.pos.x, d.pos.z, now);
    }
    for id in gone {
        w.dynamic.debris.remove(&id);
    }
    for (id, pid) in arrived {
        let Some(d) = w.dynamic.debris.get(&id).cloned() else {
            continue;
        };
        if w.give(pid, Goods::Item(d.kind, 1), ev) {
            w.dynamic.debris.remove(&id);
            sound(ev, SoundKind::Pickup, d.pos);
        } else if let Some(d) = w.dynamic.debris.get_mut(&id) {
            d.dragged_by = None;
            toast(ev, Some(pid), "Your sac is full");
        }
    }
}

fn spawn(w: &mut World, dt: f32) {
    let s = &mut w.dynamic.debris_spawn;
    s.timer -= dt;
    if s.timer > 0.0 {
        return;
    }
    // Gaps of 0.5-1.5 x the interval average out at DEBRIS_SPAWN_INTERVAL.
    s.timer += w.rng.range(0.5, 1.5) * DEBRIS_SPAWN_INTERVAL;
    if w.dynamic.debris.len() >= DEBRIS_MAX {
        return;
    }
    let weights = DEBRIS_WEIGHTS.map(|(_, weight)| weight);
    let kind = DEBRIS_WEIGHTS[w.rng.weighted(&weights)].0;
    let z = w.rng.range(-DEBRIS_SPAWN_HALF_Z, DEBRIS_SPAWN_HALF_Z);
    let y = wave(DEBRIS_SPAWN_X, z, w.now());
    w.spawn_debris(kind, V3::new(DEBRIS_SPAWN_X, y, z));
}

pub(crate) fn handle(
    w: &mut World,
    pid: PlayerId,
    req: Request,
    _g: &dyn Ground,
    ev: &mut Vec<Event>,
) {
    let now = w.now();
    let Some(p) = w.player(pid) else {
        return;
    };
    let (ppos, tool, ready_at) = (p.pos, harpoon(p.gear.harpoon), p.harpoon_ready_at);
    let (Request::PickUpDebris(id) | Request::HarpoonDebris(id)) = req else {
        return deny(ev, pid, "Not a debris action");
    };
    let Some(d) = w.dynamic.debris.get(&id) else {
        return deny(ev, pid, "It is gone");
    };
    let (kind, pos, dragged_by) = (d.kind, d.pos, d.dragged_by);
    if dragged_by.is_some_and(|by| by != pid) {
        return deny(ev, pid, "Someone is already pulling that");
    }
    if let Request::PickUpDebris(_) = req {
        if pos.dist_xz(ppos) > DEBRIS_PICKUP_RANGE {
            return deny(ev, pid, "Too far away");
        }
        if !w.give(pid, Goods::Item(kind, 1), ev) {
            return deny(ev, pid, "Your sac is full");
        }
        w.dynamic.debris.remove(&id);
        sound(ev, SoundKind::Pickup, pos);
        return;
    }
    if now < ready_at {
        return deny(ev, pid, "The harpoon is reloading");
    }
    if dragged_by.is_some() {
        return deny(ev, pid, "You are already pulling that");
    }
    if pos.dist_xz(ppos) > tool.range {
        return deny(ev, pid, "Out of range");
    }
    if let Some(d) = w.dynamic.debris.get_mut(&id) {
        d.dragged_by = Some(pid);
    }
    if let Some(p) = w.player_mut(pid) {
        p.harpoon_ready_at = now + tool.cooldown as f64;
    }
    sound(ev, SoundKind::HarpoonHit, pos);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::ItemKind::*;
    use crate::inventory::Stack;
    use crate::world::FlatGround;

    const G: FlatGround = FlatGround::WATER;

    fn denied(ev: &[Event]) -> bool {
        matches!(ev, [Event::Denied { .. }])
    }

    /// Fills every sac slot with a flare (stack size 1), so no debris fits.
    fn fill_sac(w: &mut World, pid: PlayerId) {
        let sac = &mut w.player_mut(pid).unwrap().sac;
        sac.slots.fill(Some(Stack::new(Flare, 1)));
    }

    /// One player at the raft centre; the spawner is switched off.
    fn setup() -> (World, PlayerId) {
        let mut w = World::new(3);
        let a = w.join("Ana").unwrap();
        w.player_mut(a).unwrap().pos = V3::new(0.0, 0.45, 0.0);
        w.dynamic.debris_spawn.timer = 1e9;
        (w, a)
    }

    #[test]
    fn wave_is_bounded_and_deterministic() {
        for i in 0..200 {
            let (x, z, t) = (i as f32 * 3.7 - 300.0, i as f32 * -1.3, i as f64 * 17.1);
            let h = wave(x, z, t);
            assert!(h.is_finite() && h.abs() <= 0.25, "{h}");
            assert_eq!(h, wave(x, z, t));
        }
        assert_ne!(wave(0.0, 0.0, 0.0), wave(0.0, 0.0, 1.0));
        assert!(wave(5.0, 5.0, 1e9).is_finite());
    }

    #[test]
    fn spawner_mix_rate_cap_and_determinism() {
        let run = || {
            let mut w = World::new(11);
            for _ in 0..(350 * 4) {
                w.step(0.25, &G);
            }
            w
        };
        let w = run();
        assert_eq!(
            w.dynamic.debris,
            run().dynamic.debris,
            "same seed, same sea"
        );
        // 350 s ~ 100 spawns; pieces live ~125 s before crossing x = 80.
        let n = w.dynamic.debris.len();
        assert!((25..=45).contains(&n), "{n} pieces");
        for d in w.dynamic.debris.values() {
            assert!(d.kind.is_debris() && !d.kind.name().contains("Carcass"));
            assert!(d.pos.x >= DEBRIS_SPAWN_X && d.pos.x <= DEBRIS_DESPAWN_X);
            assert!(d.pos.z.abs() <= DEBRIS_SPAWN_HALF_Z && d.pos.y.abs() <= 0.25);
        }
        // Planks are the most common kind over many rolls.
        let mut w = World::new(5);
        let mut planks = 0;
        let mut barrels = 0;
        for _ in 0..2000 {
            w.dynamic.debris.clear();
            w.dynamic.debris_spawn.timer = 0.0;
            spawn(&mut w, 0.1);
            match w.dynamic.debris.values().next().unwrap().kind {
                Plank => planks += 1,
                OilBarrel => barrels += 1,
                _ => {}
            }
        }
        assert!(
            planks > 550 && barrels < 250 && barrels > 80,
            "{planks} {barrels}"
        );
        // Cap.
        let (mut w, _) = setup();
        for i in 0..DEBRIS_MAX {
            w.spawn_debris(Plank, V3::xz(-60.0, i as f32));
        }
        w.dynamic.debris_spawn.timer = 0.0;
        w.step(0.1, &G);
        assert_eq!(w.dynamic.debris.len(), DEBRIS_MAX);
    }

    #[test]
    fn drift_and_despawn() {
        let (mut w, _) = setup();
        let near = w.spawn_debris(Plank, V3::xz(DEBRIS_DESPAWN_X - 0.1, 5.0));
        let island = w.spawn_debris(CrawlerCarcass, V3::xz(1150.0, -600.0));
        let mid = w.spawn_debris(ScrapChunk, V3::xz(0.0, 20.0));
        w.step(0.25, &G);
        assert!(!w.dynamic.debris.contains_key(&near), "crossed the line");
        assert!(
            w.dynamic.debris.contains_key(&island),
            "far carcasses stay a while"
        );
        let x = w.dynamic.debris[&mid].pos.x;
        assert!((x - 0.3).abs() < 1e-4, "drifts +X at 1.2 m/s: {x}");
        w.clock.elapsed += DEBRIS_MAX_AGE;
        w.step(0.25, &G);
        assert!(!w.dynamic.debris.contains_key(&island));
    }

    #[test]
    fn pick_up() {
        let (mut w, a) = setup();
        let id = w.spawn_debris(Plank, V3::xz(2.5, 0.0));
        let far = w.spawn_debris(Plank, V3::xz(3.5, 0.0));
        let before = w.clone();
        assert!(denied(&w.apply(a, Request::PickUpDebris(far), &G)));
        assert!(denied(&w.apply(a, Request::PickUpDebris(999), &G)));
        assert_eq!(w, before);
        let ev = w.apply(a, Request::PickUpDebris(id), &G);
        assert!(ev.contains(&Event::Gained {
            player: a,
            goods: Goods::Item(Plank, 1)
        }));
        assert!(!w.dynamic.debris.contains_key(&id));
        assert_eq!(w.player(a).unwrap().sac.count(Plank), 1);
        // A full sac refuses and leaves the debris floating.
        fill_sac(&mut w, a);
        w.player_mut(a).unwrap().pos = V3::xz(3.5, 0.0);
        let before = w.clone();
        assert!(denied(&w.apply(a, Request::PickUpDebris(far), &G)));
        assert_eq!(w, before);
    }

    #[test]
    fn harpoon_drag_and_auto_pickup() {
        let (mut w, a) = setup();
        let far = w.spawn_debris(FloatingCrate, V3::xz(-20.0, 0.0));
        let id = w.spawn_debris(FloatingCrate, V3::xz(-13.0, 0.0));
        let before = w.clone();
        assert!(
            denied(&w.apply(a, Request::HarpoonDebris(far), &G)),
            "T1 = 14 m"
        );
        assert_eq!(w, before);
        assert!(!denied(&w.apply(a, Request::HarpoonDebris(id), &G)));
        assert_eq!(w.dynamic.debris[&id].dragged_by, Some(a));
        // Cooldown, then a second player cannot steal it.
        assert!(denied(&w.apply(a, Request::HarpoonDebris(id), &G)));
        let b = w.join("Bo").unwrap();
        w.player_mut(b).unwrap().pos = V3::xz(-12.0, 1.0);
        assert!(denied(&w.apply(b, Request::PickUpDebris(id), &G)));
        // 12 m/s against the 1.2 m/s current: 11.5 m in about a second.
        w.step(0.25, &G);
        let x = w.dynamic.debris[&id].pos.x;
        assert!((x - -10.0).abs() < 1e-3, "{x}");
        let mut got = false;
        for _ in 0..8 {
            got |= w.step(0.25, &G).iter().any(|e| {
                *e == Event::Gained {
                    player: a,
                    goods: Goods::Item(FloatingCrate, 1),
                }
            });
        }
        assert!(got && !w.dynamic.debris.contains_key(&id));
        assert_eq!(w.player(a).unwrap().sac.count(FloatingCrate), 1);
        // Full sac: it stops at the player and floats on.
        fill_sac(&mut w, a);
        w.clock.elapsed += 2.0;
        w.player_mut(a).unwrap().pos = V3::xz(-15.0, 0.0);
        assert!(!denied(&w.apply(a, Request::HarpoonDebris(far), &G)));
        let ev = w.step(0.25, &G);
        assert!(ev.iter().any(|e| matches!(e, Event::Toast { .. })));
        assert_eq!(w.dynamic.debris[&far].dragged_by, None);
    }

    #[test]
    fn downed_or_gone_puller_lets_go() {
        let (mut w, a) = setup();
        let id = w.spawn_debris(Plank, V3::xz(-10.0, 0.0));
        w.apply(a, Request::HarpoonDebris(id), &G);
        w.player_mut(a).unwrap().downed = true;
        w.step(0.25, &G);
        let d = &w.dynamic.debris[&id];
        assert_eq!(d.dragged_by, None);
        assert!((d.pos.x - -9.7).abs() < 1e-4, "drifts again");
    }
}
