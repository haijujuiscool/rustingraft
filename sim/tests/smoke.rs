use raft_sim::{FlatGround, IslandGround, Request, World, V3};

#[test]
fn ten_minutes_two_players_round_trip() {
    let mut w = World::new(42);
    let a = w.join("Ana").unwrap();
    let b = w.join("Bo").unwrap();
    let ground = FlatGround::WATER;
    for tick in 0..(10 * 60 * 60) {
        if tick % 3 == 0 {
            let t = tick as f32 / 60.0;
            let pos = V3::new(t.sin() * 3.0, 0.45, t.cos() * 3.0);
            w.apply(
                a,
                Request::PlayerTransform {
                    pos,
                    yaw: t,
                    in_water: false,
                    sprinting: tick % 2 == 0,
                },
                &ground,
            );
        }
        // Idle players: keep them fed and the fire lit so hunger and night enemies
        // (crawlers board while the fire is out) do not end the run early.
        w.economy.campfire.fuel = 600.0;
        for id in [a, b] {
            w.player_mut(id).unwrap().hunger = 100.0;
        }
        w.step(1.0 / 60.0, &ground);
    }
    assert!(!w.game_over);
    assert_eq!(w.clock.day, 2, "600 s = one full cycle plus 240 s");
    assert!(w.player(b).is_some());
    let json = w.to_json();
    let back = World::from_json(&json).unwrap();
    assert_eq!(back, w);
    assert_eq!(back.to_json(), json);
}

#[test]
fn island_ground_and_every_request_never_panics() {
    use raft_sim::content::*;
    use raft_sim::msg::Hand;
    let mut w = World::new(9);
    let a = w.join("Ana").unwrap();
    let g = IslandGround;
    let reqs = vec![
        Request::Eat,
        Request::Heal,
        Request::Equip {
            slot: 99,
            hand: Hand::Food,
        },
        Request::Unequip(Hand::Heal),
        Request::MoveItem { from: 0, to: 99 },
        Request::DropItem { slot: 3 },
        Request::UseFlare { slot: 0 },
        Request::Cook { slot: None },
        Request::StartRevive(7),
        Request::StopRevive,
        Request::PickUpDebris(12345),
        Request::HarpoonDebris(1),
        Request::HarpoonEnemy(1),
        Request::MeleeHit(2),
        Request::CastLine {
            at: V3::new(f32::NAN, 0.0, 0.0),
        },
        Request::ReelIn { success: true },
        Request::CancelFishing,
        Request::DepositShredder,
        Request::FeedCampfire { oil: true },
        Request::Craft(RecipeId::BeaconCore),
        Request::Place {
            recipe: RecipeId::RaftFloor,
            cell: (i32::MAX, i32::MIN),
            rot: 255,
        },
        Request::PlaceRailing {
            cell: (0, 0),
            side: raft_sim::raft::Side::North,
        },
        Request::StartFarm(0),
        Request::CollectFarm(u32::MAX),
        Request::SaveWorld,
        Request::Sell { slot: usize::MAX },
        Request::SellAllFish,
        Request::Buy {
            shop: Shop::Trader,
            offer: 99,
        },
        Request::ClaimQuest(QuestId::Leviathan),
        Request::OpenChest(200),
        Request::ActivateShrine {
            island: 255,
            shrine: 9,
        },
        Request::DeliverGeneratorMetal,
        Request::InstallBeacon,
        Request::TalkToSurvivor(255),
        Request::BoardBoat(77),
        Request::LeaveBoat,
        Request::BoatTransform {
            pos: V3::new(f32::INFINITY, 0.0, 0.0),
            yaw: f32::NAN,
            speed: -1.0,
        },
    ];
    for r in reqs {
        w.apply(a, r, &g);
        w.step(1.0 / 60.0, &g);
    }
    assert!(World::from_json(&w.to_json()).is_ok());
}
