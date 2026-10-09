//! Long-run progression check: one bot plays the whole game on FlatGround through
//! `World::apply` and `World::step`. It fishes and sells at Barnaby, harpoons debris,
//! shreds it, feeds the campfire to level 5, makes every recipe (all three boats among
//! them), rescues the six survivors once the radio stands and finishes quests 1-7.
//!
//! Shortcuts, on purpose: travel is a teleport (the bot sets its own position, as a
//! client's transform stream would), and the bot is kept fed and healed every step
//! because hunger and combat damage have their own tests. Everything else (yields,
//! prices, unlocks, spawns, quest logic) is the real game.

use raft_sim::content::BoatKind;
use raft_sim::content::{
    harpoon, island, EnemyKind, Fish, IslandId, ItemKind, PlaceableKind, QuestId, RecipeId,
    RecipeOutput, Shop, Spot, CAMPFIRE_MAX_LEVEL, FEED_WOOD, HOOK_ISLE, ISLANDS, LIGHTHOUSE_ATOLL,
    PALM_REEF, QUESTS, RECIPES, RUSTED_WRECK, SMOKING_RIG,
};
use raft_sim::islands::QuestStatus;
use raft_sim::raft::{Raft, Structure};
use raft_sim::survivors::SurvivorState;
use raft_sim::{EntityId, Event, FlatGround, PlayerId, Request, World, V3};

const DT: f32 = 0.25;
const G: FlatGround = FlatGround::WATER;
/// Generous bound on simulated time for the whole game.
const LIMIT_SECONDS: f64 = 4.0 * 3600.0;
/// Floors added east of the start raft, so the farms and the radio have room.
const NEW_FLOORS: [(i32, i32); 4] = [(2, -2), (2, -1), (2, 0), (2, 1)];

struct Bot {
    w: World,
    me: PlayerId,
    made: Vec<RecipeId>,
    floors: usize,
    bitten: bool,
    ending: bool,
    kills: u32,
}

impl Bot {
    fn new() -> Bot {
        let mut w = World::new(2026);
        let me = w.join("Bot").unwrap();
        Bot {
            w,
            me,
            made: Vec::new(),
            floors: 0,
            bitten: false,
            ending: false,
            kills: 0,
        }
    }

    fn now(&self) -> f64 {
        self.w.now()
    }

    fn status(&self) -> String {
        let p = self.w.player(self.me).unwrap();
        let e = &self.w.economy;
        let quests: Vec<_> = QUESTS.iter().map(|q| self.w.quests.status(q.id)).collect();
        format!(
            "t={:.0}s day {} fire L{} ({} pts, {:.0} fuel) stock {:?} coins {} gear {:?} \
             made {:?} quests {:?} survivors {:?} kills {}",
            self.now(),
            self.w.clock.day,
            e.campfire.level,
            e.campfire.points,
            e.campfire.fuel,
            e.stock,
            p.coins,
            p.gear,
            self.made,
            quests,
            e.survivors,
            self.kills
        )
    }

    fn seen(&mut self, ev: &[Event]) {
        for e in ev {
            match e {
                Event::EndingReached => self.ending = true,
                Event::FishBite { player, .. } if *player == self.me => self.bitten = true,
                Event::EnemyKilled { by: Some(by), .. } if *by == self.me => self.kills += 1,
                _ => {}
            }
        }
    }

    /// One step. Keeps the bot fed and healed (see the module doc).
    fn tick(&mut self) {
        let p = self.w.player_mut(self.me).unwrap();
        p.hunger = 100.0;
        p.health = 100.0;
        let ev = self.w.step(DT, &G);
        self.seen(&ev);
        assert!(!self.w.game_over, "game over: {}", self.status());
        assert!(self.now() < LIMIT_SECONDS, "out of time: {}", self.status());
    }

    /// Applies a request; the refusal reason, if any.
    fn act(&mut self, req: Request) -> Option<String> {
        let ev = self.w.apply(self.me, req, &G);
        self.seen(&ev);
        ev.into_iter().find_map(|e| match e {
            Event::Denied { reason, .. } => Some(reason),
            _ => None,
        })
    }

    fn must(&mut self, req: Request) {
        if let Some(why) = self.act(req.clone()) {
            panic!("{req:?} denied: {why}\n{}", self.status());
        }
    }

    fn go(&mut self, pos: V3) {
        let p = self.w.player_mut(self.me).unwrap();
        p.pos = pos;
        p.in_water = false;
    }

    fn go_to(&mut self, i: IslandId, spot: Spot) {
        self.go(island(i).spot(spot).with_y(1.0));
    }

    /// Between the shredder and the campfire, in reach of both.
    fn work_spot(&self) -> V3 {
        let s = self.w.structure_pos(PlaceableKind::Shredder).unwrap();
        s.lerp(self.w.campfire_pos().unwrap(), 0.5)
    }

    fn quest(&self, q: QuestId) -> QuestStatus {
        self.w.quests.status(q)
    }

    fn sac_count(&self, f: impl Fn(ItemKind) -> bool) -> u32 {
        let p = self.w.player(self.me).unwrap();
        p.sac
            .stacks()
            .filter(|(_, s)| f(s.kind))
            .map(|(_, s)| s.count)
            .sum()
    }

    fn free_slots(&self) -> usize {
        let sac = &self.w.player(self.me).unwrap().sac;
        sac.slots.iter().filter(|s| s.is_none()).count()
    }

    // ------------------------------------------------------------ raft life

    /// One step of raft life: fight, harpoon debris, shred, feed the fire, fish, tend
    /// the farms and make the next affordable recipe. With `keep_scrap` the bot only
    /// harpoons Scrap Chunks and keeps them in the sac (quest 5).
    fn work(&mut self, keep_scrap: bool) {
        let home = self.work_spot();
        self.go(home);
        // Gloomfin only hunt swimmers inside the light: take a dip at night to farm
        // biomass, the only source of it.
        self.w.player_mut(self.me).unwrap().in_water = self.w.clock.is_night();
        self.fight_nearby();
        self.harpoon_debris(keep_scrap);
        if !keep_scrap && self.sac_count(ItemKind::is_debris) > 0 {
            self.must(Request::DepositShredder);
        }
        self.feed_fire();
        self.fish();
        if self.wants_barnaby() {
            self.barnaby();
        }
        if !keep_scrap {
            self.tend_farms();
        }
        self.make_next();
        self.rescue_survivors();
        self.tick();
    }

    /// Harpoon (and melee, when close) the nearest enemy within harpoon range.
    fn fight_nearby(&mut self) {
        let p = self.w.player(self.me).unwrap();
        let (pos, range) = (p.pos, harpoon(p.gear.harpoon).range);
        let target = self
            .w
            .dynamic
            .enemies
            .iter()
            .map(|(&id, e)| (id, e.pos.dist_xz(pos)))
            .filter(|&(_, d)| d <= range)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((id, _)) = target {
            self.strike(id);
        }
    }

    /// Harpoon and swing at `id` when each tool is ready. Out of reach is fine.
    fn strike(&mut self, id: EntityId) {
        let now = self.now();
        if now >= self.w.player(self.me).unwrap().harpoon_ready_at {
            let _ = self.act(Request::HarpoonEnemy(id));
        }
        let ready = now >= self.w.player(self.me).unwrap().melee_ready_at;
        if ready && self.w.dynamic.enemies.contains_key(&id) {
            let _ = self.act(Request::MeleeHit(id));
        }
    }

    fn harpoon_debris(&mut self, only_scrap: bool) {
        let p = self.w.player(self.me).unwrap();
        if self.now() < p.harpoon_ready_at {
            return;
        }
        let (pos, range) = (p.pos, harpoon(p.gear.harpoon).range);
        let target = self
            .w
            .dynamic
            .debris
            .iter()
            .filter(|(_, d)| d.dragged_by.is_none())
            .filter(|(_, d)| !only_scrap || d.kind == ItemKind::ScrapChunk)
            .filter(|(_, d)| p.sac.room_for(d.kind) > 0)
            .map(|(&id, d)| (id, d.pos.dist_xz(pos)))
            .filter(|&(_, d)| d <= range)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        match target {
            Some((id, d)) if d <= 3.0 => self.must(Request::PickUpDebris(id)),
            Some((id, _)) => self.must(Request::HarpoonDebris(id)),
            None => {}
        }
    }

    /// Wood goes to the fire until level 5, then whenever its fuel runs below half.
    fn feed_fire(&mut self) {
        let fire = &self.w.economy.campfire;
        let wants = fire.level < CAMPFIRE_MAX_LEVEL || fire.fuel < 300.0;
        if wants && self.w.economy.stock.wood >= FEED_WOOD {
            self.must(Request::FeedCampfire { oil: false });
        }
    }

    /// Casts east off the raft and reels in every bite.
    fn fish(&mut self) {
        if std::mem::take(&mut self.bitten) {
            // Refused only with a full sac; the fish then gets away.
            let _ = self.act(Request::ReelIn { success: true });
        }
        let p = self.w.player(self.me).unwrap();
        if p.fishing.is_none() && self.free_slots() > 1 {
            let at = p.pos + V3::xz(10.5, 0.0);
            self.must(Request::CastLine { at });
        }
    }

    fn wants_barnaby(&self) -> bool {
        let fish = self.sac_count(|k| matches!(k, ItemKind::Fish(_)));
        let ready = [QuestId::SellFish, QuestId::CampfireLevel]
            .iter()
            .any(|&q| self.quest(q) == QuestStatus::Ready);
        let goldfin = self.quest(QuestId::BringGoldfin) == QuestStatus::Active
            && self.sac_count(|k| k == ItemKind::Fish(Fish::Goldfin)) > 0;
        ready || goldfin || (fish > 0 && (fish >= 8 || self.free_slots() <= 2))
    }

    /// Claims Barnaby's quests, sells the fish and buys the rod and harpoon upgrades.
    fn barnaby(&mut self) {
        self.go_to(HOOK_ISLE, Spot::Barnaby);
        for q in [QuestId::SellFish, QuestId::CampfireLevel] {
            if self.quest(q) == QuestStatus::Ready {
                self.must(Request::ClaimQuest(q));
            }
        }
        if self.quest(QuestId::BringGoldfin) == QuestStatus::Active
            && self.sac_count(|k| k == ItemKind::Fish(Fish::Goldfin)) > 0
        {
            self.must(Request::ClaimQuest(QuestId::BringGoldfin));
        }
        if self.sac_count(|k| matches!(k, ItemKind::Fish(_))) > 0 {
            self.must(Request::SellAllFish);
        }
        // Barnaby's stock: 0 = Reed Rod (60), 1 = Harpoon T2 (120).
        let p = self.w.player(self.me).unwrap();
        if p.gear.harpoon < 2 && p.coins >= 120 {
            self.must(Request::Buy {
                shop: Shop::Barnaby,
                offer: 1,
            });
        }
        let p = self.w.player(self.me).unwrap();
        if p.gear.rod < 2 && p.coins >= 60 {
            self.must(Request::Buy {
                shop: Shop::Barnaby,
                offer: 0,
            });
        }
        let home = self.work_spot();
        self.go(home);
    }

    /// Manual farming until survivors take over.
    fn tend_farms(&mut self) {
        if !self.w.economy.survivors.is_empty() {
            return;
        }
        let farms: Vec<(EntityId, V3)> = self
            .w
            .raft
            .structures
            .iter()
            .filter(|(_, s)| s.farm().is_some())
            .map(|(&id, s)| (id, s.center()))
            .collect();
        for (id, at) in farms {
            let state = |w: &World| w.raft.structures[&id].farm().unwrap().clone();
            if state(&self.w).ready && self.free_slots() > 1 {
                self.go(at);
                self.must(Request::CollectFarm(id));
            }
            let f = state(&self.w);
            if !f.ready && !f.running {
                self.go(at);
                self.must(Request::StartFarm(id));
            }
        }
    }

    /// Makes the first recipe of the plan that is unlocked, affordable and not made yet.
    fn make_next(&mut self) {
        let e = &self.w.economy;
        if self.floors < NEW_FLOORS.len() && e.stock.covers(&RecipeId::RaftFloor.def().cost) {
            let cell = NEW_FLOORS[self.floors];
            self.go(raft_sim::raft::cell_center((cell.0 - 1, cell.1)));
            self.must(Request::Place {
                recipe: RecipeId::RaftFloor,
                cell,
                rot: 0,
            });
            self.floors += 1;
            if !self.made.contains(&RecipeId::RaftFloor) {
                self.made.push(RecipeId::RaftFloor);
            }
            return;
        }
        let next = RECIPES.iter().find(|d| {
            d.id != RecipeId::RaftFloor
                && !self.made.contains(&d.id)
                && e.unlocked(d.id)
                && e.stock.covers(&d.cost)
        });
        let Some(def) = next else { return };
        match def.output {
            RecipeOutput::Railing => {
                let (cell, side) = self.w.raft.outer_edges()[0];
                self.go(Raft::edge_pos(cell, side));
                self.must(Request::PlaceRailing { cell, side });
            }
            RecipeOutput::Structure(kind) => {
                // Structures wait for the new floors, so all three farms and the radio fit.
                if self.floors < NEW_FLOORS.len() {
                    return;
                }
                let spot = (-3..=3)
                    .flat_map(|x| (-3..=3).map(move |z| (x, z)))
                    .flat_map(|c| [(c, 0u8), (c, 1)])
                    .find(|&(c, r)| self.w.raft.can_place(kind, c, r).is_ok());
                let Some((cell, rot)) = spot else {
                    panic!("no room for {kind:?}\n{}", self.status());
                };
                self.go(Structure::new(kind, cell, rot).center());
                self.must(Request::Place {
                    recipe: def.id,
                    cell,
                    rot,
                });
            }
            RecipeOutput::Boat(_) | RecipeOutput::Give(_) => {
                let at = self.w.structure_pos(PlaceableKind::Workbench).unwrap();
                self.go(at);
                self.must(Request::Craft(def.id));
            }
            RecipeOutput::Floor => unreachable!("floors are placed above"),
        }
        self.made.push(def.id);
    }

    /// Once the radio picks up their signals, fetch every waiting survivor.
    fn rescue_survivors(&mut self) {
        let waiting: Vec<IslandId> = (0..ISLANDS.len() as IslandId)
            .filter(|&i| self.w.islands.get(i).unwrap().survivor == SurvivorState::Waiting)
            .collect();
        for i in waiting {
            self.go_to(i, Spot::Survivor);
            self.must(Request::TalkToSurvivor(i));
        }
    }

    // ------------------------------------------------------------ island quests

    /// Stays next to the first enemy of `kind` and hits it until it dies.
    fn defeat(&mut self, kind: EnemyKind) {
        loop {
            let found = self
                .w
                .dynamic
                .enemies
                .iter()
                .find(|(_, e)| e.kind == kind)
                .map(|(&id, e)| (id, e.pos));
            let Some((id, pos)) = found else { return };
            self.go(pos.with_y(1.0) + V3::xz(1.5, 0.0));
            self.w.player_mut(self.me).unwrap().in_water = kind == EnemyKind::Leviathan;
            self.strike(id);
            if self.w.dynamic.enemies.contains_key(&id) {
                self.tick();
            }
        }
    }

    /// Picks up the floating carcass of `kind` (it drifts off with the current).
    fn pick_up(&mut self, kind: ItemKind) {
        let found = self.w.dynamic.debris.iter().find(|(_, d)| d.kind == kind);
        let (id, pos) = found
            .map(|(&id, d)| (id, d.pos))
            .expect("the carcass floats");
        self.go(pos.with_y(0.0));
        self.must(Request::PickUpDebris(id));
    }

    /// Quest 4: light the three tide shrines, then loot the chest.
    fn palm_reef(&mut self) {
        for s in 0..ISLANDS[PALM_REEF as usize].shrines {
            self.go_to(PALM_REEF, Spot::Shrine(s));
            self.must(Request::ActivateShrine {
                island: PALM_REEF,
                shrine: s,
            });
            self.tick();
        }
        assert_eq!(self.quest(QuestId::TideShrines), QuestStatus::Done);
        self.go_to(PALM_REEF, Spot::Chest);
        self.must(Request::OpenChest(PALM_REEF));
        assert_eq!(self.w.player(self.me).unwrap().gear.harpoon, 3);
    }

    /// Quest 5: harpoon 10 Scrap Chunks into the sac, feed them to the generator.
    fn rusted_wreck(&mut self) {
        while self.sac_count(|k| k == ItemKind::ScrapChunk) < 10 {
            self.work(true);
        }
        self.go_to(RUSTED_WRECK, Spot::Generator);
        self.must(Request::DeliverGeneratorMetal);
        self.tick();
        assert_eq!(self.quest(QuestId::Generator), QuestStatus::Done);
        self.go_to(RUSTED_WRECK, Spot::Chest);
        self.must(Request::OpenChest(RUSTED_WRECK));
        let p = self.w.player(self.me).unwrap();
        assert_eq!(p.sac.capacity(), 24);
    }

    /// Quest 6: defeat the Rig Brute, keep its carcass, loot the chest.
    fn smoking_rig(&mut self) {
        self.go_to(SMOKING_RIG, Spot::Chest);
        // Residents spawn once a player is near.
        self.tick();
        self.defeat(EnemyKind::RigBrute);
        self.pick_up(ItemKind::BruteCarcass);
        self.tick();
        assert_eq!(self.quest(QuestId::RigBrute), QuestStatus::Done);
        let oil = self.w.economy.stock.oil;
        self.go_to(SMOKING_RIG, Spot::Chest);
        self.must(Request::OpenChest(SMOKING_RIG));
        assert_eq!(self.w.player(self.me).unwrap().gear.harpoon, 4);
        assert_eq!(self.w.economy.stock.oil, oil + 20);
    }

    /// Quest 7: install the Beacon Core at night and defeat the Leviathan.
    fn lighthouse(&mut self) {
        while !self.w.clock.is_night() {
            self.work(false);
        }
        self.go_to(LIGHTHOUSE_ATOLL, Spot::Beacon);
        self.must(Request::InstallBeacon);
        while !self.w.islands.get(LIGHTHOUSE_ATOLL).unwrap().boss_defeated {
            // A Leviathan that survives the night dives and comes back the next one.
            self.defeat(EnemyKind::Leviathan);
            if !self.w.islands.get(LIGHTHOUSE_ATOLL).unwrap().boss_defeated {
                self.tick();
            }
        }
        self.tick();
        assert!(self.ending, "EndingReached fired");
        assert_eq!(self.quest(QuestId::Leviathan), QuestStatus::Ready);
        let coins = self.w.player(self.me).unwrap().coins;
        self.must(Request::ClaimQuest(QuestId::Leviathan));
        assert_eq!(self.w.player(self.me).unwrap().coins, coins + 2000);
    }
}

#[test]
fn a_bot_can_finish_the_game() {
    let mut b = Bot::new();
    // Raft life until the fire is at level 5, every recipe is made and Barnaby's
    // quests are done. The radio comes up along the way and the survivors are fetched.
    let barnaby = [
        QuestId::SellFish,
        QuestId::CampfireLevel,
        QuestId::BringGoldfin,
    ];
    while b.made.len() < RECIPES.len()
        || b.w.economy.campfire.level < CAMPFIRE_MAX_LEVEL
        || barnaby.iter().any(|&q| b.quest(q) != QuestStatus::Done)
    {
        b.work(false);
    }
    let raft_done = b.now();
    b.palm_reef();
    b.rusted_wreck();
    b.smoking_rig();
    b.lighthouse();

    let w = &b.w;
    for q in &QUESTS {
        assert_eq!(w.quests.status(q.id), QuestStatus::Done, "{:?}", q.id);
    }
    assert_eq!(w.economy.campfire.level, CAMPFIRE_MAX_LEVEL);
    let mut made = b.made.clone();
    made.sort();
    assert_eq!(made, RECIPES.iter().map(|d| d.id).collect::<Vec<_>>());
    let mut boats: Vec<BoatKind> = w.dynamic.boats.values().map(|b| b.kind).collect();
    boats.sort();
    assert_eq!(
        boats,
        [BoatKind::Dinghy, BoatKind::Skiff, BoatKind::MotorLaunch]
    );
    let stranded: Vec<IslandId> = (0..ISLANDS.len() as IslandId)
        .filter(|&i| ISLANDS[i as usize].survivor)
        .collect();
    assert_eq!(stranded.len(), 6);
    for &i in &stranded {
        assert_eq!(w.islands.get(i).unwrap().survivor, SurvivorState::Rescued);
    }
    assert_eq!(w.economy.survivors.len(), 6);
    let p = w.player(b.me).unwrap();
    assert_eq!((p.gear.harpoon, p.gear.melee), (4, 4));
    eprintln!(
        "progression: raft goals at {:.0} s, ending at {:.0} s (day {}), {} kills",
        raft_done,
        b.now(),
        w.clock.day,
        b.kills
    );
}
