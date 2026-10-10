//! Saves the player scenarios load (tests/fixtures/player_*.json, via "Load world").
//! Regenerate after a sim change:
//! `cargo test --bin rustingraft player::fixtures -- --ignored`
//! `fixtures_are_current` fails when a file no longer loads or no longer matches what
//! these builders make, so the fixtures cannot go stale silently.

use raft_sim::content::{BoatKind, EnemyKind, Fish, ItemKind, DECK_Y};
use raft_sim::enemies::EnemyHome;
use raft_sim::{Player, Stack, World, V3};
use std::f32::consts::PI;

/// The name `Session::solo` joins with (default settings), so the save's entry is ours.
const ME: &str = "Sailor";

fn sailor(w: &mut World, pos: V3, yaw: f32) -> &mut Player {
    let id = w.join(ME).unwrap();
    let p = w.player_mut(id).unwrap();
    p.pos = pos;
    p.yaw = yaw;
    p
}

/// 40 wood and a plank floating upstream (west) of the raft: harpoon and build.
fn world() -> World {
    let mut w = World::new(7);
    w.economy.stock.wood = 40;
    w.spawn_debris(ItemKind::Plank, V3::new(-14.0, 0.0, 0.0));
    w
}

/// A Crawler on the deck 2.3 m in front (-Z) of me, on a clear row of tiles.
fn melee() -> World {
    let mut w = World::new(7);
    sailor(&mut w, V3::new(-1.0, DECK_Y, 1.5), 0.0);
    w.spawn_enemy(EnemyKind::Crawler, V3::new(-1.0, DECK_Y, -0.8), EnemyHome::Raft, &mut Vec::new());
    w
}

/// Hungry and hurt, with cooked fish in the food hand and bandages in the heal hand.
fn eat_heal() -> World {
    let mut w = World::new(7);
    let p = sailor(&mut w, V3::new(-1.0, DECK_Y, 1.5), 0.0);
    p.hunger = 40.0;
    p.health = 50.0;
    p.food = Some(Stack::new(ItemKind::CookedFish(Fish::Sardine), 2));
    p.heal = Some(Stack::new(ItemKind::Bandage, 2));
    w
}

/// A Dinghy off the south edge, bow out to sea (+Z), and me on the deck beside it,
/// out of reach of the campfire and logbook.
fn boat() -> World {
    let mut w = World::new(7);
    sailor(&mut w, V3::new(3.0, DECK_Y, 3.6), PI);
    let id = w.spawn_boat(BoatKind::Dinghy, V3::xz(3.0, 7.0));
    w.dynamic.boats.get_mut(&id).unwrap().yaw = PI;
    w
}

/// Afloat 12 m south of Hook Isle's beach (centre z -38, radius 11), facing it.
fn grapple() -> World {
    let mut w = World::new(7);
    let p = sailor(&mut w, V3::new(0.0, -1.2, -15.0), 0.0);
    p.in_water = true;
    w
}

/// On the deck facing a clear row (-Z), 3 bait then 2 planks in the sac: F drops a plank.
fn stash() -> World {
    let mut w = World::new(7);
    let p = sailor(&mut w, V3::new(-1.0, DECK_Y, 1.5), 0.0);
    p.sac.add(ItemKind::Bait, 3);
    p.sac.add(ItemKind::Plank, 2);
    w
}

fn all() -> [(&'static str, World); 6] {
    [
        ("player_world", world()),
        ("player_melee", melee()),
        ("player_eat_heal", eat_heal()),
        ("player_boat", boat()),
        ("player_grapple", grapple()),
        ("player_stash", stash()),
    ]
}

fn path(name: &str) -> String {
    format!("{}/tests/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"))
}

#[test]
#[ignore]
fn write_player_fixtures() {
    for (name, w) in all() {
        std::fs::write(path(name), w.to_json()).unwrap();
    }
}

#[test]
fn fixtures_are_current() {
    for (name, w) in all() {
        let text = std::fs::read_to_string(path(name)).unwrap();
        let saved = World::load(&text).unwrap_or_else(|e| panic!("{name} no longer loads: {e}"));
        let fresh = World::load(&w.to_json()).unwrap();
        assert!(
            saved == fresh,
            "tests/fixtures/{name}.json is stale: cargo test --bin rustingraft player::fixtures -- --ignored"
        );
    }
}
