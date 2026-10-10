//! Players (spec 6): health, hunger, downed/revive, hands, sac actions, flares, cooking.
//!
//! Rules (`step` / `handle`):
//! - Hunger drains HUNGER_DRAIN/s, x HUNGER_FAST_MULT while sprinting (fast swim = in
//!   water + sprinting). At hunger 0 health drains STARVE_DPS.
//! - Health 0 -> downed (emit PlayerDowned, drop seat/fishing/revive). A reviver holding
//!   StartRevive within REVIVE_RANGE for REVIVE_SECONDS revives at REVIVE_HEALTH
//!   (PlayerRevived). Moving out of range resets progress.
//! - If at least one player is connected and every connected player is downed: set
//!   `World::game_over` and emit GameOver once.
//! - Eat (food hand, raw fish give half), Heal (heal hand), Equip/Unequip/MoveItem,
//!   DropItem (debris kinds float away via `World::spawn_debris`), UseFlare
//!   (flare_until = now + FLARE_SECONDS), Cook (raw fish -> cooked, at the lit campfire
//!   within INTERACT_RANGE).
//! - DropLast: one item of `Sac::last_out` (newest kind first, Quest items stay, hands
//!   never) becomes a world item DROP_AHEAD in front of the player. It rests on the deck
//!   or ground (`debris::rest_y`), else floats like debris. Empty: "Your sac is empty";
//!   MAX_RESTING items already resting and this one would rest: "Too many things lying
//!   around".

use crate::content::{
    ItemClass, ItemKind, PlaceableKind, FLARE_SECONDS, HUNGER_DRAIN, HUNGER_FAST_MULT, MAX_HEALTH,
    MAX_HUNGER, REVIVE_HEALTH, REVIVE_RANGE, REVIVE_SECONDS, SAC_CAPACITIES, STARVE_DPS,
};
use crate::debris::{rest_y, wave, MAX_RESTING};
use crate::fishing::Fishing;
use crate::inventory::{Sac, Stack};
use crate::math::{facing, wrap_angle, V3};
use crate::msg::{deny, sound, toast, Event, Hand, Request, SoundKind, Target};
use crate::raft::{cell_center, SPAWN_CELLS};
use crate::world::{EntityId, Ground, PlayerId, World};
use serde::{Deserialize, Serialize};

/// Positions farther than this from the origin are refused (keeps quantization in range).
pub const MAX_COORD: f32 = 100_000.0;
/// One PlayerTransform may move the player at most this far (absurd teleports are refused).
pub const MAX_MOVE: f32 = 50.0;
/// DropLast puts the item this far in front of the player's feet (m).
pub const DROP_AHEAD: f32 = 1.2;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Player {
    pub name: String,
    pub connected: bool,
    /// Feet position. Client-authoritative while walking/swimming.
    pub pos: V3,
    pub yaw: f32,
    pub in_water: bool,
    pub sprinting: bool,
    pub health: f32,
    pub hunger: f32,
    pub downed: bool,
    /// The downed player this player is holding E on.
    pub reviving: Option<PlayerId>,
    pub revive_progress: f32,
    pub coins: u32,
    pub sac: Sac,
    pub gear: Gear,
    /// Hotbar hand 4.
    pub food: Option<Stack>,
    /// Hotbar hand 5.
    pub heal: Option<Stack>,
    pub seat: Option<Seat>,
    /// `World::now` until which this player's flare burns.
    pub flare_until: f64,
    pub fishing: Option<Fishing>,
    /// `World::now` when the harpoon / melee may be used again.
    pub harpoon_ready_at: f64,
    pub melee_ready_at: f64,
}

/// Hotbar tiers (1-based, see `content::harpoon/melee/rod`) and permanent upgrades.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gear {
    pub harpoon: u8,
    pub melee: u8,
    pub rod: u8,
    pub barbed_tip: bool,
    pub lucky_lure: bool,
}

impl Default for Gear {
    fn default() -> Gear {
        Gear {
            harpoon: 1,
            melee: 1,
            rod: 1,
            barbed_tip: false,
            lucky_lure: false,
        }
    }
}

/// A boat seat; index 0 is the driver.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seat {
    pub boat: EntityId,
    pub index: u8,
}

impl Player {
    pub fn new(name: &str, pos: V3) -> Player {
        Player {
            name: name.to_string(),
            connected: true,
            pos,
            yaw: 0.0,
            in_water: false,
            sprinting: false,
            health: MAX_HEALTH,
            hunger: MAX_HUNGER,
            downed: false,
            reviving: None,
            revive_progress: 0.0,
            coins: 0,
            sac: Sac::new(SAC_CAPACITIES[0] as usize),
            gear: Gear::default(),
            food: None,
            heal: None,
            seat: None,
            flare_until: 0.0,
            fishing: None,
            harpoon_ready_at: 0.0,
            melee_ready_at: 0.0,
        }
    }

    /// Up and about (not downed).
    pub fn is_up(&self) -> bool {
        !self.downed
    }
}

/// Where a player with this id appears on the start raft.
pub fn spawn_pos(id: PlayerId) -> V3 {
    cell_center(SPAWN_CELLS[id as usize % SPAWN_CELLS.len()])
}

pub(crate) fn step(w: &mut World, dt: f32, _g: &dyn Ground, ev: &mut Vec<Event>) {
    let mut fallen = Vec::new();
    for (&id, p) in w
        .players
        .iter_mut()
        .filter(|(_, p)| p.connected && !p.downed)
    {
        let mult = if p.sprinting { HUNGER_FAST_MULT } else { 1.0 };
        p.hunger = (p.hunger - HUNGER_DRAIN * mult * dt).clamp(0.0, MAX_HUNGER);
        if p.hunger <= 0.0 {
            p.health -= STARVE_DPS * dt;
        }
        p.health = p.health.clamp(0.0, MAX_HEALTH);
        if p.health <= 0.0 {
            fallen.push(id);
        }
    }
    for id in fallen {
        down(w, id, ev);
    }
    revive_tick(w, dt, ev);
    check_game_over(w, ev);
}

/// Damage to an up player (enemy attacks). Emits Damage; 0 health downs the player.
pub(crate) fn hurt(w: &mut World, pid: PlayerId, amount: f32, ev: &mut Vec<Event>) {
    let Some(p) = w.players.get_mut(&pid).filter(|p| p.connected && !p.downed) else {
        return;
    };
    p.health = (p.health - amount).clamp(0.0, MAX_HEALTH);
    let (pos, dead) = (p.pos, p.health <= 0.0);
    ev.push(Event::Damage {
        target: Target::Player(pid),
        amount,
        pos,
    });
    sound(ev, SoundKind::Hurt, pos);
    if dead {
        down(w, pid, ev);
        check_game_over(w, ev);
    }
}

/// Lies down where it fell; the toast is the "ping" for the other players.
fn down(w: &mut World, pid: PlayerId, ev: &mut Vec<Event>) {
    let Some(p) = w.players.get_mut(&pid) else {
        return;
    };
    p.health = 0.0;
    p.downed = true;
    p.sprinting = false;
    p.seat = None;
    p.fishing = None;
    p.reviving = None;
    p.revive_progress = 0.0;
    ev.push(Event::PlayerDowned(pid));
    sound(ev, SoundKind::Downed, p.pos);
    toast(ev, None, format!("{} is down!", p.name));
}

/// Revivers within REVIVE_RANGE of their downed target gain progress; anything else
/// (target up or gone, reviver out of range) drops the revive.
fn revive_tick(w: &mut World, dt: f32, ev: &mut Vec<Event>) {
    let revivers: Vec<(PlayerId, PlayerId)> = w
        .connected()
        .filter(|(_, p)| !p.downed)
        .filter_map(|(id, p)| p.reviving.map(|t| (id, t)))
        .collect();
    for (id, target) in revivers {
        let pos = w.player_pos(id);
        let ok = w
            .player(target)
            .is_some_and(|t| t.connected && t.downed && t.pos.dist_xz(pos) <= REVIVE_RANGE);
        let p = w.players.get_mut(&id).expect("listed above");
        if ok {
            p.revive_progress += dt;
        }
        if ok && p.revive_progress < REVIVE_SECONDS {
            continue;
        }
        p.reviving = None;
        p.revive_progress = 0.0;
        if ok {
            let t = w.players.get_mut(&target).expect("checked above");
            t.downed = false;
            t.health = REVIVE_HEALTH;
            ev.push(Event::PlayerRevived(target));
            sound(ev, SoundKind::Revive, t.pos);
        }
    }
}

/// At least one player connected and every connected player downed: game over, once.
fn check_game_over(w: &mut World, ev: &mut Vec<Event>) {
    if !w.game_over && w.connected().next().is_some() && w.connected().all(|(_, p)| p.downed) {
        w.game_over = true;
        ev.push(Event::GameOver);
    }
}

fn hand(p: &mut Player, h: Hand) -> &mut Option<Stack> {
    match h {
        Hand::Food => &mut p.food,
        Hand::Heal => &mut p.heal,
    }
}

fn fits_hand(kind: ItemKind, h: Hand) -> bool {
    match h {
        Hand::Food => kind.hunger().is_some(),
        Hand::Heal => kind.heal().is_some(),
    }
}

fn use_one(stack: &mut Option<Stack>) {
    if let Some(s) = stack {
        s.count -= 1;
        if s.count == 0 {
            *stack = None;
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
    let now = w.now();
    let fire_near =
        w.economy.campfire.lit() && w.near_structure(pid, PlaceableKind::Campfire).is_some();
    let Some(p) = w.players.get_mut(&pid) else {
        return;
    };
    match req {
        Request::PlayerTransform {
            pos,
            yaw,
            in_water,
            sprinting,
        } => {
            if p.downed || p.seat.is_some() {
                return;
            }
            if !pos.is_finite()
                || !yaw.is_finite()
                || pos.x.abs() > MAX_COORD
                || pos.z.abs() > MAX_COORD
                || pos.y.abs() > MAX_COORD
            {
                return deny(ev, pid, "Bad transform");
            }
            // ponytail: a client that got this far off (long lag spike) stays refused
            // until it snaps back to its replicated position; answer with a
            // PlayerTeleport instead if that happens in practice.
            if pos.dist(p.pos) > MAX_MOVE {
                return deny(ev, pid, "Moved too far");
            }
            p.pos = pos;
            p.yaw = wrap_angle(yaw);
            p.in_water = in_water;
            p.sprinting = sprinting;
        }
        Request::Eat => {
            let Some(gain) = p.food.and_then(|s| s.kind.hunger()) else {
                return deny(ev, pid, "Nothing to eat");
            };
            if p.hunger >= MAX_HUNGER {
                return deny(ev, pid, "You are not hungry");
            }
            use_one(&mut p.food);
            p.hunger = (p.hunger + gain).min(MAX_HUNGER);
            sound(ev, SoundKind::Eat, p.pos);
        }
        Request::Heal => {
            let Some(gain) = p.heal.and_then(|s| s.kind.heal()) else {
                return deny(ev, pid, "Nothing to heal with");
            };
            if p.health >= MAX_HEALTH {
                return deny(ev, pid, "You are not hurt");
            }
            use_one(&mut p.heal);
            p.health = (p.health + gain).min(MAX_HEALTH);
            sound(ev, SoundKind::Heal, p.pos);
        }
        Request::Equip { slot, hand: h } => {
            let Some(s) = p.sac.get(slot) else {
                return deny(ev, pid, "Nothing there");
            };
            if !fits_hand(s.kind, h) {
                return deny(ev, pid, "That does not go in that hand");
            }
            let mut held = *hand(p, h);
            p.sac.exchange_with(slot, &mut held);
            *hand(p, h) = held;
        }
        Request::Unequip(h) => {
            let Some(s) = *hand(p, h) else {
                return deny(ev, pid, "That hand is empty");
            };
            if !p.sac.add_all(s.kind, s.count) {
                return deny(ev, pid, "Your sac is full");
            }
            *hand(p, h) = None;
        }
        Request::MoveItem { from, to } => {
            if p.sac.get(from).is_none() || to >= p.sac.capacity() {
                return deny(ev, pid, "Nothing to move");
            }
            p.sac.move_item(from, to);
        }
        Request::DropItem { slot } => {
            let Some(s) = p.sac.get(slot) else {
                return deny(ev, pid, "Nothing there");
            };
            if s.kind.class() == ItemClass::Quest {
                return deny(ev, pid, "You can't drop that");
            }
            p.sac.take(slot);
            if s.kind.is_debris() {
                // One floating piece per item, in a row in front of the player.
                let (pos, dir) = (p.pos.with_y(0.0), facing(p.yaw));
                for i in 0..s.count {
                    w.spawn_debris(s.kind, pos + dir * (1.5 + 0.6 * i as f32));
                }
                sound(ev, SoundKind::Splash, pos);
            }
        }
        Request::UseFlare { slot } => {
            if p.sac.get(slot).map(|s| s.kind) != Some(ItemKind::Flare) {
                return deny(ev, pid, "That is not a flare");
            }
            p.sac.take_from(slot, 1);
            p.flare_until = now + FLARE_SECONDS as f64;
            sound(ev, SoundKind::Flare, p.pos);
        }
        Request::Cook { slot } => {
            let stack = match slot {
                Some(i) => p.sac.slots.get_mut(i).and_then(Option::as_mut),
                None => p.food.as_mut(),
            };
            let Some(s) = stack else {
                return deny(ev, pid, "Nothing to cook");
            };
            let ItemKind::Fish(fish) = s.kind else {
                return deny(ev, pid, "Only raw fish can be cooked");
            };
            if !fire_near {
                return deny(ev, pid, "Stand by the lit campfire");
            }
            // A raw stack (max 5) always fits the cooked stack size (10): cook in place.
            s.kind = ItemKind::CookedFish(fish);
        }
        Request::StartRevive(target) => {
            let pos = p.pos;
            let ok = target != pid
                && w.player(target)
                    .is_some_and(|t| t.connected && t.downed && t.pos.dist_xz(pos) <= REVIVE_RANGE);
            if !ok {
                return deny(ev, pid, "Nobody to revive here");
            }
            let p = w.players.get_mut(&pid).expect("checked above");
            if p.reviving != Some(target) {
                p.reviving = Some(target);
                p.revive_progress = 0.0;
            }
        }
        Request::StopRevive => {
            // Sent on every E release, so stopping while not reviving is not an error.
            p.reviving = None;
            p.revive_progress = 0.0;
        }
        Request::DropLast => {
            let Some(kind) = p.sac.last_out(|k| k.class() == ItemClass::Quest) else {
                return deny(ev, pid, "Your sac is empty");
            };
            let at = p.pos.with_y(0.0) + facing(p.yaw) * DROP_AHEAD;
            let rest = rest_y(&w.raft, g, at.x, at.z);
            if rest.is_some() && w.dynamic.debris.values().filter(|d| d.resting).count() >= MAX_RESTING {
                return deny(ev, pid, "Too many things lying around");
            }
            w.players.get_mut(&pid).expect("checked above").sac.remove(kind, 1);
            let id = w.spawn_debris(kind, at.with_y(rest.unwrap_or_else(|| wave(at.x, at.z, now))));
            if let Some(d) = w.dynamic.debris.get_mut(&id) {
                d.resting = rest.is_some();
            }
            sound(ev, if rest.is_some() { SoundKind::Place } else { SoundKind::Splash }, at);
        }
        _ => deny(ev, pid, "Not a player action"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::DECK_Y;
    use crate::world::FlatGround;

    #[test]
    fn transform_updates_pose_and_rejects_garbage() {
        let mut w = World::new(1);
        let a = w.join("Ana").unwrap();
        let g = FlatGround::WATER;
        let t = |x: f32| Request::PlayerTransform {
            pos: V3::new(x, 0.0, 2.0),
            yaw: 7.0,
            in_water: true,
            sprinting: true,
        };
        assert!(w.apply(a, t(3.0), &g).is_empty());
        let p = w.player(a).unwrap();
        assert_eq!(p.pos, V3::new(3.0, 0.0, 2.0));
        assert!(p.in_water && p.yaw.abs() <= std::f32::consts::PI);
        assert!(matches!(
            w.apply(a, t(f32::NAN), &g)[..],
            [Event::Denied { .. }]
        ));
        assert!(matches!(w.apply(a, t(1e9), &g)[..], [Event::Denied { .. }]));
        assert_eq!(w.player(a).unwrap().pos.x, 3.0);
        w.player_mut(a).unwrap().seat = Some(Seat { boat: 99, index: 0 });
        w.apply(a, t(9.0), &g);
        assert_eq!(
            w.player(a).unwrap().pos.x,
            3.0,
            "seated players follow the boat"
        );
    }

    #[test]
    fn spawn_is_on_deck() {
        let w = World::new(1);
        for id in 0..10 {
            assert!(w.raft.on_deck(spawn_pos(id)));
        }
    }

    const G: FlatGround = FlatGround::WATER;

    /// Applies `req`, expecting exactly one Denied with `reason` and an unchanged world.
    fn denied(w: &mut World, pid: PlayerId, req: Request, reason: &str) {
        let before = w.clone();
        let ev = w.apply(pid, req, &G);
        assert!(
            matches!(&ev[..], [Event::Denied { reason: r, .. }] if r == reason),
            "{ev:?}"
        );
        assert_eq!(*w, before, "denied request changed the world");
    }

    #[test]
    fn hunger_drains_faster_when_sprinting_and_starving_downs() {
        let mut w = World::new(1);
        let a = w.join("Ana").unwrap();
        for _ in 0..4 {
            w.step(0.25, &G);
        }
        let h = w.player(a).unwrap().hunger;
        assert!((h - (MAX_HUNGER - HUNGER_DRAIN)).abs() < 1e-4, "{h}");
        w.player_mut(a).unwrap().sprinting = true;
        w.step(0.25, &G);
        let fast = h - w.player(a).unwrap().hunger;
        assert!((fast - HUNGER_DRAIN * HUNGER_FAST_MULT * 0.25).abs() < 1e-4);

        let p = w.player_mut(a).unwrap();
        (p.hunger, p.health) = (0.0, 0.2);
        let ev = w.step(0.25, &G);
        assert!(w.player(a).unwrap().downed && w.game_over);
        assert!(ev.contains(&Event::PlayerDowned(a)));
        assert_eq!(ev.iter().filter(|e| **e == Event::GameOver).count(), 1);
        assert!(w.step(0.25, &G).is_empty(), "game over freezes the world");
    }

    #[test]
    fn revive_takes_three_seconds_in_range() {
        let mut w = World::new(1);
        let a = w.join("Ana").unwrap();
        let b = w.join("Bo").unwrap();
        let c = w.join("Cy").unwrap();
        let near = w.player(a).unwrap().pos + V3::new(1.5, 0.0, 0.0);
        let p = w.player_mut(b).unwrap();
        (p.pos, p.health, p.downed) = (near, 0.0, true);
        denied(&mut w, a, Request::StartRevive(a), "Nobody to revive here");
        denied(&mut w, a, Request::StartRevive(c), "Nobody to revive here");

        assert!(w.apply(a, Request::StartRevive(b), &G).is_empty());
        for _ in 0..11 {
            w.step(0.25, &G);
        }
        assert!(w.player(b).unwrap().downed && !w.game_over);
        assert!(w.step(0.25, &G).contains(&Event::PlayerRevived(b)));
        let (pa, pb) = (w.player(a).unwrap(), w.player(b).unwrap());
        assert!(!pb.downed && pb.health == REVIVE_HEALTH && pa.reviving.is_none());

        // Out of range: the revive drops and progress resets.
        let p = w.player_mut(b).unwrap();
        (p.health, p.downed) = (0.0, true);
        w.apply(a, Request::StartRevive(b), &G);
        w.step(0.25, &G);
        w.player_mut(b).unwrap().pos = near + V3::new(5.0, 0.0, 0.0);
        denied(&mut w, a, Request::StartRevive(b), "Nobody to revive here");
        w.step(0.25, &G);
        let pa = w.player(a).unwrap();
        assert!(pa.reviving.is_none() && pa.revive_progress == 0.0);
    }

    #[test]
    fn hands_eat_heal_equip() {
        let mut w = World::new(1);
        let a = w.join("Ana").unwrap();
        let sardine = ItemKind::CookedFish(crate::content::Fish::Sardine);
        w.player_mut(a).unwrap().food = Some(Stack::new(sardine, 2));
        denied(&mut w, a, Request::Eat, "You are not hungry");
        w.player_mut(a).unwrap().hunger = 50.0;
        w.apply(a, Request::Eat, &G);
        let p = w.player(a).unwrap();
        assert_eq!(p.hunger, 50.0 + sardine.hunger().unwrap());
        assert_eq!(p.food.unwrap().count, 1);

        denied(&mut w, a, Request::Heal, "Nothing to heal with");
        w.player_mut(a).unwrap().sac.add(ItemKind::Bandage, 1);
        let food = Request::Equip {
            slot: 0,
            hand: Hand::Food,
        };
        denied(&mut w, a, food, "That does not go in that hand");
        w.apply(
            a,
            Request::Equip {
                slot: 0,
                hand: Hand::Heal,
            },
            &G,
        );
        assert!(w.player(a).unwrap().sac.get(0).is_none());
        denied(&mut w, a, Request::Heal, "You are not hurt");
        w.player_mut(a).unwrap().health = 50.0;
        w.apply(a, Request::Heal, &G);
        let p = w.player(a).unwrap();
        assert!(p.health == 80.0 && p.heal.is_none());

        let p = w.player_mut(a).unwrap();
        p.heal = Some(Stack::new(ItemKind::Bandage, 1));
        p.sac.add(ItemKind::Flare, 1000);
        denied(&mut w, a, Request::Unequip(Hand::Heal), "Your sac is full");
    }

    #[test]
    fn sac_move_drop_flare() {
        let mut w = World::new(1);
        let a = w.join("Ana").unwrap();
        let p = w.player_mut(a).unwrap();
        p.sac.add(ItemKind::Plank, 3);
        p.sac.add(ItemKind::BeaconCore, 1);
        p.sac.add(ItemKind::Bandage, 1);
        p.sac.add(ItemKind::Flare, 1);
        let cap = p.sac.capacity();
        denied(
            &mut w,
            a,
            Request::MoveItem { from: 7, to: 0 },
            "Nothing to move",
        );
        denied(
            &mut w,
            a,
            Request::MoveItem { from: 0, to: cap },
            "Nothing to move",
        );
        denied(
            &mut w,
            a,
            Request::DropItem { slot: 1 },
            "You can't drop that",
        );
        denied(
            &mut w,
            a,
            Request::UseFlare { slot: 2 },
            "That is not a flare",
        );

        let n = w.dynamic.debris.len();
        w.apply(a, Request::DropItem { slot: 0 }, &G);
        assert_eq!(w.dynamic.debris.len(), n + 3, "one plank per piece");
        w.apply(a, Request::DropItem { slot: 2 }, &G);
        assert_eq!(w.dynamic.debris.len(), n + 3, "bandages are not debris");

        w.apply(a, Request::UseFlare { slot: 3 }, &G);
        let p = w.player(a).unwrap();
        assert_eq!(p.flare_until, w.now() + FLARE_SECONDS as f64);
        assert_eq!(p.sac.count(ItemKind::Flare), 0);
    }

    #[test]
    fn drop_last_rests_on_deck_or_floats() {
        let mut w = World::new(1);
        let a = w.join("Ana").unwrap();
        w.dynamic.debris_spawn.timer = 1e9;
        w.dynamic.debris.clear();
        let p = w.player_mut(a).unwrap();
        (p.pos, p.yaw) = (V3::new(0.0, DECK_Y, 0.0), 0.0);
        p.heal = Some(Stack::new(ItemKind::Bandage, 1));
        p.sac.add(ItemKind::Plank, 2);
        p.sac.add(ItemKind::BeaconCore, 1);
        p.sac.add(ItemKind::Bait, 1);
        p.sac.add(ItemKind::Plank, 1);
        // Newest kind first: every Plank, then the Bait; the beacon core and hands stay.
        w.apply(a, Request::DropLast, &G);
        let ids: Vec<EntityId> = w.dynamic.debris.keys().copied().collect();
        let d = w.dynamic.debris[&ids[0]].clone();
        assert!(d.kind == ItemKind::Plank && d.resting, "{d:?}");
        assert_eq!(d.pos, V3::new(0.0, DECK_Y, -DROP_AHEAD), "on the deck in front");
        for _ in 0..8 {
            w.step(0.25, &G);
        }
        assert_eq!(w.dynamic.debris[&ids[0]].pos, d.pos, "resting items do not drift");
        assert_eq!(w.player(a).unwrap().sac.count(ItemKind::Plank), 2);
        w.apply(a, Request::DropLast, &G);
        w.apply(a, Request::DropLast, &G);
        w.apply(a, Request::DropLast, &G);
        let kinds: Vec<ItemKind> = w.dynamic.debris.values().map(|d| d.kind).collect();
        assert_eq!(kinds, [ItemKind::Plank, ItemKind::Plank, ItemKind::Plank, ItemKind::Bait]);
        denied(&mut w, a, Request::DropLast, "Your sac is empty");
        let p = w.player(a).unwrap();
        assert!(p.sac.count(ItemKind::BeaconCore) == 1 && p.heal.is_some());
        // Over water it floats and drifts.
        w.player_mut(a).unwrap().pos = V3::new(20.0, 0.0, 0.0);
        w.player_mut(a).unwrap().sac.add(ItemKind::Bandage, 1);
        w.apply(a, Request::DropLast, &G);
        let (&id, d) = w.dynamic.debris.last_key_value().unwrap();
        assert!(d.kind == ItemKind::Bandage && !d.resting && d.pos.y.abs() <= 0.25);
        w.step(0.25, &G);
        assert!((w.dynamic.debris[&id].pos.x - 20.3).abs() < 1e-4);
        // Back in with PickUpDebris.
        w.player_mut(a).unwrap().pos = V3::new(0.0, DECK_Y, 0.0);
        assert!(w.dynamic.debris[&ids[0]].resting);
        w.apply(a, Request::PickUpDebris(ids[0]), &G);
        assert!(!w.dynamic.debris.contains_key(&ids[0]));
        assert_eq!(w.player(a).unwrap().sac.count(ItemKind::Plank), 1);
    }

    #[test]
    fn drop_last_caps_resting_items() {
        let mut w = World::new(1);
        let a = w.join("Ana").unwrap();
        w.dynamic.debris_spawn.timer = 1e9;
        w.dynamic.debris.clear();
        let p = w.player_mut(a).unwrap();
        (p.pos, p.yaw) = (V3::new(0.0, DECK_Y, 0.0), 0.0);
        p.sac.add(ItemKind::Bait, 3);
        for _ in 1..MAX_RESTING {
            let id = w.spawn_debris(ItemKind::Plank, V3::new(0.0, DECK_Y, 1.0));
            w.dynamic.debris.get_mut(&id).unwrap().resting = true;
        }
        w.apply(a, Request::DropLast, &G);
        assert_eq!(w.dynamic.debris.values().filter(|d| d.resting).count(), MAX_RESTING);
        denied(&mut w, a, Request::DropLast, "Too many things lying around");
        // Floating drops age out, so they are not capped.
        w.player_mut(a).unwrap().pos = V3::new(20.0, 0.0, 0.0);
        w.apply(a, Request::DropLast, &G);
        assert_eq!(w.player(a).unwrap().sac.count(ItemKind::Bait), 1);
    }

    #[test]
    fn cook_only_by_the_lit_campfire() {
        let mut w = World::new(1);
        let a = w.join("Ana").unwrap();
        let raw = ItemKind::Fish(crate::content::Fish::Sardine);
        let fire = w.raft.structures.values();
        let fire = fire.filter(|s| s.kind == PlaceableKind::Campfire);
        let fire = fire.map(|s| s.center()).next().unwrap();
        let p = w.player_mut(a).unwrap();
        p.sac.add(raw, 2);
        p.pos = fire + V3::new(10.0, 0.0, 0.0);
        let cook = || Request::Cook { slot: Some(0) };
        denied(&mut w, a, cook(), "Stand by the lit campfire");
        w.player_mut(a).unwrap().pos = fire;
        w.economy.campfire.fuel = 0.0;
        denied(&mut w, a, cook(), "Stand by the lit campfire");
        denied(&mut w, a, Request::Cook { slot: None }, "Nothing to cook");
        w.economy.campfire.fuel = 10.0;
        w.apply(a, cook(), &G);
        let s = w.player(a).unwrap().sac.get(0).unwrap();
        assert_eq!(
            (s.kind, s.count),
            (ItemKind::CookedFish(crate::content::Fish::Sardine), 2)
        );
    }

    #[test]
    fn transform_refuses_teleports() {
        let mut w = World::new(1);
        let a = w.join("Ana").unwrap();
        let start = w.player(a).unwrap().pos;
        let to = |dx: f32| Request::PlayerTransform {
            pos: start + V3::new(dx, 0.0, 0.0),
            yaw: 0.0,
            in_water: true,
            sprinting: false,
        };
        denied(&mut w, a, to(51.0), "Moved too far");
        assert!(w.apply(a, to(49.0), &G).is_empty());
        assert_eq!(w.player(a).unwrap().pos.x, start.x + 49.0);
    }
}
