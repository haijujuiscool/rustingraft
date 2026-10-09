//! Paid saving at the Captain's Logbook (spec 9). The sim only charges and signals; the
//! host writes `World::to_json` to disk on `Event::SaveRequested` and loads with
//! `World::load`.

use crate::content::{save_cost, PlaceableKind};
use crate::msg::{deny, sound, toast, Event, Request, SoundKind};
use crate::world::{Ground, PlayerId, World};

pub(crate) fn handle(
    w: &mut World,
    pid: PlayerId,
    req: Request,
    _g: &dyn Ground,
    ev: &mut Vec<Event>,
) {
    let Request::SaveWorld = req else { return };
    if w.near_structure(pid, PlaceableKind::Logbook).is_none() {
        deny(ev, pid, "Walk up to the Captain's Logbook");
        return;
    }
    let cost = save_cost(w.economy.campfire.level);
    let Some(p) = w.players.get_mut(&pid) else {
        return;
    };
    if p.coins < cost {
        deny(ev, pid, format!("Saving costs {cost} coins"));
        return;
    }
    p.coins -= cost;
    let (name, pos) = (p.name.clone(), p.pos);
    w.economy.save_count += 1;
    ev.push(Event::SaveRequested { by: pid });
    sound(ev, SoundKind::Save, pos);
    toast(ev, None, format!("{name} saved the world"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::V3;
    use crate::world::FlatGround;

    #[test]
    fn save_charges_at_the_logbook() {
        let mut w = World::new(1);
        let a = w.join("Ana").unwrap();
        let g = FlatGround::WATER;
        w.player_mut(a).unwrap().coins = 300;
        w.player_mut(a).unwrap().pos = V3::new(3.0, 0.45, -1.0);
        let ev = w.apply(a, Request::SaveWorld, &g);
        assert!(matches!(ev[..], [Event::Denied { .. }]), "too far: {ev:?}");
        w.player_mut(a).unwrap().pos = w.structure_pos(PlaceableKind::Logbook).unwrap();
        let ev = w.apply(a, Request::SaveWorld, &g);
        assert!(ev.contains(&Event::SaveRequested { by: a }));
        assert_eq!((w.player(a).unwrap().coins, w.economy.save_count), (75, 1));
        let ev = w.apply(a, Request::SaveWorld, &g);
        assert!(
            matches!(&ev[..], [Event::Denied { reason, .. }] if reason == "Saving costs 225 coins")
        );
        assert_eq!(w.economy.save_count, 1);
    }

    #[test]
    fn load_disconnects_everyone() {
        let mut w = World::new(1);
        let a = w.join("Ana").unwrap();
        w.player_mut(a).unwrap().coins = 42;
        w.game_over = true;
        let back = World::load(&w.to_json()).unwrap();
        assert_eq!(back.connected().count(), 0);
        assert!(!back.game_over);
        let mut back = back;
        assert_eq!(back.join("Ana"), Ok(a));
        assert_eq!(back.player(a).unwrap().coins, 42);
        assert!(World::load("{").is_err());
        // Tables the systems index by id must match the content.
        let mut bad = World::new(1);
        bad.islands.list.pop();
        assert!(World::load(&bad.to_json()).is_err());
        let mut bad = World::new(1);
        bad.quests.entries.clear();
        assert!(World::load(&bad.to_json()).is_err());
    }

    /// Ana fishes off the east edge and reels in every other bite; Bo sells at Barnaby
    /// and buys bait. Uses only requests, so both copies see the same inputs.
    fn act(w: &mut World, tick: u32, last: &[Event], g: &FlatGround) -> Vec<Event> {
        use crate::content::{island, Fish, ItemKind, Shop, Spot, HOOK_ISLE};
        let (a, b) = (0, 1);
        let mut ev = Vec::new();
        if tick == 0 {
            w.player_mut(b).unwrap().pos = island(HOOK_ISLE).spot(Spot::Barnaby).with_y(1.0);
            w.player_mut(b).unwrap().coins = 100;
        }
        let bitten = last
            .iter()
            .any(|e| matches!(e, Event::FishBite { player, .. } if *player == a));
        if bitten {
            ev.extend(w.apply(
                a,
                Request::ReelIn {
                    success: tick.is_multiple_of(2),
                },
                g,
            ));
        } else if w.player(a).unwrap().fishing.is_none() {
            let at = V3::new(9.0, 0.0, (tick % 7) as f32 - 3.0);
            ev.extend(w.apply(a, Request::CastLine { at }, g));
        }
        if tick.is_multiple_of(97) {
            w.player_mut(b)
                .unwrap()
                .sac
                .add(ItemKind::Fish(Fish::Sardine), 1);
            ev.extend(w.apply(b, Request::SellAllFish, g));
            ev.extend(w.apply(
                b,
                Request::Buy {
                    shop: Shop::Barnaby,
                    offer: 3,
                },
                g,
            ));
        }
        ev
    }

    #[test]
    fn saved_and_loaded_worlds_continue_identically() {
        let g = FlatGround::WATER;
        let mut w = World::new(7);
        w.join("Ana").unwrap();
        w.join("Bo").unwrap();
        let mut last = Vec::new();
        for tick in 0..300 {
            act(&mut w, tick, &last, &g);
            last = w.step(0.25, &g);
        }
        // A JSON copy is the same world and stays the same world.
        let mut copy = World::from_json(&w.to_json()).unwrap();
        assert_eq!(copy, w);
        // A load (everyone disconnected) plus rejoining matches leave + rejoin.
        let mut loaded = World::load(&w.to_json()).unwrap();
        let mut rejoined = w.clone();
        for (id, name) in [(0, "Ana"), (1, "Bo")] {
            rejoined.leave(id);
            assert_eq!(rejoined.join(name), Ok(id));
            assert_eq!(loaded.join(name), Ok(id));
        }
        assert_eq!(loaded, rejoined);

        let mut caught = 0;
        let (mut l1, mut l2, mut l3, mut l4) = (last.clone(), last.clone(), last.clone(), last);
        for tick in 300..1300 {
            let e1 = act(&mut w, tick, &l1, &g);
            let e2 = act(&mut copy, tick, &l2, &g);
            let e3 = act(&mut loaded, tick, &l3, &g);
            let e4 = act(&mut rejoined, tick, &l4, &g);
            assert_eq!(e1, e2, "tick {tick}");
            assert_eq!(e3, e4, "tick {tick}");
            caught += e1
                .iter()
                .filter(|e| matches!(e, Event::Gained { .. }))
                .count();
            (l1, l2, l3, l4) = (
                w.step(0.25, &g),
                copy.step(0.25, &g),
                loaded.step(0.25, &g),
                rejoined.step(0.25, &g),
            );
            assert_eq!(l1, l2, "tick {tick}");
            assert_eq!(l3, l4, "tick {tick}");
        }
        assert_eq!(copy, w);
        assert_eq!(loaded, rejoined);
        assert!(
            caught > 3,
            "the script exercised fishing and the shop: {caught}"
        );
    }
}
