//! The world root: sections, join/leave, the `step`/`apply` dispatch and helpers shared
//! by every system. Per-system state types live in their own module.

use crate::boats::Boat;
use crate::clock::{self, Clock};
use crate::content::{
    island, island_at, BoatKind, EnemyKind, Goods, IslandId, ItemKind, PlaceableKind, Spot,
    FLARE_RADIUS, INTERACT_RANGE,
};
use crate::debris::{Debris, DebrisSpawn};
use crate::economy::{self, Economy};
use crate::enemies::{self, Enemy, EnemyHome, EnemySpawn};
use crate::islands::{self, Islands, Quests};
use crate::math::V3;
use crate::msg::{deny, Event, Request};
use crate::players::{self, Player};
use crate::raft::{self, Raft};
use crate::rng::Rng;
use crate::trader::{self, Trader};
use crate::{boats, debris, fishing, save, survivors};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type PlayerId = u8;
pub type EntityId = u32;

/// Longest step `World::step` simulates; longer frames are clamped.
pub const MAX_DT: f32 = 0.25;
pub const MAX_NAME_LEN: usize = 20;

/// Terrain height query, implemented by the host with an engine raycast.
pub trait Ground {
    /// Height of solid terrain (islands, rocks, wrecks) at (x, z); None over open water.
    /// The raft is not ground: use `Raft::covers_xz`.
    fn height_at(&self, x: f32, z: f32) -> Option<f32>;
}

/// The same answer everywhere. Test mock.
pub struct FlatGround {
    pub height: Option<f32>,
}

impl FlatGround {
    pub const WATER: FlatGround = FlatGround { height: None };
    pub const LAND: FlatGround = FlatGround { height: Some(0.0) };
}

impl Ground for FlatGround {
    fn height_at(&self, _x: f32, _z: f32) -> Option<f32> {
        self.height
    }
}

/// Every island as a flat disc 1 m above the sea; water elsewhere. Test/headless mock.
pub struct IslandGround;

impl Ground for IslandGround {
    fn height_at(&self, x: f32, z: f32) -> Option<f32> {
        island_at(x, z).map(|_| 1.0)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct World {
    pub seed: u64,
    pub rng: Rng,
    pub next_id: EntityId,
    /// Every connected player is downed; requests are refused and `step` does nothing.
    pub game_over: bool,
    pub clock: Clock,
    pub raft: Raft,
    pub economy: Economy,
    pub islands: Islands,
    pub quests: Quests,
    pub trader: Trader,
    /// Entries stay after a player leaves so a returning name gets its things back.
    pub players: BTreeMap<PlayerId, Player>,
    pub dynamic: Dynamic,
}

/// Fast-changing objects, replicated by `DynamicSnapshot`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Dynamic {
    pub debris: BTreeMap<EntityId, Debris>,
    pub enemies: BTreeMap<EntityId, Enemy>,
    pub boats: BTreeMap<EntityId, Boat>,
    /// Debris spawner bookkeeping (debris.rs).
    pub debris_spawn: DebrisSpawn,
    /// Enemy spawner bookkeeping (enemies.rs).
    pub enemy_spawn: EnemySpawn,
}

impl World {
    pub fn new(seed: u64) -> World {
        let mut w = World {
            seed,
            rng: Rng::new(seed),
            next_id: 1,
            game_over: false,
            clock: Clock::default(),
            raft: Raft::default(),
            economy: Economy::new(),
            islands: Islands::new(),
            quests: Quests::new(),
            trader: Trader::new(),
            players: BTreeMap::new(),
            dynamic: Dynamic::default(),
        };
        raft::setup_start(&mut w);
        w
    }

    /// A known name gets its old entry back (same id); a new name gets the smallest free
    /// id. Refuses empty or overlong names and names already connected.
    pub fn join(&mut self, name: &str) -> Result<PlayerId, &'static str> {
        let name = name.trim();
        if name.is_empty()
            || name.chars().count() > MAX_NAME_LEN
            || name.chars().any(char::is_control)
        {
            return Err("bad name");
        }
        if let Some((&id, p)) = self.players.iter_mut().find(|(_, p)| p.name == name) {
            if p.connected {
                return Err("name in use");
            }
            p.connected = true;
            return Ok(id);
        }
        let id = (0..=PlayerId::MAX)
            .find(|id| !self.players.contains_key(id))
            .ok_or("world is full")?;
        self.players
            .insert(id, Player::new(name, players::spawn_pos(id)));
        Ok(id)
    }

    /// Disconnects a player and clears what it was doing. The entry stays.
    pub fn leave(&mut self, id: PlayerId) {
        let Some(p) = self.players.get_mut(&id) else {
            return;
        };
        p.connected = false;
        p.seat = None;
        p.fishing = None;
        p.reviving = None;
        p.revive_progress = 0.0;
        for p in self.players.values_mut().filter(|p| p.reviving == Some(id)) {
            p.reviving = None;
            p.revive_progress = 0.0;
        }
        for d in self
            .dynamic
            .debris
            .values_mut()
            .filter(|d| d.dragged_by == Some(id))
        {
            d.dragged_by = None;
        }
        for e in self
            .dynamic
            .enemies
            .values_mut()
            .filter(|e| e.target == Some(id))
        {
            e.target = None;
        }
    }

    /// Advances the simulation (host only). Non-finite or non-positive `dt` does nothing.
    pub fn step(&mut self, dt: f32, ground: &dyn Ground) -> Vec<Event> {
        let mut ev = Vec::new();
        if !(dt.is_finite() && dt > 0.0) || self.game_over {
            return ev;
        }
        let dt = dt.min(MAX_DT);
        clock::step(self, dt, &mut ev);
        boats::step(self, dt, ground, &mut ev);
        players::step(self, dt, ground, &mut ev);
        economy::step(self, dt, ground, &mut ev);
        debris::step(self, dt, ground, &mut ev);
        fishing::step(self, dt, ground, &mut ev);
        enemies::step(self, dt, ground, &mut ev);
        islands::step(self, dt, ground, &mut ev);
        survivors::step(self, dt, ground, &mut ev);
        trader::step(self, dt, ground, &mut ev);
        ev
    }

    /// Applies one player action (host only). A refused request yields `Event::Denied`
    /// and changes nothing.
    pub fn apply(&mut self, pid: PlayerId, req: Request, ground: &dyn Ground) -> Vec<Event> {
        let mut ev = Vec::new();
        let Some(p) = self.players.get(&pid).filter(|p| p.connected) else {
            deny(&mut ev, pid, "Unknown player");
            return ev;
        };
        let transform = matches!(req, Request::PlayerTransform { .. });
        if self.game_over {
            if !transform {
                deny(&mut ev, pid, "The game is over");
            }
            return ev;
        }
        if p.downed && !transform {
            deny(&mut ev, pid, "You are downed");
            return ev;
        }
        let g = ground;
        let ev_ = &mut ev;
        use Request as R;
        match req {
            R::PlayerTransform { .. }
            | R::Eat
            | R::Heal
            | R::Equip { .. }
            | R::Unequip(_)
            | R::MoveItem { .. }
            | R::DropItem { .. }
            | R::UseFlare { .. }
            | R::Cook { .. }
            | R::StartRevive(_)
            | R::StopRevive
            | R::DropLast => players::handle(self, pid, req, g, ev_),
            R::PickUpDebris(_) | R::HarpoonDebris(_) => debris::handle(self, pid, req, g, ev_),
            R::HarpoonEnemy(_) | R::MeleeHit(_) => enemies::handle(self, pid, req, g, ev_),
            R::CastLine { .. } | R::ReelIn { .. } | R::CancelFishing => {
                fishing::handle(self, pid, req, g, ev_)
            }
            R::DepositShredder
            | R::FeedCampfire { .. }
            | R::Craft(_)
            | R::Place { .. }
            | R::PlaceRailing { .. }
            | R::StartFarm(_)
            | R::CollectFarm(_) => economy::handle(self, pid, req, g, ev_),
            R::SaveWorld => save::handle(self, pid, req, g, ev_),
            R::Sell { .. } | R::SellAllFish | R::Buy { .. } => {
                trader::handle(self, pid, req, g, ev_)
            }
            R::ClaimQuest(_)
            | R::OpenChest(_)
            | R::ActivateShrine { .. }
            | R::DeliverGeneratorMetal
            | R::InstallBeacon => islands::handle(self, pid, req, g, ev_),
            R::TalkToSurvivor(_) => survivors::handle(self, pid, req, g, ev_),
            R::BoardBoat(_) | R::LeaveBoat | R::BoatTransform { .. } => {
                boats::handle(self, pid, req, g, ev_)
            }
        }
        ev
    }

    /// Restores a saved world: everyone starts disconnected and the game is running.
    pub fn load(json: &str) -> Result<World, String> {
        let mut w = World::from_json(json)?;
        let ids: Vec<PlayerId> = w.players.keys().copied().collect();
        for id in ids {
            w.leave(id);
        }
        w.game_over = false;
        Ok(w)
    }

    // ---------------------------------------------------------------- helpers

    /// Simulated seconds since the world was created; use for timestamps and cooldowns.
    pub fn now(&self) -> f64 {
        self.clock.elapsed
    }

    pub fn alloc_id(&mut self) -> EntityId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn player(&self, id: PlayerId) -> Option<&Player> {
        self.players.get(&id)
    }

    pub fn player_mut(&mut self, id: PlayerId) -> Option<&mut Player> {
        self.players.get_mut(&id)
    }

    pub fn connected(&self) -> impl Iterator<Item = (PlayerId, &Player)> + '_ {
        self.players
            .iter()
            .filter(|(_, p)| p.connected)
            .map(|(id, p)| (*id, p))
    }

    /// Player position, or the origin for an unknown id.
    pub fn player_pos(&self, id: PlayerId) -> V3 {
        self.players.get(&id).map_or(V3::ZERO, |p| p.pos)
    }

    /// Centre of the first structure of `kind`.
    pub fn structure_pos(&self, kind: PlaceableKind) -> Option<V3> {
        self.raft.find(kind).map(|(_, s)| s.center())
    }

    pub fn campfire_pos(&self) -> Option<V3> {
        self.structure_pos(PlaceableKind::Campfire)
    }

    /// Within the burning campfire's light radius (horizontal distance).
    pub fn in_campfire_light(&self, pos: V3) -> bool {
        let r = self.economy.campfire.light_radius();
        r > 0.0 && self.campfire_pos().is_some_and(|c| c.dist_xz(pos) <= r)
    }

    /// Within FLARE_RADIUS of a connected player holding a burning flare.
    pub fn in_flare_light(&self, pos: V3) -> bool {
        let now = self.now();
        self.connected()
            .any(|(_, p)| p.flare_until > now && p.pos.dist_xz(pos) <= FLARE_RADIUS)
    }

    /// Safe from Gloomfin light: campfire or flare.
    pub fn in_light(&self, pos: V3) -> bool {
        self.in_campfire_light(pos) || self.in_flare_light(pos)
    }

    /// Structure of `kind` within INTERACT_RANGE (horizontal) of the player.
    pub fn near_structure(&self, pid: PlayerId, kind: PlaceableKind) -> Option<EntityId> {
        let pos = self.player(pid)?.pos;
        self.raft
            .structures
            .iter()
            .find(|(_, s)| s.kind == kind && s.center().dist_xz(pos) <= INTERACT_RANGE)
            .map(|(id, _)| *id)
    }

    /// Player within `range` (horizontal) of an island interaction spot.
    pub fn near_spot(&self, pid: PlayerId, isl: IslandId, spot: Spot, range: f32) -> bool {
        (isl as usize) < crate::content::ISLANDS.len()
            && self
                .player(pid)
                .is_some_and(|p| p.pos.dist_xz(island(isl).spot(spot)) <= range)
    }

    /// Gives one lot of goods; all or nothing. False (nothing changed) if the items do
    /// not fit in the sac or the player is unknown.
    pub fn give(&mut self, pid: PlayerId, goods: Goods, ev: &mut Vec<Event>) -> bool {
        self.give_all(pid, &[goods], ev)
    }

    /// Gives every lot or none. Sac upgrades in the list apply before items are packed.
    pub fn give_all(&mut self, pid: PlayerId, goods: &[Goods], ev: &mut Vec<Event>) -> bool {
        let Some(p) = self.players.get(&pid) else {
            return false;
        };
        let mut sac = p.sac.clone();
        for g in goods {
            if let Goods::Sac(c) = g {
                sac.upgrade(*c as usize);
            }
        }
        for g in goods {
            if let Goods::Item(kind, n) = g {
                if !sac.add_all(*kind, *n) {
                    return false;
                }
            }
        }
        let p = self.players.get_mut(&pid).expect("checked above");
        p.sac = sac;
        for g in goods {
            match *g {
                Goods::Item(..) | Goods::Sac(_) => {}
                Goods::Coins(n) => p.coins = p.coins.saturating_add(n),
                Goods::Stock(s) => self.economy.stock.add(&s),
                Goods::Harpoon(t) => p.gear.harpoon = p.gear.harpoon.max(t),
                Goods::Melee(t) => p.gear.melee = p.gear.melee.max(t),
                Goods::Rod(t) => p.gear.rod = p.gear.rod.max(t),
                Goods::BarbedTip => p.gear.barbed_tip = true,
                Goods::LuckyLure => p.gear.lucky_lure = true,
            }
            ev.push(Event::Gained {
                player: pid,
                goods: *g,
            });
        }
        true
    }

    pub fn spawn_debris(&mut self, kind: ItemKind, pos: V3) -> EntityId {
        let id = self.alloc_id();
        self.dynamic
            .debris
            .insert(id, Debris::new(kind, pos, self.clock.elapsed));
        id
    }

    pub fn spawn_enemy(
        &mut self,
        kind: EnemyKind,
        pos: V3,
        home: EnemyHome,
        ev: &mut Vec<Event>,
    ) -> EntityId {
        let id = self.alloc_id();
        self.dynamic.enemies.insert(id, Enemy::new(kind, pos, home));
        ev.push(Event::EnemySpawned { id, kind, pos });
        id
    }

    pub fn spawn_boat(&mut self, kind: BoatKind, pos: V3) -> EntityId {
        let id = self.alloc_id();
        self.dynamic.boats.insert(id, Boat::new(kind, pos));
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{ItemKind::*, Stockpile};

    #[test]
    fn join_reuses_names_and_picks_free_ids() {
        let mut w = World::new(7);
        assert_eq!(w.join("Ana"), Ok(0));
        assert_eq!(w.join("Bo"), Ok(1));
        assert_eq!(w.join("Ana"), Err("name in use"));
        assert_eq!(w.join("  "), Err("bad name"));
        w.player_mut(0).unwrap().coins = 99;
        w.leave(0);
        assert_eq!(w.connected().count(), 1);
        assert_eq!(w.join(" Ana "), Ok(0));
        assert_eq!(w.player(0).unwrap().coins, 99);
        assert_ne!(w.player(0).unwrap().pos, w.player(1).unwrap().pos);
    }

    #[test]
    fn apply_refuses_unknown_disconnected_and_downed() {
        let mut w = World::new(7);
        let g = FlatGround::WATER;
        let denied = |ev: &[Event]| matches!(ev, [Event::Denied { .. }]);
        assert!(denied(&w.apply(3, Request::Eat, &g)));
        let a = w.join("Ana").unwrap();
        w.leave(a);
        assert!(denied(&w.apply(a, Request::Eat, &g)));
        w.join("Ana").unwrap();
        w.player_mut(a).unwrap().downed = true;
        assert!(denied(&w.apply(a, Request::SaveWorld, &g)));
        let before = w.clone();
        let t = Request::PlayerTransform {
            pos: V3::new(5.0, 0.0, 5.0),
            yaw: 0.0,
            in_water: true,
            sprinting: false,
        };
        assert!(
            w.apply(a, t, &g).is_empty(),
            "downed transforms are ignored silently"
        );
        assert_eq!(w, before);
    }

    #[test]
    fn give_all_is_all_or_nothing() {
        let mut w = World::new(7);
        let a = w.join("Ana").unwrap();
        let mut ev = Vec::new();
        assert!(!w.give_all(a, &[Goods::Coins(5), Goods::Item(Plank, 81)], &mut ev));
        assert_eq!(w.player(a).unwrap().coins, 0);
        assert!(ev.is_empty());
        assert!(w.give_all(
            a,
            &[
                Goods::Sac(12),
                Goods::Item(Plank, 81),
                Goods::Coins(5),
                Goods::Melee(3)
            ],
            &mut ev
        ));
        let p = w.player(a).unwrap();
        assert_eq!(
            (p.sac.capacity(), p.sac.count(Plank), p.coins, p.gear.melee),
            (12, 81, 5, 3)
        );
        assert_eq!(ev.len(), 4);
        assert!(w.give(a, Goods::Melee(2), &mut ev));
        assert!(w.give(a, Goods::Stock(Stockpile::new(1, 2, 3, 4)), &mut ev));
        assert_eq!(w.player(a).unwrap().gear.melee, 3, "tiers never go down");
        assert_eq!(w.economy.stock, Stockpile::new(1, 2, 3, 4));
    }

    #[test]
    fn bad_dt_and_light() {
        let mut w = World::new(7);
        assert!(w.step(f32::NAN, &FlatGround::WATER).is_empty());
        assert!(w.step(-1.0, &FlatGround::WATER).is_empty());
        assert_eq!(w.now(), 0.0);
        w.step(10.0, &FlatGround::WATER);
        assert!((w.now() - MAX_DT as f64).abs() < 1e-6);
        let fire = w.campfire_pos().unwrap();
        assert!(w.in_light(fire));
        assert!(!w.in_light(fire + V3::xz(50.0, 0.0)));
        w.economy.campfire.fuel = 0.0;
        assert!(!w.in_light(fire));
    }
}
