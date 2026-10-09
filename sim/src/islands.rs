//! Islands and quests (spec 4, 12, 13): discovery, chests, shrines, generator, beacon,
//! quest log.
//!
//! - `step`: a connected player within DISCOVER_MARGIN of an island's shore discovers it
//!   (IslandDiscovered). Rig Brute / Leviathan EnemyKilled events mark the boss defeated.
//!   Then `refresh` derives every quest's status from the world state.
//! - Quest flow: Locked -> Active (Start / After(q) Done / Discover(island)) -> Ready
//!   (goal met, waiting for ClaimQuest) -> Done. Island quests 4-6 skip Ready: they are
//!   Done when the goal is met and pay through their chest. Quest 3 stays Active; its
//!   claim takes the Goldfin from the sac.
//! - ClaimQuest: Barnaby quests within SHOP_RANGE of Spot::Barnaby; quest 7 anywhere,
//!   paying every connected player. A Sac reward the claimer already has becomes
//!   QUEST3_OWNED_COINS.
//! - OpenChest: near Spot::Chest, `chest_quest` Done, once per player, all or nothing.
//! - ActivateShrine: near Spot::Shrine(i) (shrines stand on Palm Reef's high rocks).
//! - DeliverGeneratorMetal: Scrap Chunks from the sac count METAL_PER_SCRAP each, up to
//!   GENERATOR_METAL.
//! - InstallBeacon: at night, near Spot::Beacon, consumes the Beacon Core. While the
//!   beacon is lit and the Leviathan undefeated it surfaces every night (`step`); its
//!   death makes quest 7 Ready (EndingReached). The world continues afterwards.
//!
//! Enemy kills made in `World::apply` (harpoon, melee) never reach `step`'s events, so
//! enemies.rs must call `on_enemy_killed` when an enemy dies.

use crate::content::{
    island, EnemyKind, Goods, IslandId, ItemKind, QuestGoal, QuestId, QuestUnlock, Spot,
    DISCOVER_MARGIN, GENERATOR_METAL, HOOK_ISLE, INTERACT_RANGE, ISLANDS, LEVIATHAN_SPAWN_DIST,
    LIGHTHOUSE_ATOLL, METAL_PER_SCRAP, PALM_REEF, QUEST3_OWNED_COINS, QUESTS, RUSTED_WRECK,
    SHOP_RANGE, SMOKING_RIG,
};
use crate::enemies::EnemyHome;
use crate::math::V3;
use crate::msg::{deny, sound, toast, Event, Request, SoundKind};
use crate::survivors::SurvivorState;
use crate::world::{Ground, PlayerId, World};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Islands {
    /// Indexed by IslandId.
    pub list: Vec<IslandState>,
    /// The Beacon Core is installed at Lighthouse Atoll.
    pub beacon_installed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IslandState {
    pub discovered: bool,
    /// Players who already looted this island's chest.
    pub chest_looted: BTreeSet<PlayerId>,
    /// One flag per tide shrine.
    pub shrines: Vec<bool>,
    pub generator_metal: u32,
    pub boss_defeated: bool,
    pub survivor: SurvivorState,
}

impl Islands {
    pub fn new() -> Islands {
        let list = ISLANDS
            .iter()
            .enumerate()
            .map(|(i, def)| IslandState {
                discovered: i == HOOK_ISLE as usize,
                chest_looted: BTreeSet::new(),
                shrines: vec![false; def.shrines as usize],
                generator_metal: 0,
                boss_defeated: false,
                survivor: SurvivorState::Hidden,
            })
            .collect();
        Islands {
            list,
            beacon_installed: false,
        }
    }

    pub fn get(&self, id: IslandId) -> Option<&IslandState> {
        self.list.get(id as usize)
    }

    pub fn get_mut(&mut self, id: IslandId) -> Option<&mut IslandState> {
        self.list.get_mut(id as usize)
    }
}

impl Default for Islands {
    fn default() -> Islands {
        Islands::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuestStatus {
    Locked,
    Active,
    /// Goal met; waiting for ClaimQuest.
    Ready,
    Done,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QuestEntry {
    pub status: QuestStatus,
    /// Goal counter (fish sold, metal delivered...).
    pub progress: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Quests {
    /// Indexed by `QuestId as usize`.
    pub entries: Vec<QuestEntry>,
}

impl Quests {
    pub fn new() -> Quests {
        let entries = QUESTS
            .iter()
            .map(|q| QuestEntry {
                status: if q.unlock == QuestUnlock::Start {
                    QuestStatus::Active
                } else {
                    QuestStatus::Locked
                },
                progress: 0,
            })
            .collect();
        Quests { entries }
    }

    pub fn status(&self, q: QuestId) -> QuestStatus {
        self.entries
            .get(q as usize)
            .map_or(QuestStatus::Locked, |e| e.status)
    }

    pub fn entry_mut(&mut self, q: QuestId) -> &mut QuestEntry {
        &mut self.entries[q as usize]
    }
}

impl Default for Quests {
    fn default() -> Quests {
        Quests::new()
    }
}

/// Island whose quest needs this boss dead.
fn boss_island(kind: EnemyKind) -> Option<IslandId> {
    match kind {
        EnemyKind::RigBrute => Some(SMOKING_RIG),
        EnemyKind::Leviathan => Some(LIGHTHOUSE_ATOLL),
        _ => None,
    }
}

/// Records a boss kill for the quests. Call from wherever an enemy dies; repeats are
/// harmless.
pub(crate) fn on_enemy_killed(w: &mut World, kind: EnemyKind) {
    if let Some(s) = boss_island(kind).and_then(|i| w.islands.get_mut(i)) {
        s.boss_defeated = true;
    }
}

/// Fish sold to Barnaby (trader.rs) count toward quest 1 while it is active.
pub(crate) fn fish_sold(w: &mut World, count: u32, ev: &mut Vec<Event>) {
    let e = w.quests.entry_mut(QuestId::SellFish);
    if e.status == QuestStatus::Active && count > 0 {
        e.progress += count;
        ev.push(Event::QuestUpdated(QuestId::SellFish));
        refresh(w, ev);
    }
}

pub(crate) fn step(w: &mut World, _dt: f32, _g: &dyn Ground, ev: &mut Vec<Event>) {
    let found: Vec<IslandId> = (0..ISLANDS.len() as IslandId)
        .filter(|&i| !w.islands.list[i as usize].discovered)
        .filter(|&i| {
            let def = island(i);
            w.connected()
                .any(|(_, p)| p.pos.dist_xz(def.center()) <= def.radius + DISCOVER_MARGIN)
        })
        .collect();
    for i in found {
        w.islands.list[i as usize].discovered = true;
        ev.push(Event::IslandDiscovered(i));
        toast(ev, None, format!("Discovered {}", island(i).name));
    }
    let killed: Vec<EnemyKind> = ev
        .iter()
        .filter_map(|e| match e {
            Event::EnemyKilled { kind, .. } => Some(*kind),
            _ => None,
        })
        .collect();
    for kind in killed {
        on_enemy_killed(w, kind);
    }
    ensure_leviathan(w, ev);
    refresh(w, ev);
}

/// Current value of a quest's goal counter and whether the goal is met.
fn goal_state(w: &World, goal: QuestGoal, progress: u32) -> (u32, bool) {
    let isl = |i: IslandId| &w.islands.list[i as usize];
    match goal {
        QuestGoal::SellFish(n) => (progress, progress >= n),
        QuestGoal::CampfireLevel(l) => {
            let lv = w.economy.campfire.level;
            (lv as u32, lv >= l)
        }
        QuestGoal::BringItem(_) => (0, false),
        QuestGoal::Shrines(n) => {
            let lit = isl(PALM_REEF).shrines.iter().filter(|s| **s).count() as u32;
            (lit, lit >= n as u32)
        }
        QuestGoal::GeneratorMetal(m) => {
            let metal = isl(RUSTED_WRECK).generator_metal;
            (metal, metal >= m)
        }
        QuestGoal::DefeatBoss(kind) => {
            let dead = boss_island(kind).is_some_and(|i| isl(i).boss_defeated);
            (dead as u32, dead)
        }
        QuestGoal::Ending => {
            let dead = w.islands.beacon_installed && isl(LIGHTHOUSE_ATOLL).boss_defeated;
            (dead as u32, dead)
        }
    }
}

/// Unlocks and completes quests from the world state. Quests only unlock after earlier
/// ones, so one pass in order settles every chain.
fn refresh(w: &mut World, ev: &mut Vec<Event>) {
    for def in &QUESTS {
        let q = def.id;
        let e = w.quests.entries[q as usize].clone();
        let mut next = e.clone();
        if e.status == QuestStatus::Locked {
            let open = match def.unlock {
                QuestUnlock::Start => true,
                QuestUnlock::After(p) => w.quests.status(p) == QuestStatus::Done,
                QuestUnlock::Discover(i) => w.islands.get(i).is_some_and(|s| s.discovered),
            };
            if open {
                next.status = QuestStatus::Active;
                toast(ev, None, format!("New quest: {}", def.title));
            }
        }
        if next.status == QuestStatus::Active {
            let (progress, met) = goal_state(w, def.goal, next.progress);
            next.progress = progress;
            if met {
                // Island quests pay through their chest; the rest wait for ClaimQuest.
                next.status = if def.reward.is_empty() {
                    QuestStatus::Done
                } else {
                    QuestStatus::Ready
                };
                toast(ev, None, format!("Quest complete: {}", def.title));
                if def.goal == QuestGoal::Ending {
                    ev.push(Event::EndingReached);
                }
            }
        }
        if next != e {
            w.quests.entries[q as usize] = next;
            ev.push(Event::QuestUpdated(q));
        }
    }
}

/// While the beacon burns at night the Leviathan is out, until it is defeated.
fn ensure_leviathan(w: &mut World, ev: &mut Vec<Event>) {
    let wanted = w.islands.beacon_installed
        && !w.islands.list[LIGHTHOUSE_ATOLL as usize].boss_defeated
        && w.clock.is_night()
        && !w
            .dynamic
            .enemies
            .values()
            .any(|e| e.kind == EnemyKind::Leviathan);
    if !wanted {
        return;
    }
    let def = island(LIGHTHOUSE_ATOLL);
    let toward_raft = (V3::ZERO - def.center()).normalize();
    let pos = def.center() + toward_raft * (def.radius + LEVIATHAN_SPAWN_DIST);
    w.spawn_enemy(
        EnemyKind::Leviathan,
        pos,
        EnemyHome::Island(LIGHTHOUSE_ATOLL),
        ev,
    );
    sound(ev, SoundKind::BossRoar, pos);
    toast(ev, None, "The Leviathan rises!");
}

pub(crate) fn handle(
    w: &mut World,
    pid: PlayerId,
    req: Request,
    _g: &dyn Ground,
    ev: &mut Vec<Event>,
) {
    let Some(p) = w.players.get(&pid) else { return };
    let pos = p.pos;
    match req {
        Request::ClaimQuest(q) => claim(w, pid, q, ev),
        Request::OpenChest(i) => {
            let Some(def) = ISLANDS.get(i as usize).filter(|d| !d.chest.is_empty()) else {
                return deny(ev, pid, "There is no chest here");
            };
            if !w.near_spot(pid, i, Spot::Chest, INTERACT_RANGE) {
                return deny(ev, pid, "Walk up to the chest");
            }
            if def
                .chest_quest
                .is_some_and(|q| w.quests.status(q) != QuestStatus::Done)
            {
                return deny(ev, pid, "The chest is locked");
            }
            if w.islands.list[i as usize].chest_looted.contains(&pid) {
                return deny(ev, pid, "You already emptied this chest");
            }
            if !w.give_all(pid, def.chest, ev) {
                return deny(ev, pid, "Your sac is full");
            }
            w.islands.list[i as usize].chest_looted.insert(pid);
            sound(ev, SoundKind::ChestOpen, def.spot(Spot::Chest));
        }
        Request::ActivateShrine { island: i, shrine } => {
            let Some(&lit) = w
                .islands
                .get(i)
                .and_then(|s| s.shrines.get(shrine as usize))
            else {
                return deny(ev, pid, "There is no shrine here");
            };
            if !w.near_spot(pid, i, Spot::Shrine(shrine), INTERACT_RANGE) {
                return deny(ev, pid, "Climb up to the shrine");
            }
            if lit {
                return deny(ev, pid, "This shrine already glows");
            }
            let s = &mut w.islands.list[i as usize];
            s.shrines[shrine as usize] = true;
            let (n, total) = (s.shrines.iter().filter(|s| **s).count(), s.shrines.len());
            sound(ev, SoundKind::Shrine, island(i).spot(Spot::Shrine(shrine)));
            toast(ev, None, format!("Tide shrine lit ({n}/{total})"));
            refresh(w, ev);
        }
        Request::DeliverGeneratorMetal => {
            if !w.near_spot(pid, RUSTED_WRECK, Spot::Generator, INTERACT_RANGE) {
                return deny(ev, pid, "Walk up to the generator");
            }
            let metal = w.islands.list[RUSTED_WRECK as usize].generator_metal;
            if metal >= GENERATOR_METAL {
                return deny(ev, pid, "The generator is already running");
            }
            let n = p
                .sac
                .count(ItemKind::ScrapChunk)
                .min((GENERATOR_METAL - metal).div_ceil(METAL_PER_SCRAP));
            if n == 0 {
                return deny(ev, pid, "The generator needs Scrap Chunks");
            }
            let p = w.players.get_mut(&pid).expect("checked above");
            p.sac.remove(ItemKind::ScrapChunk, n);
            let metal = (metal + n * METAL_PER_SCRAP).min(GENERATOR_METAL);
            w.islands.list[RUSTED_WRECK as usize].generator_metal = metal;
            sound(ev, SoundKind::Generator, pos);
            toast(
                ev,
                None,
                format!("Generator: {metal}/{GENERATOR_METAL} metal"),
            );
            refresh(w, ev);
        }
        Request::InstallBeacon => {
            if !w.near_spot(pid, LIGHTHOUSE_ATOLL, Spot::Beacon, INTERACT_RANGE) {
                return deny(ev, pid, "Walk up to the beacon socket");
            }
            if w.islands.beacon_installed {
                return deny(ev, pid, "The beacon is already lit");
            }
            if !w.clock.is_night() {
                return deny(ev, pid, "The beacon can only be lit at night");
            }
            if p.sac.count(ItemKind::BeaconCore) == 0 {
                return deny(ev, pid, "You need a Beacon Core");
            }
            w.players
                .get_mut(&pid)
                .expect("checked above")
                .sac
                .remove(ItemKind::BeaconCore, 1);
            w.islands.beacon_installed = true;
            toast(ev, None, "The beacon is lit");
            ensure_leviathan(w, ev);
            refresh(w, ev);
        }
        _ => {}
    }
}

fn claim(w: &mut World, pid: PlayerId, q: QuestId, ev: &mut Vec<Event>) {
    let def = q.def();
    let status = w.quests.status(q);
    let bring = match def.goal {
        QuestGoal::BringItem(item) => Some(item),
        _ => None,
    };
    let claimable = match bring {
        Some(_) => status == QuestStatus::Active,
        None => status == QuestStatus::Ready,
    };
    if !claimable {
        let why = match status {
            QuestStatus::Locked => "You do not have that quest yet",
            QuestStatus::Done => "That quest is already done",
            _ if def.reward.is_empty() => "Open the chest on the island",
            _ => "That quest is not finished yet",
        };
        return deny(ev, pid, why);
    }
    let ending = def.goal == QuestGoal::Ending;
    if !ending && !w.near_spot(pid, def.giver, Spot::Barnaby, SHOP_RANGE) {
        return deny(ev, pid, "Talk to Barnaby on Hook Isle");
    }
    if ending {
        let all: Vec<PlayerId> = w.connected().map(|(id, _)| id).collect();
        for id in all {
            w.give_all(id, def.reward, ev);
        }
    } else {
        let p = w.players.get(&pid).expect("apply checked the player");
        if bring.is_some_and(|item| p.sac.count(item) == 0) {
            let name = bring.map_or("", |i| i.name());
            return deny(ev, pid, format!("Bring a {name}"));
        }
        let cap = p.sac.capacity();
        let reward: Vec<Goods> = def
            .reward
            .iter()
            .map(|g| match *g {
                Goods::Sac(c) if cap >= c as usize => Goods::Coins(QUEST3_OWNED_COINS),
                g => g,
            })
            .collect();
        let mut sac = p.sac.clone();
        if let Some(item) = bring {
            sac.remove(item, 1);
        }
        let old = std::mem::replace(&mut w.players.get_mut(&pid).expect("checked").sac, sac);
        if !w.give_all(pid, &reward, ev) {
            w.players.get_mut(&pid).expect("checked").sac = old;
            return deny(ev, pid, "Make room in your sac");
        }
    }
    w.quests.entry_mut(q).status = QuestStatus::Done;
    ev.push(Event::QuestUpdated(q));
    sound(ev, SoundKind::Coins, w.player_pos(pid));
    refresh(w, ev);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{Fish, FIRST_ISLET};
    use crate::world::FlatGround;

    const G: FlatGround = FlatGround::WATER;

    fn denied(ev: &[Event]) -> bool {
        matches!(ev, [Event::Denied { .. }])
    }

    fn setup() -> (World, PlayerId) {
        let mut w = World::new(1);
        let a = w.join("Ana").unwrap();
        (w, a)
    }

    fn go(w: &mut World, pid: PlayerId, isl: IslandId, spot: Spot) {
        w.player_mut(pid).unwrap().pos = island(isl).spot(spot).with_y(1.0);
    }

    fn status(w: &World, q: QuestId) -> QuestStatus {
        w.quests.status(q)
    }

    #[test]
    fn start_state() {
        let w = World::new(1);
        assert_eq!(w.islands.list.len(), ISLANDS.len());
        assert!(w.islands.get(0).unwrap().discovered);
        assert_eq!(w.islands.get(PALM_REEF).unwrap().shrines.len(), 3);
        assert_eq!(w.quests.status(QuestId::SellFish), QuestStatus::Active);
        assert_eq!(w.quests.status(QuestId::CampfireLevel), QuestStatus::Locked);
    }

    #[test]
    fn barnaby_quest_chain() {
        let (mut w, a) = setup();
        go(&mut w, a, HOOK_ISLE, Spot::Barnaby);
        let sell = Request::SellAllFish;
        assert!(denied(&w.apply(
            a,
            Request::ClaimQuest(QuestId::SellFish),
            &G
        )));
        w.player_mut(a)
            .unwrap()
            .sac
            .add(ItemKind::Fish(Fish::Sardine), 2);
        w.apply(a, sell.clone(), &G);
        assert_eq!(status(&w, QuestId::SellFish), QuestStatus::Active);
        assert_eq!(w.quests.entries[0].progress, 2);
        w.player_mut(a)
            .unwrap()
            .sac
            .add(ItemKind::CookedFish(Fish::Sardine), 1);
        let ev = w.apply(a, sell.clone(), &G);
        assert!(ev.contains(&Event::QuestUpdated(QuestId::SellFish)));
        assert_eq!(status(&w, QuestId::SellFish), QuestStatus::Ready);
        let coins = w.player(a).unwrap().coins;

        // Claiming far away is refused.
        w.player_mut(a).unwrap().pos = V3::new(0.5, 0.45, 0.5);
        assert!(denied(&w.apply(
            a,
            Request::ClaimQuest(QuestId::SellFish),
            &G
        )));
        go(&mut w, a, HOOK_ISLE, Spot::Barnaby);
        w.apply(a, Request::ClaimQuest(QuestId::SellFish), &G);
        assert_eq!(w.player(a).unwrap().coins, coins + 30);
        assert_eq!(status(&w, QuestId::SellFish), QuestStatus::Done);
        assert!(denied(&w.apply(
            a,
            Request::ClaimQuest(QuestId::SellFish),
            &G
        )));

        // Quest 2 unlocks; it is met when the campfire reaches level 2.
        assert_eq!(status(&w, QuestId::CampfireLevel), QuestStatus::Active);
        assert!(denied(&w.apply(
            a,
            Request::ClaimQuest(QuestId::CampfireLevel),
            &G
        )));
        w.economy.campfire.level = 2;
        w.step(0.1, &G);
        assert_eq!(status(&w, QuestId::CampfireLevel), QuestStatus::Ready);
        w.apply(a, Request::ClaimQuest(QuestId::CampfireLevel), &G);
        let p = w.player(a).unwrap();
        assert_eq!((p.coins, p.sac.count(ItemKind::Bait)), (coins + 70, 5));

        // Quest 3: bring a Goldfin, paid with Sac 12.
        assert_eq!(status(&w, QuestId::BringGoldfin), QuestStatus::Active);
        assert!(denied(&w.apply(
            a,
            Request::ClaimQuest(QuestId::BringGoldfin),
            &G
        )));
        w.player_mut(a)
            .unwrap()
            .sac
            .add(ItemKind::Fish(Fish::Goldfin), 2);
        w.apply(a, Request::ClaimQuest(QuestId::BringGoldfin), &G);
        let p = w.player(a).unwrap();
        assert_eq!(p.sac.capacity(), 12);
        assert_eq!(p.sac.count(ItemKind::Fish(Fish::Goldfin)), 1);
        assert_eq!(status(&w, QuestId::BringGoldfin), QuestStatus::Done);
    }

    #[test]
    fn quest3_pays_coins_when_the_sac_is_big_enough() {
        let (mut w, a) = setup();
        w.quests.entry_mut(QuestId::BringGoldfin).status = QuestStatus::Active;
        go(&mut w, a, HOOK_ISLE, Spot::Barnaby);
        let p = w.player_mut(a).unwrap();
        p.sac.upgrade(16);
        p.sac.add(ItemKind::Fish(Fish::Goldfin), 1);
        w.apply(a, Request::ClaimQuest(QuestId::BringGoldfin), &G);
        let p = w.player(a).unwrap();
        assert_eq!((p.coins, p.sac.capacity()), (QUEST3_OWNED_COINS, 16));
        assert!(p.sac.is_empty());
    }

    #[test]
    fn quest2_reward_needs_room_and_changes_nothing_otherwise() {
        let (mut w, a) = setup();
        w.quests.entry_mut(QuestId::SellFish).status = QuestStatus::Done;
        w.economy.campfire.level = 2;
        w.step(0.1, &G);
        assert_eq!(status(&w, QuestId::CampfireLevel), QuestStatus::Ready);
        go(&mut w, a, HOOK_ISLE, Spot::Barnaby);
        for _ in 0..8 {
            w.player_mut(a).unwrap().sac.add(ItemKind::Flare, 1);
        }
        let before = w.clone();
        assert!(denied(&w.apply(
            a,
            Request::ClaimQuest(QuestId::CampfireLevel),
            &G
        )));
        assert_eq!(w, before);
    }

    #[test]
    fn discovery_unlocks_island_quests() {
        let (mut w, a) = setup();
        let reef = island(PALM_REEF);
        w.player_mut(a).unwrap().pos =
            reef.center() + V3::xz(reef.radius + DISCOVER_MARGIN + 5.0, 0.0);
        w.step(0.1, &G);
        assert!(!w.islands.list[PALM_REEF as usize].discovered);
        w.player_mut(a).unwrap().pos =
            reef.center() + V3::xz(reef.radius + DISCOVER_MARGIN - 5.0, 0.0);
        let ev = w.step(0.1, &G);
        assert!(ev.contains(&Event::IslandDiscovered(PALM_REEF)));
        assert!(ev.contains(&Event::QuestUpdated(QuestId::TideShrines)));
        assert_eq!(status(&w, QuestId::TideShrines), QuestStatus::Active);
        assert!(
            !w.step(0.1, &G)
                .contains(&Event::IslandDiscovered(PALM_REEF)),
            "once"
        );
    }

    #[test]
    fn shrines_unlock_the_palm_reef_chest() {
        let (mut w, a) = setup();
        let shrine = |i| Request::ActivateShrine {
            island: PALM_REEF,
            shrine: i,
        };
        go(&mut w, a, PALM_REEF, Spot::Chest);
        w.step(0.1, &G);
        assert_eq!(status(&w, QuestId::TideShrines), QuestStatus::Active);
        assert!(
            denied(&w.apply(a, Request::OpenChest(PALM_REEF), &G)),
            "locked"
        );
        assert!(denied(&w.apply(a, shrine(0), &G)), "too far");
        assert!(denied(&w.apply(a, shrine(3), &G)), "no such shrine");
        for i in 0..3 {
            go(&mut w, a, PALM_REEF, Spot::Shrine(i));
            assert!(!denied(&w.apply(a, shrine(i), &G)));
            assert!(denied(&w.apply(a, shrine(i), &G)), "already lit");
        }
        assert_eq!(status(&w, QuestId::TideShrines), QuestStatus::Done);
        assert!(denied(&w.apply(
            a,
            Request::ClaimQuest(QuestId::TideShrines),
            &G
        )));
        go(&mut w, a, PALM_REEF, Spot::Chest);
        w.apply(a, Request::OpenChest(PALM_REEF), &G);
        assert_eq!(w.player(a).unwrap().gear.harpoon, 3);
        assert!(
            denied(&w.apply(a, Request::OpenChest(PALM_REEF), &G)),
            "once each"
        );
        // Another player gets their own loot.
        let b = w.join("Bo").unwrap();
        go(&mut w, b, PALM_REEF, Spot::Chest);
        w.apply(b, Request::OpenChest(PALM_REEF), &G);
        assert_eq!(w.player(b).unwrap().gear.harpoon, 3);
    }

    #[test]
    fn chests_never_lower_gear_or_sac() {
        let (mut w, a) = setup();
        w.quests.entry_mut(QuestId::Generator).status = QuestStatus::Done;
        let p = w.player_mut(a).unwrap();
        p.gear.melee = 4;
        p.sac.upgrade(32);
        go(&mut w, a, RUSTED_WRECK, Spot::Chest);
        w.apply(a, Request::OpenChest(RUSTED_WRECK), &G);
        let p = w.player(a).unwrap();
        assert_eq!((p.gear.melee, p.sac.capacity()), (4, 32));
        assert!(w.islands.list[RUSTED_WRECK as usize]
            .chest_looted
            .contains(&a));
    }

    #[test]
    fn islet_chests_and_bad_ids() {
        let (mut w, a) = setup();
        assert!(denied(&w.apply(a, Request::OpenChest(200), &G)));
        assert!(
            denied(&w.apply(a, Request::OpenChest(HOOK_ISLE), &G)),
            "no chest"
        );
        assert!(
            denied(&w.apply(a, Request::OpenChest(FIRST_ISLET), &G)),
            "far"
        );
        go(&mut w, a, FIRST_ISLET, Spot::Chest);
        let ev = w.apply(a, Request::OpenChest(FIRST_ISLET), &G);
        let def = island(FIRST_ISLET);
        let gained: Vec<Goods> = ev
            .iter()
            .filter_map(|e| match e {
                Event::Gained { goods, .. } => Some(*goods),
                _ => None,
            })
            .collect();
        assert_eq!(gained, def.chest);
        // A full sac refuses the items and leaves the chest closed.
        let b = w.join("Bo").unwrap();
        for _ in 0..8 {
            w.player_mut(b).unwrap().sac.add(ItemKind::Flare, 1);
        }
        go(&mut w, b, FIRST_ISLET, Spot::Chest);
        let before = w.clone();
        assert!(denied(&w.apply(b, Request::OpenChest(FIRST_ISLET), &G)));
        assert_eq!(w, before);
        assert!(denied(&w.apply(
            a,
            Request::ActivateShrine {
                island: 255,
                shrine: 0
            },
            &G
        )));
    }

    #[test]
    fn generator_takes_scrap_from_the_sac() {
        let (mut w, a) = setup();
        assert!(
            denied(&w.apply(a, Request::DeliverGeneratorMetal, &G)),
            "far"
        );
        go(&mut w, a, RUSTED_WRECK, Spot::Generator);
        w.step(0.1, &G);
        assert_eq!(status(&w, QuestId::Generator), QuestStatus::Active);
        assert!(
            denied(&w.apply(a, Request::DeliverGeneratorMetal, &G)),
            "no scrap"
        );
        w.player_mut(a).unwrap().sac.add(ItemKind::ScrapChunk, 4);
        w.apply(a, Request::DeliverGeneratorMetal, &G);
        assert_eq!(w.islands.list[RUSTED_WRECK as usize].generator_metal, 8);
        assert_eq!(w.quests.entries[QuestId::Generator as usize].progress, 8);
        w.player_mut(a).unwrap().sac.add(ItemKind::ScrapChunk, 10);
        w.apply(a, Request::DeliverGeneratorMetal, &G);
        assert_eq!(w.islands.list[RUSTED_WRECK as usize].generator_metal, 20);
        assert_eq!(
            w.player(a).unwrap().sac.count(ItemKind::ScrapChunk),
            4,
            "only 6 used"
        );
        assert_eq!(status(&w, QuestId::Generator), QuestStatus::Done);
        assert!(
            denied(&w.apply(a, Request::DeliverGeneratorMetal, &G)),
            "running"
        );
        go(&mut w, a, RUSTED_WRECK, Spot::Chest);
        w.apply(a, Request::OpenChest(RUSTED_WRECK), &G);
        let p = w.player(a).unwrap();
        assert_eq!((p.gear.melee, p.sac.capacity()), (3, 24));
    }

    #[test]
    fn rig_brute_kill_completes_quest6() {
        let (mut w, a) = setup();
        go(&mut w, a, SMOKING_RIG, Spot::Chest);
        w.step(0.1, &G);
        assert_eq!(status(&w, QuestId::RigBrute), QuestStatus::Active);
        assert!(denied(&w.apply(a, Request::OpenChest(SMOKING_RIG), &G)));
        on_enemy_killed(&mut w, EnemyKind::Crawler);
        w.step(0.1, &G);
        assert_eq!(status(&w, QuestId::RigBrute), QuestStatus::Active);
        on_enemy_killed(&mut w, EnemyKind::RigBrute);
        w.step(0.1, &G);
        assert_eq!(status(&w, QuestId::RigBrute), QuestStatus::Done);
        let oil = w.economy.stock.oil;
        w.apply(a, Request::OpenChest(SMOKING_RIG), &G);
        assert_eq!(w.player(a).unwrap().gear.harpoon, 4);
        assert_eq!(w.economy.stock.oil, oil + 20);
    }

    fn at_beacon_at_night() -> (World, PlayerId) {
        let (mut w, a) = setup();
        go(&mut w, a, LIGHTHOUSE_ATOLL, Spot::Beacon);
        w.step(0.1, &G);
        w.clock.skip_to(310.0);
        (w, a)
    }

    #[test]
    fn beacon_rules() {
        let (mut w, a) = at_beacon_at_night();
        assert_eq!(status(&w, QuestId::Leviathan), QuestStatus::Active);
        let before = w.clone();
        assert!(denied(&w.apply(a, Request::InstallBeacon, &G)), "no core");
        w.player_mut(a).unwrap().sac.add(ItemKind::BeaconCore, 1);
        w.clock.skip_to(100.0);
        assert!(denied(&w.apply(a, Request::InstallBeacon, &G)), "daytime");
        w.clock.skip_to(310.0);
        w.player_mut(a).unwrap().pos = V3::new(0.5, 0.45, 0.5);
        assert!(denied(&w.apply(a, Request::InstallBeacon, &G)), "far");
        assert_eq!(w.islands, before.islands);
        go(&mut w, a, LIGHTHOUSE_ATOLL, Spot::Beacon);
        let ev = w.apply(a, Request::InstallBeacon, &G);
        assert!(ev.iter().any(|e| matches!(
            e,
            Event::EnemySpawned {
                kind: EnemyKind::Leviathan,
                ..
            }
        )));
        assert!(w.islands.beacon_installed);
        assert_eq!(w.player(a).unwrap().sac.count(ItemKind::BeaconCore), 0);
        assert!(
            denied(&w.apply(a, Request::InstallBeacon, &G)),
            "already lit"
        );
        // Only one Leviathan at a time.
        w.step(0.1, &G);
        let count = |w: &World| {
            w.dynamic
                .enemies
                .values()
                .filter(|e| e.kind == EnemyKind::Leviathan)
                .count()
        };
        assert_eq!(count(&w), 1);
        let spawn = w.dynamic.enemies.values().next().unwrap().pos;
        let def = island(LIGHTHOUSE_ATOLL);
        assert!((spawn.dist_xz(def.center()) - def.radius - LEVIATHAN_SPAWN_DIST).abs() < 0.1);
    }

    #[test]
    fn leviathan_returns_each_night_until_defeated_then_the_ending() {
        let (mut w, a) = at_beacon_at_night();
        let b = w.join("Bo").unwrap();
        w.player_mut(a).unwrap().sac.add(ItemKind::BeaconCore, 1);
        w.apply(a, Request::InstallBeacon, &G);
        // Something else removed it (despawn at dawn): it comes back at night.
        w.dynamic.enemies.clear();
        w.clock.skip_to(100.0);
        w.step(0.1, &G);
        assert!(w.dynamic.enemies.is_empty(), "not by day");
        w.clock.skip_to(305.0);
        w.step(0.1, &G);
        let (&id, e) = w.dynamic.enemies.iter().next().unwrap();
        assert_eq!(e.kind, EnemyKind::Leviathan);
        // It dies (the enemies system emits EnemyKilled during step).
        let pos = e.pos;
        w.dynamic.enemies.remove(&id);
        let mut ev = vec![Event::EnemyKilled {
            id,
            kind: EnemyKind::Leviathan,
            pos,
            by: Some(a),
        }];
        step(&mut w, 0.1, &G, &mut ev);
        assert!(ev.contains(&Event::EndingReached));
        assert_eq!(status(&w, QuestId::Leviathan), QuestStatus::Ready);
        w.step(1.0, &G);
        assert!(w.dynamic.enemies.is_empty(), "stays dead");
        // Claimed anywhere; every connected player gets 2000 coins.
        w.player_mut(a).unwrap().pos = V3::new(0.5, 0.45, 0.5);
        w.apply(a, Request::ClaimQuest(QuestId::Leviathan), &G);
        assert_eq!(w.player(a).unwrap().coins, 2000);
        assert_eq!(w.player(b).unwrap().coins, 2000);
        assert_eq!(status(&w, QuestId::Leviathan), QuestStatus::Done);
        assert!(denied(&w.apply(
            a,
            Request::ClaimQuest(QuestId::Leviathan),
            &G
        )));
        // The world continues.
        assert!(!w.game_over);
        let t = w.clock.elapsed;
        w.step(1.0, &G);
        assert!(w.clock.elapsed > t);
    }

    #[test]
    fn locked_and_garbage_claims_are_denied() {
        let (mut w, a) = setup();
        go(&mut w, a, HOOK_ISLE, Spot::Barnaby);
        let before = w.clone();
        for q in QuestId::ALL {
            if q != QuestId::SellFish {
                assert!(denied(&w.apply(a, Request::ClaimQuest(q), &G)), "{q:?}");
            }
        }
        assert_eq!(w, before);
        // Downed players cannot do anything here.
        w.player_mut(a).unwrap().downed = true;
        w.player_mut(a)
            .unwrap()
            .sac
            .add(ItemKind::Fish(Fish::Sardine), 1);
        assert!(denied(&w.apply(a, Request::SellAllFish, &G)));
    }
}
