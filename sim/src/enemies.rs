//! Enemies (spec 5, 14).
//!
//! `step`, in order:
//! - Not night: every Gloomfin (home Night) and the Leviathan dive and despawn (no
//!   carcass). islands.rs brings the Leviathan back each night until it is defeated.
//! - Night: a player is *prey* while up, outside flare light, and in the water or outside
//!   the campfire light. Every GLOOMFIN_GAP s (the first that long into the night) each
//!   prey with fewer than `gloomfin_per_player(day)` hunters gets one more, spawned
//!   GLOOMFIN_SPAWN_DIST away in open water, GLOOMFIN_MAX_TOTAL in all. Hunters whose
//!   prey is downed or gone take a prey that is short of hunters.
//! - Night with the campfire out: every CRAWLER_INTERVAL s a Crawler climbs a random raft
//!   outer edge, up to `crawler_max(day)` aboard (home Raft). They stay until killed.
//! - Island residents (`IslandDef::enemies`, home Island) spawn when an up player is
//!   within ISLAND_WAKE_DIST of the island's edge and none are alive. After the last one
//!   dies they stay away ENEMY_RESPAWN_DAYS.
//! - AI: stunned enemies do nothing. Gloomfin chase their prey and back off to
//!   GLOOMFIN_LURK while it is safe. Raft crawlers go for the nearest up player; island
//!   residents for one within aggro_range (kept up to 2x). Within attack_range they hit
//!   every attack_interval (`players::hurt`). Swimmers keep to open water, raft crawlers
//!   to the tiles, residents to their island radius.
//! - Leviathan: circles its atoll at radius + LEVIATHAN_SPAWN_DIST, charges an up player
//!   within LEVIATHAN_CHARGE_RANGE (kept up to aggro_range), bites, then recoils (Flee)
//!   along the ring for LEVIATHAN_RECOVER s.
//!
//! `handle` (both aggro the enemy, both refuse before changing anything):
//! - HarpoonEnemy: harpoon range + cooldown, `harpoon(tier).damage` (+ BARBED_TIP_DAMAGE),
//!   stun for `harpoon(tier).stun` (T4), pull HARPOON_ENEMY_PULL m toward the player.
//! - MeleeHit: MELEE_RANGE (or the enemy's own reach) + MELEE_SWING, `melee(tier).damage`.
//! - Death: EnemyKilled, carcass via `World::spawn_debris`, `islands::on_enemy_killed`
//!   (marks bosses defeated; quests 6 and 7 follow from it).

use crate::content::{
    crawler_max, gloomfin_per_player, harpoon, island, melee, tier_hp_mult, EnemyKind, IslandId,
    BARBED_TIP_DAMAGE, CRAWLER_INTERVAL, CYCLE_SECONDS, DECK_Y, ENEMY_RESPAWN_DAYS,
    GLOOMFIN_MAX_TOTAL, GLOOMFIN_SPAWN_DIST, HARPOON_ENEMY_PULL, ISLANDS, LEVIATHAN_SPAWN_DIST,
    MELEE_RANGE, MELEE_SWING,
};
use crate::math::{yaw_toward, V3};
use crate::msg::{deny, sound, Event, Request, SoundKind, Target};
use crate::players;
use crate::raft::{cell_center, Raft};
use crate::world::{EntityId, Ground, PlayerId, World};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::f32::consts::TAU;

/// Seconds between Gloomfin arrivals; the first comes this long into the night (tuning).
pub const GLOOMFIN_GAP: f32 = 4.0;
/// A Gloomfin whose prey is safe waits this far away (tuning).
pub const GLOOMFIN_LURK: f32 = 12.0;
/// Residents wake when an up player is this close to their island's edge (tuning).
pub const ISLAND_WAKE_DIST: f32 = 80.0;
/// Ground lower than this is open water for swimmers (tuning).
pub const SWIM_DEPTH: f32 = -0.5;
/// The Leviathan leaves its ring for up players this close (tuning).
pub const LEVIATHAN_CHARGE_RANGE: f32 = 40.0;
/// Seconds the Leviathan circles after a bite, on top of its attack interval (tuning).
pub const LEVIATHAN_RECOVER: f32 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnemyState {
    Idle,
    Chase,
    Attack,
    /// Leaving (Gloomfin at dawn) or recoiling.
    Flee,
}

/// Where an enemy belongs; decides despawn and respawn rules and the tier multiplier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnemyHome {
    /// Night hunter (Gloomfin): despawns at dawn.
    Night,
    /// Climbed onto the raft (Crawler).
    Raft,
    /// Island resident (respawns), also the Leviathan at Lighthouse Atoll.
    Island(IslandId),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Enemy {
    pub kind: EnemyKind,
    pub pos: V3,
    pub yaw: f32,
    pub hp: f32,
    pub max_hp: f32,
    pub state: EnemyState,
    pub target: Option<PlayerId>,
    pub home: EnemyHome,
    /// `World::now` values.
    pub attack_ready_at: f64,
    pub stun_until: f64,
}

impl Enemy {
    pub fn new(kind: EnemyKind, pos: V3, home: EnemyHome) -> Enemy {
        let mult = match home {
            EnemyHome::Island(id) => tier_hp_mult(island(id).tier),
            _ => 1.0,
        };
        let hp = kind.def().hp * mult;
        Enemy {
            kind,
            pos,
            yaw: 0.0,
            hp,
            max_hp: hp,
            state: EnemyState::Idle,
            target: None,
            home,
            attack_ready_at: 0.0,
            stun_until: 0.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EnemySpawn {
    /// Seconds until the next crawler may climb aboard.
    pub crawler_timer: f32,
    /// Seconds until the next Gloomfin arrives.
    #[serde(default)]
    pub gloomfin_timer: f32,
    /// `World::now` before which an island's residents do not return.
    #[serde(default)]
    pub respawn_at: BTreeMap<IslandId, f64>,
}

impl Default for EnemySpawn {
    fn default() -> EnemySpawn {
        EnemySpawn {
            crawler_timer: CRAWLER_INTERVAL,
            gloomfin_timer: GLOOMFIN_GAP,
            respawn_at: BTreeMap::new(),
        }
    }
}

pub(crate) fn step(w: &mut World, dt: f32, g: &dyn Ground, ev: &mut Vec<Event>) {
    let night = w.clock.is_night();
    if !night {
        w.dynamic
            .enemies
            .retain(|_, e| e.home != EnemyHome::Night && e.kind != EnemyKind::Leviathan);
    }
    gloomfin(w, dt, night, g, ev);
    crawlers(w, dt, night, ev);
    residents(w, g, ev);
    let ids: Vec<EntityId> = w.dynamic.enemies.keys().copied().collect();
    for id in ids {
        think(w, id, dt, g, ev);
    }
}

/// Position of a connected, up player.
fn up(w: &World, pid: PlayerId) -> Option<V3> {
    w.player(pid)
        .filter(|p| p.connected && !p.downed)
        .map(|p| p.pos)
}

/// Gloomfin prey: up, not under a flare, and in the water or outside the campfire light.
fn is_prey(w: &World, pid: PlayerId) -> bool {
    w.player(pid).is_some_and(|p| {
        p.connected
            && !p.downed
            && !w.in_flare_light(p.pos)
            && (p.in_water || !w.in_campfire_light(p.pos))
    })
}

fn nearest_up(w: &World, pos: V3, max: f32) -> Option<(PlayerId, V3)> {
    w.connected()
        .filter(|(_, p)| !p.downed && p.pos.dist_xz(pos) <= max)
        .map(|(id, p)| (id, p.pos))
        .min_by(|a, b| a.1.dist_xz(pos).total_cmp(&b.1.dist_xz(pos)))
}

fn open_water(w: &World, g: &dyn Ground, x: f32, z: f32) -> bool {
    !w.raft.covers_xz(x, z) && g.height_at(x, z).is_none_or(|h| h < SWIM_DEPTH)
}

fn can_stand(w: &World, g: &dyn Ground, e: &Enemy, x: f32, z: f32) -> bool {
    if e.kind.def().swims {
        return open_water(w, g, x, z);
    }
    match e.home {
        EnemyHome::Raft => w.raft.covers_xz(x, z),
        EnemyHome::Island(i) => V3::xz(x, z).dist_xz(island(i).center()) <= island(i).radius,
        EnemyHome::Night => true,
    }
}

/// Moves up to `step` toward `goal` (sliding along one axis when blocked), faces the
/// way it went and sits at its home's height.
fn walk(w: &World, g: &dyn Ground, e: &mut Enemy, goal: V3, step: f32) {
    if step > 0.0 {
        let to = e.pos.move_toward(goal.with_y(e.pos.y), step);
        let (x0, z0) = (e.pos.x, e.pos.z);
        for (x, z) in [(to.x, to.z), (to.x, z0), (x0, to.z)] {
            if (x, z) != (x0, z0) && can_stand(w, g, e, x, z) {
                e.yaw = yaw_toward(x - x0, z - z0);
                (e.pos.x, e.pos.z) = (x, z);
                break;
            }
        }
    }
    e.pos.y = if e.kind.def().swims {
        0.0
    } else if e.home == EnemyHome::Raft {
        DECK_Y
    } else {
        g.height_at(e.pos.x, e.pos.z).unwrap_or(0.0)
    };
}

fn count(w: &World, f: impl Fn(&Enemy) -> bool) -> usize {
    w.dynamic.enemies.values().filter(|e| f(e)).count()
}

fn hunters(w: &World, pid: PlayerId) -> u32 {
    count(w, |e| {
        e.kind == EnemyKind::Gloomfin && e.target == Some(pid)
    }) as u32
}

fn gloomfin(w: &mut World, dt: f32, night: bool, g: &dyn Ground, ev: &mut Vec<Event>) {
    if !night {
        w.dynamic.enemy_spawn.gloomfin_timer = GLOOMFIN_GAP;
        return;
    }
    let quota = gloomfin_per_player(w.clock.day);
    let prey: Vec<PlayerId> = w
        .connected()
        .map(|(id, _)| id)
        .filter(|&id| is_prey(w, id))
        .collect();
    let lost: Vec<EntityId> = w
        .dynamic
        .enemies
        .iter()
        .filter(|(_, e)| e.kind == EnemyKind::Gloomfin && e.target.and_then(|t| up(w, t)).is_none())
        .map(|(&id, _)| id)
        .collect();
    for id in lost {
        let pick = prey.iter().copied().find(|&p| hunters(w, p) < quota);
        if let Some(e) = w.dynamic.enemies.get_mut(&id) {
            e.target = pick;
        }
    }
    let s = &mut w.dynamic.enemy_spawn;
    s.gloomfin_timer -= dt;
    if s.gloomfin_timer > 0.0 {
        return;
    }
    s.gloomfin_timer = GLOOMFIN_GAP;
    for pid in prey {
        if count(w, |e| e.kind == EnemyKind::Gloomfin) >= GLOOMFIN_MAX_TOTAL {
            break;
        }
        if hunters(w, pid) >= quota {
            continue;
        }
        let at = w.player_pos(pid).with_y(0.0);
        let spot = (0..4).find_map(|_| {
            let a = w.rng.range(0.0, TAU);
            let p = at + V3::xz(a.cos(), a.sin()) * GLOOMFIN_SPAWN_DIST;
            open_water(w, g, p.x, p.z).then_some(p)
        });
        if let Some(pos) = spot {
            let id = w.spawn_enemy(EnemyKind::Gloomfin, pos, EnemyHome::Night, ev);
            if let Some(e) = w.dynamic.enemies.get_mut(&id) {
                e.target = Some(pid);
            }
        }
    }
}

fn crawlers(w: &mut World, dt: f32, night: bool, ev: &mut Vec<Event>) {
    let lit = w.economy.campfire.lit();
    let s = &mut w.dynamic.enemy_spawn;
    if !night || lit {
        s.crawler_timer = CRAWLER_INTERVAL;
        return;
    }
    s.crawler_timer -= dt;
    if s.crawler_timer > 0.0 {
        return;
    }
    s.crawler_timer = CRAWLER_INTERVAL;
    if count(w, |e| e.home == EnemyHome::Raft) as u32 >= crawler_max(w.clock.day) {
        return;
    }
    let edges = w.raft.outer_edges();
    if let Some(&(cell, side)) = w.rng.pick(&edges) {
        // Just inside the edge, so it stands on the tile it climbed.
        let pos = Raft::edge_pos(cell, side).lerp(cell_center(cell), 0.1);
        w.spawn_enemy(EnemyKind::Crawler, pos, EnemyHome::Raft, ev);
    }
}

fn residents(w: &mut World, g: &dyn Ground, ev: &mut Vec<Event>) {
    let now = w.now();
    for i in 0..ISLANDS.len() as IslandId {
        let def = island(i);
        let home = EnemyHome::Island(i);
        let waiting = w
            .dynamic
            .enemy_spawn
            .respawn_at
            .get(&i)
            .is_some_and(|&t| now < t);
        let near = || {
            w.connected().any(|(_, p)| {
                !p.downed && p.pos.dist_xz(def.center()) <= def.radius + ISLAND_WAKE_DIST
            })
        };
        if def.enemies.is_empty() || waiting || count(w, |e| e.home == home) > 0 || !near() {
            continue;
        }
        w.dynamic.enemy_spawn.respawn_at.remove(&i);
        for &(kind, n) in def.enemies {
            for _ in 0..n {
                let a = w.rng.range(0.0, TAU);
                let r = def.radius * 0.7 * w.rng.f32().sqrt();
                let p = def.center() + V3::xz(a.cos(), a.sin()) * r;
                let y = g.height_at(p.x, p.z).unwrap_or(0.0);
                w.spawn_enemy(kind, p.with_y(y), home, ev);
            }
        }
    }
}

fn think(w: &mut World, id: EntityId, dt: f32, g: &dyn Ground, ev: &mut Vec<Event>) {
    let now = w.now();
    let Some(mut e) = w.dynamic.enemies.get(&id).cloned() else {
        return;
    };
    if now < e.stun_until {
        return;
    }
    let def = e.kind.def();
    let run = def.speed * dt;
    // (victim, where to go, how far)
    let (victim, goal, step) = match (e.kind, e.home) {
        (EnemyKind::Gloomfin, _) => match e.target.and_then(|t| up(w, t).map(|p| (t, p))) {
            Some((t, tp)) if is_prey(w, t) => engage(&mut e, t, tp, now, run),
            Some((_, tp)) => {
                e.state = EnemyState::Flee;
                let away = e.pos + (e.pos - tp).with_y(0.0).normalize() * run;
                let d = e.pos.dist_xz(tp);
                (None, away, run.min(GLOOMFIN_LURK - d))
            }
            None => {
                e.state = EnemyState::Idle;
                (None, e.pos, 0.0)
            }
        },
        (EnemyKind::Leviathan, _) => leviathan(w, &mut e, now, run),
        (_, home) => {
            let keep = e
                .target
                .and_then(|t| up(w, t).map(|p| (t, p)))
                .filter(|(_, p)| {
                    home == EnemyHome::Raft || p.dist_xz(e.pos) <= 2.0 * def.aggro_range
                });
            let reach = if home == EnemyHome::Raft {
                f32::MAX
            } else {
                def.aggro_range
            };
            match keep.or_else(|| nearest_up(w, e.pos, reach)) {
                Some((t, tp)) => {
                    e.target = Some(t);
                    engage(&mut e, t, tp, now, run)
                }
                None => {
                    e.target = None;
                    e.state = EnemyState::Idle;
                    (None, e.pos, 0.0)
                }
            }
        }
    };
    walk(w, g, &mut e, goal, step);
    let pos = e.pos;
    w.dynamic.enemies.insert(id, e);
    if let Some(pid) = victim {
        sound(ev, SoundKind::Bite, pos);
        players::hurt(w, pid, def.damage, ev);
    }
}

/// Chase into attack range, then hit every attack_interval. Returns (victim, goal, step).
fn engage(e: &mut Enemy, t: PlayerId, tp: V3, now: f64, run: f32) -> (Option<PlayerId>, V3, f32) {
    let def = e.kind.def();
    let d = e.pos.dist_xz(tp);
    if d > def.attack_range {
        e.state = EnemyState::Chase;
        return (None, tp, run.min(d - 0.8 * def.attack_range));
    }
    e.state = EnemyState::Attack;
    if d > 1e-3 {
        e.yaw = yaw_toward(tp.x - e.pos.x, tp.z - e.pos.z);
    }
    if now < e.attack_ready_at {
        return (None, e.pos, 0.0);
    }
    e.attack_ready_at = now + def.attack_interval as f64;
    (Some(t), e.pos, 0.0)
}

fn leviathan(w: &World, e: &mut Enemy, now: f64, run: f32) -> (Option<PlayerId>, V3, f32) {
    if now >= e.attack_ready_at {
        let prey = e
            .target
            .and_then(|t| up(w, t).map(|p| (t, p)))
            .filter(|(_, p)| p.dist_xz(e.pos) <= e.kind.def().aggro_range)
            .or_else(|| nearest_up(w, e.pos, LEVIATHAN_CHARGE_RANGE));
        if let Some((t, tp)) = prey {
            e.target = Some(t);
            let out = engage(e, t, tp, now, run);
            if out.0.is_some() {
                e.attack_ready_at += LEVIATHAN_RECOVER as f64;
            }
            return out;
        }
        e.state = EnemyState::Idle;
    } else {
        e.state = EnemyState::Flee;
    }
    let EnemyHome::Island(i) = e.home else {
        return (None, e.pos, 0.0);
    };
    // Circle: the next point along the ring, `run` metres on.
    let (c, ring) = (island(i).center(), island(i).radius + LEVIATHAN_SPAWN_DIST);
    let off = e.pos - c;
    let a = off.z.atan2(off.x) + run / ring;
    (None, c + V3::xz(a.cos(), a.sin()) * ring, run)
}

pub(crate) fn handle(
    w: &mut World,
    pid: PlayerId,
    req: Request,
    g: &dyn Ground,
    ev: &mut Vec<Event>,
) {
    let now = w.now();
    let (Request::HarpoonEnemy(id) | Request::MeleeHit(id)) = req else {
        return deny(ev, pid, "Not an enemy action");
    };
    let Some(p) = w.player(pid) else {
        return;
    };
    let (ppos, gear) = (p.pos, p.gear.clone());
    let (harpoon_ready, melee_ready) = (p.harpoon_ready_at, p.melee_ready_at);
    let Some(e) = w.dynamic.enemies.get(&id) else {
        return deny(ev, pid, "It is gone");
    };
    let d = e.pos.dist_xz(ppos);
    let reach = MELEE_RANGE.max(e.kind.def().attack_range);
    let harpooned = matches!(req, Request::HarpoonEnemy(_));
    let tool = harpoon(gear.harpoon);
    if harpooned && now < harpoon_ready {
        return deny(ev, pid, "The harpoon is reloading");
    }
    if harpooned && d > tool.range {
        return deny(ev, pid, "Out of range");
    }
    if !harpooned && now < melee_ready {
        return deny(ev, pid, "Still swinging");
    }
    if !harpooned && d > reach {
        return deny(ev, pid, "Too far away");
    }
    let p = w.player_mut(pid).expect("checked above");
    let (damage, sfx) = if harpooned {
        p.harpoon_ready_at = now + tool.cooldown as f64;
        let barb = if gear.barbed_tip {
            BARBED_TIP_DAMAGE
        } else {
            0.0
        };
        (tool.damage + barb, SoundKind::HarpoonHit)
    } else {
        p.melee_ready_at = now + MELEE_SWING as f64;
        (melee(gear.melee).damage, SoundKind::EnemyHit)
    };
    let mut e = w.dynamic.enemies[&id].clone();
    e.hp -= damage;
    e.target = Some(pid);
    ev.push(Event::Damage {
        target: Target::Enemy(id),
        amount: damage,
        pos: e.pos,
    });
    sound(ev, sfx, e.pos);
    if e.hp <= 0.0 {
        return kill(w, id, Some(pid), ev);
    }
    if harpooned {
        if tool.stun > 0.0 {
            e.stun_until = e.stun_until.max(now + tool.stun as f64);
        }
        walk(w, g, &mut e, ppos, HARPOON_ENEMY_PULL.min(d - 1.0));
    }
    w.dynamic.enemies.insert(id, e);
}

fn kill(w: &mut World, id: EntityId, by: Option<PlayerId>, ev: &mut Vec<Event>) {
    let Some(e) = w.dynamic.enemies.remove(&id) else {
        return;
    };
    ev.push(Event::EnemyKilled {
        id,
        kind: e.kind,
        pos: e.pos,
        by,
    });
    sound(ev, SoundKind::EnemyDie, e.pos);
    if let Some(carcass) = e.kind.def().carcass {
        w.spawn_debris(carcass, e.pos.with_y(0.0));
    }
    crate::islands::on_enemy_killed(w, e.kind);
    if let EnemyHome::Island(i) = e.home {
        if !island(i).enemies.is_empty() && count(w, |o| o.home == e.home) == 0 {
            let back = w.now() + (ENEMY_RESPAWN_DAYS as f32 * CYCLE_SECONDS) as f64;
            w.dynamic.enemy_spawn.respawn_at.insert(i, back);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{ItemKind, LIGHTHOUSE_ATOLL, PALM_REEF, RUSTED_WRECK, SMOKING_RIG};
    use crate::world::FlatGround;

    const G: FlatGround = FlatGround::WATER;

    fn denied(ev: &[Event]) -> bool {
        matches!(ev, [Event::Denied { .. }])
    }

    fn of(w: &World, kind: EnemyKind) -> Vec<(EntityId, Enemy)> {
        w.dynamic
            .enemies
            .iter()
            .filter(|(_, e)| e.kind == kind)
            .map(|(&id, e)| (id, e.clone()))
            .collect()
    }

    fn hurt_player(ev: &[Event], pid: PlayerId) -> bool {
        ev.iter()
            .any(|e| matches!(e, Event::Damage { target: Target::Player(p), .. } if *p == pid))
    }

    /// Ana swims 20 m east of the raft, Bo stands by the campfire; it is night.
    fn night() -> (World, PlayerId, PlayerId) {
        let mut w = World::new(7);
        let a = w.join("Ana").unwrap();
        let b = w.join("Bo").unwrap();
        let p = w.player_mut(a).unwrap();
        p.pos = V3::xz(20.0, 0.0);
        p.in_water = true;
        w.player_mut(b).unwrap().pos = V3::new(1.0, 0.45, 0.0);
        w.clock.skip_to(301.0);
        (w, a, b)
    }

    #[test]
    fn tier_scales_hp() {
        assert_eq!(
            Enemy::new(EnemyKind::Crawler, V3::ZERO, EnemyHome::Raft).max_hp,
            30.0
        );
        assert_eq!(
            Enemy::new(
                EnemyKind::Crawler,
                V3::ZERO,
                EnemyHome::Island(RUSTED_WRECK)
            )
            .max_hp,
            39.0
        );
        assert_eq!(
            Enemy::new(
                EnemyKind::Leviathan,
                V3::ZERO,
                EnemyHome::Island(LIGHTHOUSE_ATOLL)
            )
            .max_hp,
            1200.0 * 2.2
        );
    }

    #[test]
    fn gloomfin_hunt_the_dark_bite_fear_flares_and_dive_at_dawn() {
        let (mut w, a, b) = night();
        w.dynamic.enemy_spawn.gloomfin_timer = GLOOMFIN_GAP;
        for _ in 0..15 {
            w.step(0.25, &G);
        }
        assert!(
            of(&w, EnemyKind::Gloomfin).is_empty(),
            "first one arrives after the gap"
        );
        w.step(0.25, &G);
        let fins = of(&w, EnemyKind::Gloomfin);
        assert_eq!(fins.len(), 1, "day 1: one per prey, Bo is in the light");
        let (id, f) = &fins[0];
        assert_eq!(f.target, Some(a));
        assert!(f.pos.dist_xz(V3::xz(20.0, 0.0)) <= GLOOMFIN_SPAWN_DIST + 2.0);
        let mut bitten = false;
        for _ in 0..40 {
            bitten |= hurt_player(&w.step(0.25, &G), a);
        }
        assert!(bitten && w.player(a).unwrap().health < 100.0);
        assert_eq!(of(&w, EnemyKind::Gloomfin).len(), 1, "quota holds");
        // A flare makes Ana safe: it backs off and stops biting.
        w.player_mut(a).unwrap().flare_until = w.now() + 30.0;
        let hp = w.player(a).unwrap().health;
        for _ in 0..8 {
            assert!(!hurt_player(&w.step(0.25, &G), a));
        }
        assert_eq!(w.player(a).unwrap().health, hp);
        let f = &w.dynamic.enemies[id];
        assert_eq!(f.state, EnemyState::Flee);
        assert!(f.pos.dist_xz(w.player_pos(a)) > 3.0);
        assert!(!hurt_player(&w.step(0.25, &G), b));
        // Dawn: they dive, no carcass.
        w.clock.skip_to(1.0);
        w.step(0.1, &G);
        assert!(
            w.dynamic.enemies.is_empty()
                && w.dynamic
                    .debris
                    .values()
                    .all(|d| d.kind != ItemKind::GloomfinCarcass)
        );
    }

    #[test]
    fn crawlers_board_while_the_fire_is_out() {
        let (mut w, a, b) = night();
        w.leave(a);
        // Bo's flare keeps the Gloomfin off once the fire is out; crawlers ignore it.
        w.player_mut(b).unwrap().flare_until = 1e9;
        assert!(w.economy.campfire.lit());
        for _ in 0..40 {
            w.step(0.25, &G);
        }
        assert!(of(&w, EnemyKind::Crawler).is_empty(), "fire lit");
        w.economy.campfire.fuel = 0.0;
        for _ in 0..31 {
            w.step(0.25, &G);
        }
        assert!(of(&w, EnemyKind::Crawler).is_empty());
        w.step(0.25, &G);
        let c = of(&w, EnemyKind::Crawler);
        assert_eq!(c.len(), 1);
        assert!(w.raft.covers_xz(c[0].1.pos.x, c[0].1.pos.z) && c[0].1.home == EnemyHome::Raft);
        // It walks to Bo and hits; max 2 + day/2 = 2 aboard.
        let mut hit = false;
        for _ in 0..120 {
            hit |= hurt_player(&w.step(0.25, &G), b);
            w.player_mut(b).unwrap().health = 100.0;
        }
        assert!(hit);
        let c = of(&w, EnemyKind::Crawler);
        assert_eq!(c.len(), 2);
        assert!(c.iter().all(|(_, e)| w.raft.covers_xz(e.pos.x, e.pos.z)));
        assert!(c.iter().all(|(_, e)| (e.pos.y - DECK_Y).abs() < 1e-6));
    }

    #[test]
    fn island_residents_wake_leash_die_and_respawn() {
        let mut w = World::new(3);
        let a = w.join("Ana").unwrap();
        let reef = island(PALM_REEF);
        let shore = reef.center() + V3::xz(reef.radius + ISLAND_WAKE_DIST + 5.0, 0.0);
        w.player_mut(a).unwrap().pos = shore;
        w.step(0.1, &G);
        assert!(of(&w, EnemyKind::ReefCrab).is_empty(), "too far");
        w.player_mut(a).unwrap().pos = reef.center() + V3::xz(reef.radius + 1.0, 0.0);
        w.step(0.1, &G);
        let crabs = of(&w, EnemyKind::ReefCrab);
        assert_eq!(crabs.len(), 4);
        for (_, c) in &crabs {
            assert!(c.pos.dist_xz(reef.center()) <= reef.radius * 0.7 + 0.01);
            assert_eq!(c.home, EnemyHome::Island(PALM_REEF));
        }
        // A crab chasing Ana stops at the shore.
        let (crab, _) = crabs[0];
        w.player_mut(a).unwrap().pos = reef.center() + V3::xz(reef.radius + 5.0, 0.0);
        let e = w.dynamic.enemies.get_mut(&crab).unwrap();
        e.pos = reef.center() + V3::xz(reef.radius - 6.0, 0.0);
        e.target = Some(a);
        for _ in 0..40 {
            w.step(0.25, &G);
        }
        let d = w.dynamic.enemies[&crab].pos.dist_xz(reef.center());
        assert!(d <= reef.radius + 1e-3 && d > reef.radius - 1.0, "{d}");
        assert_eq!(w.dynamic.enemies[&crab].state, EnemyState::Chase);
        // Kill them all.
        for (id, _) in of(&w, EnemyKind::ReefCrab) {
            let e = w.dynamic.enemies.get_mut(&id).unwrap();
            e.hp = 1.0;
            w.player_mut(a).unwrap().pos = e.pos;
            w.player_mut(a).unwrap().melee_ready_at = 0.0;
            let ev = w.apply(a, Request::MeleeHit(id), &G);
            assert!(ev
                .iter()
                .any(|e| matches!(e, Event::EnemyKilled { by: Some(_), .. })));
        }
        let carcasses = w
            .dynamic
            .debris
            .values()
            .filter(|d| d.kind == ItemKind::ReefCrabCarcass);
        assert_eq!(carcasses.count(), 4);
        let back = w.dynamic.enemy_spawn.respawn_at[&PALM_REEF];
        assert!((back - w.now() - 720.0).abs() < 1e-6);
        w.step(0.1, &G);
        assert!(of(&w, EnemyKind::ReefCrab).is_empty(), "two days away");
        w.clock.elapsed = back;
        w.step(0.1, &G);
        assert_eq!(of(&w, EnemyKind::ReefCrab).len(), 4);
        assert!(w.dynamic.enemy_spawn.respawn_at.is_empty());
    }

    #[test]
    fn harpoon_and_melee() {
        let mut w = World::new(5);
        let a = w.join("Ana").unwrap();
        w.player_mut(a).unwrap().pos = V3::new(0.0, 0.45, 0.0);
        w.clock.skip_to(301.0);
        let mut ev = Vec::new();
        let near = w.spawn_enemy(
            EnemyKind::Gloomfin,
            V3::xz(0.0, 10.0),
            EnemyHome::Night,
            &mut ev,
        );
        let far = w.spawn_enemy(
            EnemyKind::Gloomfin,
            V3::xz(15.0, 0.0),
            EnemyHome::Night,
            &mut ev,
        );
        let before = w.clone();
        for req in [
            Request::HarpoonEnemy(far),
            Request::HarpoonEnemy(999),
            Request::MeleeHit(near),
            Request::MeleeHit(999),
        ] {
            assert!(denied(&w.apply(a, req.clone(), &G)), "{req:?}");
        }
        assert_eq!(w, before);
        // T1 harpoon: 15 damage, pulled 3 m closer, aggro, no stun.
        let ev = w.apply(a, Request::HarpoonEnemy(near), &G);
        assert!(ev.contains(&Event::Damage {
            target: Target::Enemy(near),
            amount: 15.0,
            pos: V3::xz(0.0, 10.0)
        }));
        let e = &w.dynamic.enemies[&near];
        assert_eq!((e.hp, e.target, e.stun_until), (25.0, Some(a), 0.0));
        assert!((e.pos.z - 7.0).abs() < 1e-4 && e.pos.x.abs() < 1e-4);
        assert!(
            denied(&w.apply(a, Request::HarpoonEnemy(near), &G)),
            "reload"
        );
        // Melee: 12 damage within reach, then the swing cooldown.
        w.player_mut(a).unwrap().pos = V3::xz(0.0, 5.5);
        let ev = w.apply(a, Request::MeleeHit(near), &G);
        assert!(ev
            .iter()
            .any(|e| matches!(e, Event::Damage { amount, .. } if *amount == 12.0)));
        assert_eq!(w.dynamic.enemies[&near].hp, 13.0);
        assert!(denied(&w.apply(a, Request::MeleeHit(near), &G)), "swinging");
        // T4 + barbed tip: 70 damage and a 1.5 s stun that freezes it.
        w.clock.elapsed += 2.0;
        let gear = &mut w.player_mut(a).unwrap().gear;
        (gear.harpoon, gear.barbed_tip) = (4, true);
        let big = w.spawn_enemy(
            EnemyKind::Leviathan,
            V3::xz(20.0, 6.0),
            EnemyHome::Night,
            &mut ev.clone(),
        );
        w.apply(a, Request::HarpoonEnemy(big), &G);
        let e = w.dynamic.enemies[&big].clone();
        assert_eq!(e.hp, 1200.0 - 70.0);
        assert!((e.stun_until - w.now() - 1.5).abs() < 1e-6);
        w.step(0.25, &G);
        assert_eq!(w.dynamic.enemies[&big].pos, e.pos, "stunned");
        w.clock.elapsed += 1.5;
        w.step(0.25, &G);
        assert_ne!(w.dynamic.enemies[&big].pos, e.pos);
        // Death: EnemyKilled, a floating carcass, gone from the map.
        w.dynamic.enemies.get_mut(&near).unwrap().hp = 1.0;
        let pos = w.dynamic.enemies[&near].pos;
        w.player_mut(a).unwrap().pos = pos;
        w.clock.elapsed += 1.0;
        let ev = w.apply(a, Request::MeleeHit(near), &G);
        assert!(ev.contains(&Event::EnemyKilled {
            id: near,
            kind: EnemyKind::Gloomfin,
            pos,
            by: Some(a)
        }));
        assert!(!w.dynamic.enemies.contains_key(&near));
        assert!(w
            .dynamic
            .debris
            .values()
            .any(|d| d.kind == ItemKind::GloomfinCarcass && d.pos == pos.with_y(0.0)));
        // The Rig Brute's death marks the boss defeated (quest 6 follows in islands.rs).
        let mut ev = Vec::new();
        let brute = w.spawn_enemy(
            EnemyKind::RigBrute,
            pos,
            EnemyHome::Island(SMOKING_RIG),
            &mut ev,
        );
        w.dynamic.enemies.get_mut(&brute).unwrap().hp = 1.0;
        w.clock.elapsed += 1.0;
        w.apply(a, Request::MeleeHit(brute), &G);
        assert!(w.islands.get(SMOKING_RIG).unwrap().boss_defeated);
        assert!(w.dynamic.enemy_spawn.respawn_at.contains_key(&SMOKING_RIG));
        assert!(w
            .dynamic
            .debris
            .values()
            .any(|d| d.kind == ItemKind::BruteCarcass));
    }

    #[test]
    fn leviathan_circles_charges_bites_and_recoils() {
        let mut w = World::new(9);
        let a = w.join("Ana").unwrap();
        let atoll = island(LIGHTHOUSE_ATOLL);
        let (c, ring) = (atoll.center(), atoll.radius + LEVIATHAN_SPAWN_DIST);
        w.player_mut(a).unwrap().pos = c;
        w.clock.skip_to(301.0);
        w.dynamic.enemy_spawn.gloomfin_timer = 1e9;
        let start = c + V3::xz(ring, 0.0);
        let home = EnemyHome::Island(LIGHTHOUSE_ATOLL);
        let id = w.spawn_enemy(EnemyKind::Leviathan, start, home, &mut Vec::new());
        for _ in 0..16 {
            assert!(!hurt_player(&w.step(0.25, &G), a));
        }
        let e = &w.dynamic.enemies[&id];
        assert!((e.pos.dist_xz(c) - ring).abs() < 0.01, "on its ring");
        assert!(e.pos.dist_xz(start) > 20.0 && e.state == EnemyState::Idle);
        // A swimmer 30 m out: charge, bite, recoil, bite again only after the recovery.
        let at = e.pos;
        let out = (at - c).with_y(0.0).normalize();
        let p = w.player_mut(a).unwrap();
        p.pos = at + out * 30.0;
        p.in_water = true;
        let mut bites = Vec::new();
        let mut recoiled = false;
        for _ in 0..100 {
            if hurt_player(&w.step(0.25, &G), a) {
                bites.push(w.now());
            }
            recoiled |= !bites.is_empty() && w.dynamic.enemies[&id].state == EnemyState::Flee;
            w.player_mut(a).unwrap().health = 100.0;
        }
        assert!(bites.len() >= 2 && recoiled, "{bites:?}");
        assert!(bites[1] - bites[0] >= 7.5 - 1e-6, "{bites:?}");
        // Dawn: it dives.
        w.clock.skip_to(1.0);
        w.step(0.1, &G);
        assert!(w.dynamic.enemies.is_empty());
    }

    #[test]
    fn same_seed_same_night() {
        let run = || {
            let (mut w, _, _) = night();
            w.economy.campfire.fuel = 0.0;
            for _ in 0..200 {
                w.step(0.25, &G);
            }
            w
        };
        let w = run();
        assert!(!of(&w, EnemyKind::Gloomfin).is_empty() && !of(&w, EnemyKind::Crawler).is_empty());
        assert_eq!(w, run());
    }
}
