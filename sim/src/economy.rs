//! Shredder, stockpile, campfire, crafting, placement and farms (spec 8, 10).
//!
//! - DepositShredder (near the Shredder): every debris stack in the sac joins the queue;
//!   `step` shreds one item per SHRED_SECONDS into the stockpile. Crate bait and coins go
//!   to the depositor while connected (bait that does not fit is lost, with a toast).
//! - FeedCampfire (near the Campfire): FEED_WOOD wood or FEED_OIL oil from the stockpile
//!   adds points (levels at CAMPFIRE_THRESHOLDS) and fuel (capped). Refused when the fire
//!   is at max level and full of fuel. Fuel burns 1/s; reaching 0 emits CampfireOut.
//! - Craft (near the Workbench) is for items, gear and boats only and pays immediately.
//!   Gear the player already has (same or better tier) is refused; items must fit in the
//!   sac or nothing is paid. Boats spawn at `Raft::dock_pos`, spaced along the edge.
//! - Placed recipes never go through Craft: the client picks one at the workbench, shows
//!   the ghost, and sends Place / PlaceRailing, which check unlock + cost + geometry and
//!   pay at place time. The target must be within PLACE_RANGE of the player; floors may
//!   not go over land; only one Radio Station.
//! - Farms: without survivors StartFarm / CollectFarm by hand. With survivors they run by
//!   themselves at `farm_rate`; wood/metal output joins the shredder queue (`by: None`),
//!   oil output burns in the campfire (its shredded oil, unit by unit, while the fire has
//!   room for fuel; the rest goes to the stockpile).

use crate::content::{
    campfire_level, farm, farm_rate, light_radius, shred, Goods, ItemKind, PlaceableKind,
    RecipeDef, RecipeId, RecipeOutput, Stockpile, CAMPFIRE_MAX_LEVEL, DOCK_SPACING, FEED_OIL,
    FEED_WOOD, FUEL_CAP, FUEL_PER_OIL, FUEL_PER_WOOD, FUEL_START, INTERACT_RANGE, PLACE_RANGE,
    POINTS_PER_OIL, POINTS_PER_WOOD, SHRED_SECONDS,
};
use crate::math::V3;
use crate::msg::{deny, sound, toast, Event, Request, SoundKind};
use crate::raft::{cell_center, Cell, FarmState, Raft, Railing, Side, Structure};
use crate::world::{EntityId, Ground, PlayerId, World};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::iter::repeat_n;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Economy {
    pub stock: Stockpile,
    pub campfire: Campfire,
    pub shredder: Shredder,
    /// Rescued survivors living on the raft, by home island (survivors.rs adds them).
    pub survivors: Vec<crate::content::IslandId>,
    /// Paid saves so far (save.rs).
    pub save_count: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Campfire {
    /// 1..=5.
    pub level: u8,
    /// Total upgrade points fed so far.
    pub points: u32,
    /// Seconds of burn left.
    pub fuel: f32,
}

impl Campfire {
    pub fn lit(&self) -> bool {
        self.fuel > 0.0
    }

    /// Current light radius; 0 when out.
    pub fn light_radius(&self) -> f32 {
        if self.lit() {
            light_radius(self.level)
        } else {
            0.0
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Shredder {
    pub queue: VecDeque<ShredJob>,
    /// Seconds spent on the front job.
    pub timer: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShredJob {
    pub kind: ItemKind,
    /// Depositor (gets crate bonuses); None for farm output.
    pub by: Option<PlayerId>,
}

impl Economy {
    pub fn new() -> Economy {
        Economy {
            stock: Stockpile::default(),
            campfire: Campfire {
                level: 1,
                points: 0,
                fuel: FUEL_START,
            },
            shredder: Shredder::default(),
            survivors: Vec::new(),
            save_count: 0,
        }
    }

    /// The campfire level is high enough for this recipe.
    pub fn unlocked(&self, recipe: RecipeId) -> bool {
        recipe.def().level <= self.campfire.level
    }
}

impl Default for Economy {
    fn default() -> Economy {
        Economy::new()
    }
}

// ---------------------------------------------------------------- step

pub(crate) fn step(w: &mut World, dt: f32, _g: &dyn Ground, ev: &mut Vec<Event>) {
    step_farms(w, dt, ev);
    step_shredder(w, dt, ev);
    let fire = &mut w.economy.campfire;
    if fire.lit() {
        fire.fuel = (fire.fuel - dt).max(0.0);
        if !fire.lit() {
            ev.push(Event::CampfireOut);
            sound(ev, SoundKind::CampfireOut, at(w, PlaceableKind::Campfire));
            toast(ev, None, "The campfire went out");
        }
    }
}

fn step_shredder(w: &mut World, dt: f32, ev: &mut Vec<Event>) {
    let sh = &mut w.economy.shredder;
    if sh.queue.is_empty() {
        sh.timer = 0.0;
        return;
    }
    sh.timer += dt;
    // dt <= MAX_DT < SHRED_SECONDS, so at most one item per step.
    if sh.timer < SHRED_SECONDS {
        return;
    }
    sh.timer -= SHRED_SECONDS;
    let Some(job) = sh.queue.pop_front() else {
        return;
    };
    let Some(out) = shred(job.kind, &mut w.rng) else {
        return;
    };
    w.economy.stock.add(&out.stock);
    sound(ev, SoundKind::Shred, at(w, PlaceableKind::Shredder));
    let Some(by) = job
        .by
        .filter(|id| w.player(*id).is_some_and(|p| p.connected))
    else {
        return;
    };
    if out.bait > 0 && !w.give(by, Goods::Item(ItemKind::Bait, out.bait), ev) {
        toast(ev, Some(by), "No room in your sac for the crate's bait");
    }
    if out.coins > 0 {
        w.give(by, Goods::Coins(out.coins), ev);
    }
}

fn step_farms(w: &mut World, dt: f32, ev: &mut Vec<Event>) {
    let auto = !w.economy.survivors.is_empty();
    let rate = farm_rate(w.economy.survivors.len());
    let mut harvest = Vec::new();
    for s in w.raft.structures.values_mut() {
        let Some(def) = farm(s.kind) else { continue };
        let Some(f) = s.farm_mut() else { continue };
        if auto {
            if f.ready {
                f.ready = false;
                harvest.push((def.output, def.count));
            }
            f.running = true;
        }
        if !f.running {
            continue;
        }
        f.progress += dt * rate;
        if f.progress >= def.cycle {
            if auto {
                f.progress -= def.cycle;
                harvest.push((def.output, def.count));
            } else {
                *f = FarmState {
                    running: false,
                    progress: 0.0,
                    ready: true,
                };
            }
        }
    }
    for (kind, n) in harvest {
        if kind == ItemKind::OilBarrel {
            let oil = n * shred(kind, &mut w.rng).map_or(0, |o| o.stock.oil);
            for _ in 0..oil {
                if w.economy.campfire.fuel < FUEL_CAP {
                    stoke(w, POINTS_PER_OIL, FUEL_PER_OIL, ev);
                } else {
                    w.economy.stock.oil += 1;
                }
            }
        } else {
            let job = ShredJob { kind, by: None };
            w.economy.shredder.queue.extend(repeat_n(job, n as usize));
        }
    }
}

/// Adds upgrade points and fuel (capped) to the campfire and announces level-ups.
fn stoke(w: &mut World, points: u32, fuel: f32, ev: &mut Vec<Event>) {
    let pos = at(w, PlaceableKind::Campfire);
    let fire = &mut w.economy.campfire;
    fire.points = fire.points.saturating_add(points);
    fire.fuel = (fire.fuel + fuel).min(FUEL_CAP);
    let level = campfire_level(fire.points);
    if level > fire.level {
        ev.extend((fire.level + 1..=level).map(Event::CampfireLevelUp));
        fire.level = level;
        sound(ev, SoundKind::CampfireLevelUp, pos);
        toast(ev, None, format!("The campfire reached level {level}"));
    }
}

/// Centre of the first structure of `kind`, or the raft centre.
fn at(w: &World, kind: PlaceableKind) -> V3 {
    w.structure_pos(kind).unwrap_or_else(|| w.raft.center())
}

// ---------------------------------------------------------------- requests

type Refusal = String;

pub(crate) fn handle(
    w: &mut World,
    pid: PlayerId,
    req: Request,
    g: &dyn Ground,
    ev: &mut Vec<Event>,
) {
    let result = match req {
        Request::DepositShredder => deposit(w, pid, ev),
        Request::FeedCampfire { oil } => feed(w, pid, oil, ev),
        Request::Craft(recipe) => craft(w, pid, recipe, ev),
        Request::Place { recipe, cell, rot } => place(w, pid, recipe, cell, rot, g, ev),
        Request::PlaceRailing { cell, side } => place_railing(w, pid, cell, side, ev),
        Request::StartFarm(id) => start_farm(w, pid, id),
        Request::CollectFarm(id) => collect_farm(w, pid, id, ev),
        _ => return,
    };
    if let Err(reason) = result {
        deny(ev, pid, reason);
    }
}

fn deposit(w: &mut World, pid: PlayerId, ev: &mut Vec<Event>) -> Result<(), Refusal> {
    if w.near_structure(pid, PlaceableKind::Shredder).is_none() {
        return Err("Walk up to the shredder".into());
    }
    let p = w.player_mut(pid).ok_or("Unknown player")?;
    let stacks = p.sac.take_all(ItemKind::is_debris);
    if stacks.is_empty() {
        return Err("You have no debris".into());
    }
    let n: u32 = stacks.iter().map(|s| s.count).sum();
    let queue = &mut w.economy.shredder.queue;
    for s in stacks {
        let job = ShredJob {
            kind: s.kind,
            by: Some(pid),
        };
        queue.extend(repeat_n(job, s.count as usize));
    }
    sound(ev, SoundKind::Shred, at(w, PlaceableKind::Shredder));
    toast(ev, Some(pid), format!("{n} items into the shredder"));
    Ok(())
}

fn feed(w: &mut World, pid: PlayerId, oil: bool, ev: &mut Vec<Event>) -> Result<(), Refusal> {
    if w.near_structure(pid, PlaceableKind::Campfire).is_none() {
        return Err("Walk up to the campfire".into());
    }
    let (cost, points, fuel) = if oil {
        (
            Stockpile::new(0, 0, 0, FEED_OIL),
            FEED_OIL * POINTS_PER_OIL,
            FEED_OIL as f32 * FUEL_PER_OIL,
        )
    } else {
        (
            Stockpile::new(FEED_WOOD, 0, 0, 0),
            FEED_WOOD * POINTS_PER_WOOD,
            FEED_WOOD as f32 * FUEL_PER_WOOD,
        )
    };
    if !w.economy.stock.covers(&cost) {
        return Err(format!("Feeding takes {}", Goods::Stock(cost).label()));
    }
    let fire = &w.economy.campfire;
    if fire.level >= CAMPFIRE_MAX_LEVEL && fire.fuel >= FUEL_CAP {
        return Err("The fire is already roaring".into());
    }
    w.economy.stock.take(&cost);
    stoke(w, points, fuel, ev);
    sound(ev, SoundKind::CampfireFeed, at(w, PlaceableKind::Campfire));
    Ok(())
}

/// Unlock and cost checks shared by every recipe.
fn affordable(w: &World, def: &RecipeDef) -> Result<(), Refusal> {
    if !w.economy.unlocked(def.id) {
        return Err(format!("Unlocks at campfire level {}", def.level));
    }
    if !w.economy.stock.covers(&def.cost) {
        return Err(format!("Needs {}", Goods::Stock(def.cost).label()));
    }
    Ok(())
}

fn craft(
    w: &mut World,
    pid: PlayerId,
    recipe: RecipeId,
    ev: &mut Vec<Event>,
) -> Result<(), Refusal> {
    let def = recipe.def();
    if w.near_structure(pid, PlaceableKind::Workbench).is_none() {
        return Err("Walk up to the workbench".into());
    }
    affordable(w, def)?;
    match def.output {
        RecipeOutput::Give(goods) => {
            let gear = &w.player(pid).ok_or("Unknown player")?.gear;
            let owned = match goods {
                Goods::Harpoon(t) => gear.harpoon >= t,
                Goods::Melee(t) => gear.melee >= t,
                Goods::Rod(t) => gear.rod >= t,
                _ => false,
            };
            if owned {
                return Err("You already have one as good".into());
            }
            if !w.give(pid, goods, ev) {
                return Err("Your sac is full".into());
            }
        }
        RecipeOutput::Boat(kind) => {
            let pos = free_dock(w);
            w.spawn_boat(kind, pos);
            toast(ev, None, format!("A {} is moored at the raft", def.name));
        }
        RecipeOutput::Floor | RecipeOutput::Railing | RecipeOutput::Structure(_) => {
            return Err("Place it on the raft".into())
        }
    }
    w.economy.stock.take(&def.cost);
    sound(ev, SoundKind::Craft, at(w, PlaceableKind::Workbench));
    Ok(())
}

/// First spot along the dock edge (centre, then alternating sides) with no boat moored.
fn free_dock(w: &World) -> V3 {
    let dock = w.raft.dock_pos();
    (0..16)
        .map(|k: i32| {
            let side = if k % 2 == 0 { 1.0 } else { -1.0 };
            dock + V3::xz(side * DOCK_SPACING * ((k + 1) / 2) as f32, 0.0)
        })
        .find(|p| {
            w.dynamic
                .boats
                .values()
                .all(|b| b.pos.dist_xz(*p) >= DOCK_SPACING)
        })
        .unwrap_or(dock)
}

fn in_reach(w: &World, pid: PlayerId, target: V3) -> Result<(), Refusal> {
    if w.player_pos(pid).dist_xz(target) > PLACE_RANGE {
        return Err("Too far away".into());
    }
    Ok(())
}

fn place(
    w: &mut World,
    pid: PlayerId,
    recipe: RecipeId,
    cell: Cell,
    rot: u8,
    g: &dyn Ground,
    ev: &mut Vec<Event>,
) -> Result<(), Refusal> {
    let def = recipe.def();
    let target = match def.output {
        RecipeOutput::Floor => {
            w.raft.can_add_tile(cell)?;
            let c = cell_center(cell);
            if g.height_at(c.x, c.z).is_some() {
                return Err("The raft must float on open water".into());
            }
            c
        }
        RecipeOutput::Structure(kind) => {
            w.raft.can_place(kind, cell, rot)?;
            if kind == PlaceableKind::RadioStation && w.raft.count(kind) > 0 {
                return Err("The raft already has a Radio Station".into());
            }
            Structure::new(kind, cell, rot).center()
        }
        RecipeOutput::Railing => return Err("Railings go on a tile edge".into()),
        RecipeOutput::Boat(_) | RecipeOutput::Give(_) => {
            return Err("Craft it at the workbench".into())
        }
    };
    in_reach(w, pid, target)?;
    affordable(w, def)?;
    w.economy.stock.take(&def.cost);
    match def.output {
        RecipeOutput::Structure(kind) => {
            let id = w.alloc_id();
            w.raft
                .structures
                .insert(id, Structure::new(kind, cell, rot));
        }
        _ => {
            w.raft.tiles.insert(cell);
        }
    }
    sound(ev, SoundKind::Place, target);
    Ok(())
}

fn place_railing(
    w: &mut World,
    pid: PlayerId,
    cell: Cell,
    side: Side,
    ev: &mut Vec<Event>,
) -> Result<(), Refusal> {
    let def = RecipeId::Railing.def();
    w.raft.can_add_railing(cell, side)?;
    let target = Raft::edge_pos(cell, side);
    in_reach(w, pid, target)?;
    affordable(w, def)?;
    w.economy.stock.take(&def.cost);
    w.raft.railings.insert(Railing { cell, side });
    sound(ev, SoundKind::Place, target);
    Ok(())
}

/// The farm structure `id`, if the player stands next to it.
fn near_farm(w: &World, pid: PlayerId, id: EntityId) -> Result<&Structure, Refusal> {
    let s = w
        .raft
        .structures
        .get(&id)
        .filter(|s| s.kind.is_farm())
        .ok_or("No such farm")?;
    if w.player_pos(pid).dist_xz(s.center()) > INTERACT_RANGE {
        return Err("Walk up to the farm".into());
    }
    Ok(s)
}

fn start_farm(w: &mut World, pid: PlayerId, id: EntityId) -> Result<(), Refusal> {
    let f = near_farm(w, pid, id)?.farm().ok_or("No such farm")?;
    if !w.economy.survivors.is_empty() {
        return Err("Your survivors run the farms".into());
    }
    if f.ready {
        return Err("Collect the harvest first".into());
    }
    if f.running {
        return Err("It is already growing".into());
    }
    let f = w
        .raft
        .structures
        .get_mut(&id)
        .and_then(Structure::farm_mut)
        .ok_or("No such farm")?;
    f.running = true;
    f.progress = 0.0;
    Ok(())
}

fn collect_farm(
    w: &mut World,
    pid: PlayerId,
    id: EntityId,
    ev: &mut Vec<Event>,
) -> Result<(), Refusal> {
    let s = near_farm(w, pid, id)?;
    let def = farm(s.kind).ok_or("No such farm")?;
    if !s.farm().is_some_and(|f| f.ready) {
        return Err("Nothing to collect yet".into());
    }
    if !w.give(pid, Goods::Item(def.output, def.count), ev) {
        return Err("Your sac is full".into());
    }
    if let Some(f) = w.raft.structures.get_mut(&id).and_then(Structure::farm_mut) {
        f.ready = false;
    }
    sound(ev, SoundKind::Pickup, w.player_pos(pid));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::ItemKind::*;
    use crate::world::FlatGround;

    const G: FlatGround = FlatGround::WATER;

    fn world() -> (World, PlayerId) {
        let mut w = World::new(3);
        let a = w.join("Ana").unwrap();
        (w, a)
    }

    fn go(w: &mut World, a: PlayerId, kind: PlaceableKind) {
        w.player_mut(a).unwrap().pos = w.structure_pos(kind).unwrap();
    }

    /// Asserts the request is refused with one Denied and leaves the world untouched.
    fn refused(w: &mut World, a: PlayerId, req: Request) -> String {
        let before = w.clone();
        let ev = w.apply(a, req, &G);
        assert_eq!(*w, before, "a refusal changed the world");
        match &ev[..] {
            [Event::Denied { reason, .. }] => reason.clone(),
            _ => panic!("expected one Denied, got {ev:?}"),
        }
    }

    fn run(w: &mut World, secs: f32) -> Vec<Event> {
        let mut ev = Vec::new();
        for _ in 0..(secs * 60.0).round() as u32 {
            ev.extend(w.step(1.0 / 60.0, &G));
        }
        ev
    }

    fn add_farm(w: &mut World, kind: PlaceableKind) -> EntityId {
        let id = w.alloc_id();
        w.raft.tiles.extend([(2, 0), (3, 0)]);
        w.raft
            .structures
            .insert(id, Structure::new(kind, (2, 0), 0));
        id
    }

    fn farm_state(w: &World, id: EntityId) -> FarmState {
        w.raft.structures[&id].farm().unwrap().clone()
    }

    #[test]
    fn campfire_burns_out_once() {
        let (mut w, _) = world();
        w.economy.campfire.fuel = 1.0;
        let ev = run(&mut w, 3.0);
        assert_eq!(ev.iter().filter(|e| **e == Event::CampfireOut).count(), 1);
        assert_eq!(w.economy.campfire.fuel, 0.0);
        assert_eq!(w.economy.campfire.light_radius(), 0.0);
    }

    #[test]
    fn feeding_levels_and_fuels_the_fire() {
        let (mut w, a) = world();
        w.player_mut(a).unwrap().pos = V3::new(-3.0, 0.45, 3.0);
        w.economy.stock = Stockpile::new(100, 0, 0, 3);
        refused(&mut w, a, Request::FeedCampfire { oil: false });
        go(&mut w, a, PlaceableKind::Campfire);
        for _ in 0..4 {
            assert!(!w
                .apply(a, Request::FeedCampfire { oil: false }, &G)
                .iter()
                .any(|e| matches!(e, Event::Denied { .. })));
        }
        assert_eq!(
            (w.economy.campfire.points, w.economy.campfire.level),
            (20, 1)
        );
        assert_eq!(
            w.economy.campfire.fuel, 600.0,
            "240 + 4 x 100, capped at 600"
        );
        let ev = w.apply(a, Request::FeedCampfire { oil: true }, &G);
        assert!(ev.contains(&Event::CampfireLevelUp(2)), "{ev:?}");
        let fire = &w.economy.campfire;
        assert_eq!(
            (fire.points, fire.level, w.economy.stock),
            (25, 2, Stockpile::new(80, 0, 0, 2))
        );
        w.economy.stock = Stockpile::new(4, 0, 0, 0);
        assert_eq!(
            refused(&mut w, a, Request::FeedCampfire { oil: false }),
            "Feeding takes 5 wood"
        );
        w.economy.stock = Stockpile::new(0, 0, 0, 1);
        w.economy.campfire.fuel = 0.0;
        w.apply(a, Request::FeedCampfire { oil: true }, &G);
        assert_eq!(
            w.economy.campfire.fuel, 100.0,
            "feeding relights a dead fire"
        );
        w.economy.stock.oil = 1;
        w.economy.campfire = Campfire {
            level: 5,
            points: 100,
            fuel: FUEL_CAP,
        };
        assert_eq!(
            refused(&mut w, a, Request::FeedCampfire { oil: true }),
            "The fire is already roaring"
        );
    }

    #[test]
    fn light_radius_by_level() {
        let (mut w, _) = world();
        for (level, r) in [(1, 6.0), (3, 10.0), (5, 15.0)] {
            w.economy.campfire.level = level;
            assert_eq!(w.economy.campfire.light_radius(), r);
        }
    }

    #[test]
    fn shredder_takes_debris_and_works_two_seconds_each() {
        let (mut w, a) = world();
        let sac = &mut w.player_mut(a).unwrap().sac;
        sac.add(Plank, 2);
        sac.add(ScrapChunk, 1);
        sac.add(Bandage, 3);
        sac.add(GloomfinCarcass, 1);
        w.player_mut(a).unwrap().pos = V3::new(-3.0, 0.45, 3.0);
        assert_eq!(
            refused(&mut w, a, Request::DepositShredder),
            "Walk up to the shredder"
        );
        go(&mut w, a, PlaceableKind::Shredder);
        w.apply(a, Request::DepositShredder, &G);
        let p = w.player(a).unwrap();
        assert_eq!((p.sac.count(Plank), p.sac.count(Bandage)), (0, 3));
        assert_eq!(w.economy.shredder.queue.len(), 4);
        assert_eq!(
            refused(&mut w, a, Request::DepositShredder),
            "You have no debris"
        );
        run(&mut w, 1.9);
        assert_eq!(w.economy.stock, Stockpile::default());
        run(&mut w, 0.2);
        assert_eq!(w.economy.stock, Stockpile::new(3, 0, 0, 0));
        run(&mut w, 6.0);
        assert_eq!(w.economy.stock, Stockpile::new(6, 2, 3, 0));
        assert!(w.economy.shredder.queue.is_empty());
        assert_eq!(w.economy.shredder.timer, 0.0);
    }

    #[test]
    fn crate_bonuses_go_to_the_depositor_only_while_connected() {
        let (mut w, a) = world();
        w.player_mut(a).unwrap().sac.upgrade(32);
        w.player_mut(a).unwrap().sac.add(FloatingCrate, 100);
        w.economy.campfire.fuel = FUEL_CAP;
        go(&mut w, a, PlaceableKind::Shredder);
        w.apply(a, Request::DepositShredder, &G);
        let ev = run(&mut w, 200.5);
        let s = w.economy.stock;
        assert!(
            (200..=400).contains(&s.wood) && (100..=200).contains(&s.metal),
            "{s:?}"
        );
        let coins: u32 = ev
            .iter()
            .filter_map(|e| match e {
                Event::Gained {
                    player,
                    goods: Goods::Coins(n),
                } if *player == a => Some(*n),
                _ => None,
            })
            .sum();
        let p = w.player(a).unwrap();
        assert!(coins > 0 && p.coins == coins, "{coins} vs {}", p.coins);
        assert!(p.sac.count(Bait) > 0 && p.sac.count(Bait) % 2 == 0);

        w.player_mut(a).unwrap().sac.add(FloatingCrate, 30);
        w.apply(a, Request::DepositShredder, &G);
        let (coins, bait) = (
            w.player(a).unwrap().coins,
            w.player(a).unwrap().sac.count(Bait),
        );
        w.leave(a);
        let ev = run(&mut w, 61.0);
        assert!(w.economy.shredder.queue.is_empty());
        assert!(!ev.iter().any(|e| matches!(e, Event::Gained { .. })));
        let p = w.player(a).unwrap();
        assert_eq!((p.coins, p.sac.count(Bait)), (coins, bait));
    }

    #[test]
    fn crafting_checks_level_cost_and_range() {
        let (mut w, a) = world();
        w.economy.stock = Stockpile::new(100, 100, 100, 100);
        go(&mut w, a, PlaceableKind::Workbench);
        assert_eq!(
            refused(&mut w, a, Request::Craft(RecipeId::Bandage)),
            "Unlocks at campfire level 2"
        );
        assert_eq!(
            refused(&mut w, a, Request::Craft(RecipeId::RaftFloor)),
            "Place it on the raft"
        );
        w.economy.campfire.level = 2;
        w.economy.stock = Stockpile::new(0, 0, 1, 0);
        assert_eq!(
            refused(&mut w, a, Request::Craft(RecipeId::Bandage)),
            "Needs 1 wood, 2 biomass"
        );
        w.economy.stock = Stockpile::new(1, 0, 2, 0);
        w.player_mut(a).unwrap().pos = V3::new(3.0, 0.45, 3.0);
        assert_eq!(
            refused(&mut w, a, Request::Craft(RecipeId::Bandage)),
            "Walk up to the workbench"
        );
        go(&mut w, a, PlaceableKind::Workbench);
        let ev = w.apply(a, Request::Craft(RecipeId::Bandage), &G);
        assert!(ev.contains(&Event::Gained {
            player: a,
            goods: Goods::Item(Bandage, 2)
        }));
        assert_eq!(w.player(a).unwrap().sac.count(Bandage), 2);
        assert_eq!(w.economy.stock, Stockpile::default());
    }

    #[test]
    fn crafting_refuses_full_sacs_and_owned_gear() {
        let (mut w, a) = world();
        w.economy.campfire.level = 5;
        w.economy.stock = Stockpile::new(100, 100, 100, 100);
        go(&mut w, a, PlaceableKind::Workbench);
        w.player_mut(a).unwrap().sac.add(Plank, 80);
        assert_eq!(
            refused(&mut w, a, Request::Craft(RecipeId::Bait)),
            "Your sac is full"
        );
        w.apply(a, Request::Craft(RecipeId::AbyssBlade), &G);
        assert_eq!(w.player(a).unwrap().gear.melee, 4, "gear needs no sac room");
        assert_eq!(w.economy.stock, Stockpile::new(100, 70, 80, 100));
        assert_eq!(
            refused(&mut w, a, Request::Craft(RecipeId::ScrapMachete)),
            "You already have one as good"
        );
        w.apply(a, Request::Craft(RecipeId::CarbonRod), &G);
        refused(&mut w, a, Request::Craft(RecipeId::CarbonRod));
    }

    #[test]
    fn boats_moor_side_by_side_at_the_dock() {
        let (mut w, a) = world();
        w.economy.campfire.level = 5;
        w.economy.stock = Stockpile::new(200, 200, 0, 100);
        go(&mut w, a, PlaceableKind::Workbench);
        for r in [RecipeId::Dinghy, RecipeId::Skiff, RecipeId::MotorLaunch] {
            w.apply(a, Request::Craft(r), &G);
        }
        assert_eq!(w.economy.stock, Stockpile::new(110, 122, 0, 80));
        let pos: Vec<V3> = w.dynamic.boats.values().map(|b| b.pos).collect();
        assert_eq!(pos[0], w.raft.dock_pos());
        assert_eq!(pos.len(), 3);
        assert!(pos[1].dist_xz(pos[0]) >= DOCK_SPACING && pos[2].dist_xz(pos[1]) >= DOCK_SPACING);
        assert!(pos.iter().all(|p| !w.raft.covers_xz(p.x, p.z)));
    }

    #[test]
    fn floors_pay_at_place_time() {
        let (mut w, a) = world();
        w.player_mut(a).unwrap().pos = V3::new(3.0, 0.45, 1.0);
        let floor = |cell| Request::Place {
            recipe: RecipeId::RaftFloor,
            cell,
            rot: 0,
        };
        assert_eq!(refused(&mut w, a, floor((2, 0))), "Needs 4 wood");
        w.economy.stock.wood = 9;
        assert_eq!(
            refused(&mut w, a, floor((3, 0))),
            "A floor must be next to the raft"
        );
        assert_eq!(
            refused(&mut w, a, floor((0, 0))),
            "There is already a tile here"
        );
        assert_eq!(refused(&mut w, a, floor((-3, 1))), "Too far away");
        assert_eq!(
            refused(&mut w, a, floor((i32::MAX, i32::MIN))),
            "A floor must be next to the raft"
        );
        let ev = w.apply(a, floor((2, 0)), &G);
        assert!(
            matches!(
                ev[..],
                [Event::Sound {
                    kind: SoundKind::Place,
                    ..
                }]
            ),
            "{ev:?}"
        );
        assert!(w.raft.has_tile((2, 0)));
        assert_eq!(w.economy.stock.wood, 5);
        let before = w.clone();
        let ev = w.apply(a, floor((3, 0)), &FlatGround::LAND);
        assert!(
            matches!(&ev[..], [Event::Denied { reason, .. }] if reason == "The raft must float on open water")
        );
        assert_eq!(w, before);
        assert_eq!(
            refused(
                &mut w,
                a,
                Request::Place {
                    recipe: RecipeId::Bandage,
                    cell: (3, 0),
                    rot: 0
                }
            ),
            "Craft it at the workbench"
        );
    }

    #[test]
    fn structures_need_level_free_tiles_and_one_radio() {
        let (mut w, a) = world();
        w.economy.stock = Stockpile::new(100, 100, 100, 100);
        w.raft.tiles.extend([(2, 0), (2, 1)]);
        w.player_mut(a).unwrap().pos = V3::new(5.0, 0.45, 2.0);
        let farm = |cell, rot| Request::Place {
            recipe: RecipeId::WoodFarm,
            cell,
            rot,
        };
        assert_eq!(
            refused(&mut w, a, farm((2, 0), 1)),
            "Unlocks at campfire level 3"
        );
        w.economy.campfire.level = 4;
        assert_eq!(
            refused(&mut w, a, farm((2, 0), 0)),
            "It must stand on the raft"
        );
        assert_eq!(
            refused(&mut w, a, farm((0, 0), 0)),
            "Something is already there"
        );
        w.apply(a, farm((2, 0), 1), &G);
        let (_, s) = w.raft.find(PlaceableKind::WoodFarm).unwrap();
        assert_eq!(
            (s.cells(), s.farm().is_some()),
            (vec![(2, 0), (2, 1)], true)
        );
        assert_eq!(w.economy.stock, Stockpile::new(80, 90, 100, 100));
        let radio = |cell| Request::Place {
            recipe: RecipeId::RadioStation,
            cell,
            rot: 0,
        };
        w.apply(a, radio((1, 1)), &G);
        assert_eq!(w.raft.count(PlaceableKind::RadioStation), 1);
        assert_eq!(
            refused(&mut w, a, radio((0, 1))),
            "The raft already has a Radio Station"
        );
        assert_eq!(
            refused(&mut w, a, radio((0, 0))),
            "Something is already there"
        );
    }

    #[test]
    fn railings_go_on_outer_edges() {
        let (mut w, a) = world();
        w.economy.stock.wood = 3;
        w.player_mut(a).unwrap().pos = V3::new(3.0, 0.45, 3.0);
        let rail = |cell, side| Request::PlaceRailing { cell, side };
        assert_eq!(
            refused(&mut w, a, rail((0, 0), Side::South)),
            "Railings go on the raft's outer edge"
        );
        assert_eq!(
            refused(&mut w, a, rail((-2, -2), Side::North)),
            "Too far away"
        );
        w.apply(a, rail((1, 1), Side::South), &G);
        assert_eq!(w.raft.railings.len(), 1);
        assert_eq!(w.economy.stock.wood, 1);
        assert_eq!(
            refused(&mut w, a, rail((1, 1), Side::South)),
            "There is already a railing here"
        );
        assert_eq!(refused(&mut w, a, rail((1, 1), Side::East)), "Needs 2 wood");
        assert_eq!(
            refused(
                &mut w,
                a,
                Request::Place {
                    recipe: RecipeId::Railing,
                    cell: (1, 1),
                    rot: 0
                }
            ),
            "Railings go on a tile edge"
        );
    }

    #[test]
    fn manual_farm_cycle() {
        let (mut w, a) = world();
        let id = add_farm(&mut w, PlaceableKind::WoodFarm);
        let start = w.structure_pos(PlaceableKind::Workbench).unwrap();
        w.player_mut(a).unwrap().pos = start;
        assert_eq!(
            refused(&mut w, a, Request::StartFarm(id)),
            "Walk up to the farm"
        );
        assert_eq!(refused(&mut w, a, Request::StartFarm(999)), "No such farm");
        let campfire = w.raft.find(PlaceableKind::Campfire).unwrap().0;
        assert_eq!(
            refused(&mut w, a, Request::StartFarm(campfire)),
            "No such farm"
        );
        go(&mut w, a, PlaceableKind::WoodFarm);
        run(&mut w, 100.0);
        assert_eq!(
            farm_state(&w, id),
            FarmState::default(),
            "farms wait for a start"
        );
        assert_eq!(
            refused(&mut w, a, Request::CollectFarm(id)),
            "Nothing to collect yet"
        );
        w.apply(a, Request::StartFarm(id), &G);
        assert_eq!(
            refused(&mut w, a, Request::StartFarm(id)),
            "It is already growing"
        );
        run(&mut w, 89.0);
        assert!(!farm_state(&w, id).ready);
        run(&mut w, 1.1);
        assert!(farm_state(&w, id).ready && !farm_state(&w, id).running);
        assert_eq!(
            refused(&mut w, a, Request::StartFarm(id)),
            "Collect the harvest first"
        );
        w.player_mut(a).unwrap().sac.add(Bait, 160);
        assert_eq!(
            refused(&mut w, a, Request::CollectFarm(id)),
            "Your sac is full"
        );
        w.player_mut(a).unwrap().sac.remove(Bait, 20);
        w.apply(a, Request::CollectFarm(id), &G);
        assert_eq!(w.player(a).unwrap().sac.count(PalmBundle), 4);
        assert_eq!(farm_state(&w, id), FarmState::default());
        assert!(w.economy.shredder.queue.is_empty());
    }

    #[test]
    fn survivors_run_farms_into_shredder_and_campfire() {
        let (mut w, a) = world();
        let wood = add_farm(&mut w, PlaceableKind::WoodFarm);
        w.raft.tiles.extend([(2, 1), (3, 1)]);
        let oil = w.alloc_id();
        w.raft
            .structures
            .insert(oil, Structure::new(PlaceableKind::OilFarm, (2, 1), 0));
        w.economy.survivors = vec![1, 2];
        go(&mut w, a, PlaceableKind::WoodFarm);
        assert_eq!(
            refused(&mut w, a, Request::StartFarm(wood)),
            "Your survivors run the farms"
        );
        w.economy.campfire.fuel = 200.0;
        // Rate 1.25: the wood farm takes 72 s, the oil farm 120 s.
        let ev = run(&mut w, 71.9);
        assert!(w.economy.shredder.queue.is_empty() && farm_state(&w, wood).running);
        assert!(!ev.contains(&Event::CampfireOut));
        run(&mut w, 0.2);
        let jobs: Vec<ShredJob> = w.economy.shredder.queue.iter().copied().collect();
        assert_eq!(
            jobs,
            vec![
                ShredJob {
                    kind: PalmBundle,
                    by: None
                };
                4
            ]
        );
        let points = w.economy.campfire.points;
        let fuel = w.economy.campfire.fuel;
        run(&mut w, 48.0);
        assert_eq!(
            w.economy.campfire.points,
            points + 4 * POINTS_PER_OIL,
            "2 barrels = 4 oil"
        );
        assert!(w.economy.campfire.fuel > fuel + 300.0);
        assert!(farm_state(&w, oil).running && !farm_state(&w, oil).ready);
        assert_eq!(
            w.economy.stock,
            Stockpile::new(8, 0, 0, 0),
            "4 bundles shredded"
        );
        assert_eq!(w.player(a).unwrap().sac.count(PalmBundle), 0);
    }

    #[test]
    fn auto_oil_overflows_into_the_stockpile_and_ready_farms_unload() {
        let (mut w, _) = world();
        let id = add_farm(&mut w, PlaceableKind::OilFarm);
        if let Some(f) = w.raft.structures.get_mut(&id).and_then(Structure::farm_mut) {
            f.ready = true;
        }
        w.economy.campfire.fuel = FUEL_CAP;
        w.economy.survivors = vec![1];
        run(&mut w, 0.1);
        assert_eq!(w.economy.stock.oil, 4);
        assert!(farm_state(&w, id).running && !farm_state(&w, id).ready);
    }
}
