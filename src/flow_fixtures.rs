//! Saves the end-to-end flow scenarios load (tests/fixtures/flow_*.json, via "Load world").
//! Each one seeds the state a step of the game loop starts from, so the scenario only
//! drives the step itself with real inputs. Regenerate after a sim change:
//! `cargo test --bin rustingraft flow_fixtures -- --ignored`

use raft_sim::content::{
    ItemKind, PlaceableKind, CYCLE_SECONDS, DECK_Y, ENEMY_RESPAWN_DAYS, PALM_REEF, TRADER_POOL,
};
use raft_sim::raft::{FarmState, Structure};
use raft_sim::{Player, World, V3};

/// The name `Session::solo` joins with (default settings), so the save's entry is ours.
const ME: &str = "Sailor";

fn sailor(w: &mut World, pos: V3) -> &mut Player {
    let id = w.join(ME).unwrap();
    let p = w.player_mut(id).unwrap();
    p.pos = pos;
    p
}

fn fire(w: &mut World, level: u8) {
    let c = &mut w.economy.campfire;
    c.level = level;
    c.points = [0, 0, 25, 50, 75, 100][level as usize];
    c.fuel = 600.0;
}

/// A plank, a scrap chunk and a crawler carcass floating just north of the raft, west of
/// the current so they drift past the player standing on the north edge.
fn shred() -> World {
    let mut w = World::new(7);
    sailor(&mut w, V3::new(0.0, DECK_Y, -3.6));
    w.spawn_debris(ItemKind::Plank, V3::new(-1.5, 0.0, -5.0));
    w.spawn_debris(ItemKind::ScrapChunk, V3::new(-0.5, 0.0, -5.0));
    w.spawn_debris(ItemKind::CrawlerCarcass, V3::new(0.5, 0.0, -5.0));
    w
}

/// 20 wood and 2 oil in the stockpile, level 1 fire with no points yet.
fn campfire() -> World {
    let mut w = World::new(7);
    sailor(&mut w, V3::new(1.0, DECK_Y, 2.8));
    w.economy.stock.wood = 20;
    w.economy.stock.oil = 2;
    w
}

/// Level 2: bandages are craftable, and wood for a floor tile.
fn craft() -> World {
    let mut w = World::new(7);
    sailor(&mut w, V3::new(-3.0, DECK_Y, -0.2));
    fire(&mut w, 2);
    w.economy.stock.wood = 40;
    w.economy.stock.biomass = 4;
    w
}

/// Level 3 with the wood and metal for a Wood Farm.
fn farm() -> World {
    let mut w = World::new(7);
    sailor(&mut w, V3::new(0.0, DECK_Y, -0.5));
    fire(&mut w, 3);
    w.economy.stock.wood = 40;
    w.economy.stock.metal = 20;
    w
}

/// Level 4 with the cost of a Radio Station, no wood, and a Wood Farm 5 s from its
/// harvest on cells (-1,1)-(0,1).
fn radio() -> World {
    let mut w = World::new(7);
    sailor(&mut w, V3::new(0.0, DECK_Y, -0.5));
    fire(&mut w, 4);
    w.economy.stock.metal = 20;
    w.economy.stock.biomass = 15;
    w.economy.stock.oil = 10;
    let mut farm = Structure::new(PlaceableKind::WoodFarm, (-1, 1), 0);
    *farm.farm_mut().unwrap() = FarmState {
        running: true,
        progress: 85.0,
        ready: false,
    };
    let id = w.alloc_id();
    w.raft.structures.insert(id, farm);
    w
}

/// The radio fixture with the Radio Station already on (0,-2), so every survivor waits,
/// and the player standing by the Palm Reef survivor (Spot::Survivor, -436,-556): a
/// `set` cannot carry the player 700 m, the sim refuses the move. The island's residents
/// were beaten just now, so they stay away while the player talks.
fn rescue() -> World {
    let mut w = radio();
    let id = w.alloc_id();
    w.raft.structures.insert(id, Structure::new(PlaceableKind::RadioStation, (0, -2), 0));
    let me = w.players.keys().copied().next().unwrap();
    w.player_mut(me).unwrap().pos = V3::new(-434.5, 3.2, -556.0);
    let back = (ENEMY_RESPAWN_DAYS as f32 * CYCLE_SECONDS) as f64;
    w.dynamic.enemy_spawn.respawn_at.insert(PALM_REEF, back);
    w
}

/// The trader moored east of the raft with bait, bandages and a flare; 100 coins.
fn trader() -> World {
    let mut w = World::new(7);
    sailor(&mut w, V3::new(3.9, DECK_Y, 0.0)).coins = 100;
    w.trader.present = true;
    w.trader.stock = [3, 4, 5].iter().map(|&i| TRADER_POOL[i]).collect();
    w.trader.dock = w.raft.trader_dock_pos();
    w
}

/// Three seconds before nightfall, swimming 6 m south of the raft, the fire lit.
fn night() -> World {
    let mut w = World::new(7);
    sailor(&mut w, V3::new(0.0, -1.2, 10.0)).in_water = true;
    w.clock.skip_to(297.0);
    w
}

/// Three seconds before nightfall, on deck by a fire with two seconds of fuel left.
fn dark() -> World {
    let mut w = World::new(7);
    sailor(&mut w, V3::new(1.0, DECK_Y, 2.8));
    w.clock.skip_to(297.0);
    w.economy.campfire.fuel = 2.0;
    w
}

/// 300 coins by the logbook: one save at level 1 (225), not a second.
fn save() -> World {
    let mut w = World::new(7);
    sailor(&mut w, V3::new(-3.0, DECK_Y, 1.2)).coins = 300;
    w
}

/// Starving on 1 health.
fn die() -> World {
    let mut w = World::new(7);
    let p = sailor(&mut w, V3::new(-1.0, DECK_Y, 1.0));
    p.hunger = 0.0;
    p.health = 1.0;
    w
}

fn all() -> [(&'static str, World); 11] {
    [
        ("flow_shred", shred()),
        ("flow_campfire", campfire()),
        ("flow_craft", craft()),
        ("flow_farm", farm()),
        ("flow_radio", radio()),
        ("flow_rescue", rescue()),
        ("flow_trader", trader()),
        ("flow_night", night()),
        ("flow_dark", dark()),
        ("flow_save", save()),
        ("flow_die", die()),
    ]
}

fn path(name: &str) -> String {
    format!("{}/tests/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"))
}

#[test]
#[ignore]
fn write_flow_fixtures() {
    for (name, w) in all() {
        std::fs::write(path(name), w.to_json()).unwrap();
    }
}

#[test]
fn flow_fixtures_are_current() {
    for (name, w) in all() {
        let text = std::fs::read_to_string(path(name)).unwrap();
        let saved = World::load(&text).unwrap_or_else(|e| panic!("{name} no longer loads: {e}"));
        let fresh = World::load(&w.to_json()).unwrap();
        assert!(
            saved == fresh,
            "tests/fixtures/{name}.json is stale: cargo test --bin rustingraft flow_fixtures -- --ignored"
        );
    }
}

