//! What E acts on: the nearest thing in reach, with its prompt. Resting world items are
//! left to F, so an item set down by a station never takes E from it.

use crate::shared::{Frame, Window};
use raft_sim::content::{
    PlaceableKind, Spot, BOARD_RANGE, DEBRIS_PICKUP_RANGE, GENERATOR_METAL, HOOK_ISLE,
    INTERACT_RANGE, ISLANDS, LIGHTHOUSE_ATOLL, REVIVE_RANGE, RUSTED_WRECK, SHOP_RANGE,
    TRADER_RANGE,
};
use raft_sim::survivors::SurvivorState;
use raft_sim::{PlayerId, Request, V3};

#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    Req(Request),
    Open(Window),
    /// Hold E on this downed player.
    Revive(PlayerId),
    /// Only a prompt ("Growing 40%").
    Nothing,
}

pub struct Target {
    pub text: String,
    pub act: Act,
}

/// The nearest thing E would act on from the feet at `feet`.
pub fn target(frame: &Frame, feet: V3) -> Option<Target> {
    let w = frame.world;
    let me = w.player(frame.me)?;
    if me.seat.is_some() {
        return Some(Target {
            text: "Leave boat".into(),
            act: Act::Req(Request::LeaveBoat),
        });
    }
    let mut best: Option<(f32, Target)> = None;
    let mut offer = |dist: f32, range: f32, text: String, act: Act| {
        if dist <= range && best.as_ref().is_none_or(|(d, _)| dist < *d) {
            best = Some((dist, Target { text, act }));
        }
    };

    for (id, d) in w.dynamic.debris.iter().filter(|(_, d)| !d.resting) {
        let text = format!("Pick up {}", d.kind.name());
        offer(d.pos.dist_xz(feet), DEBRIS_PICKUP_RANGE, text, Act::Req(Request::PickUpDebris(*id)));
    }
    for (id, s) in &w.raft.structures {
        let name = s.kind.name();
        let (text, act) = match s.kind {
            PlaceableKind::Campfire => (name.to_string(), Act::Open(Window::Campfire)),
            PlaceableKind::Workbench => (name.to_string(), Act::Open(Window::Workbench)),
            PlaceableKind::Logbook => (name.to_string(), Act::Open(Window::Logbook)),
            PlaceableKind::RadioStation => (name.to_string(), Act::Open(Window::Radio)),
            PlaceableKind::Shredder => ("Shred debris".into(), Act::Req(Request::DepositShredder)),
            _ => match s.farm() {
                Some(f) if f.ready => (format!("Collect {name}"), Act::Req(Request::CollectFarm(*id))),
                Some(_) if !w.economy.survivors.is_empty() => {
                    (format!("{name}: your survivors tend it"), Act::Nothing)
                }
                Some(f) if f.running => (format!("{name}: growing"), Act::Nothing),
                Some(_) => (format!("Start {name}"), Act::Req(Request::StartFarm(*id))),
                None => continue,
            },
        };
        offer(s.center().dist_xz(feet), INTERACT_RANGE, text, act);
    }

    if let Some(i) = raft_sim::content::island_near(feet.x, feet.z, 10.0) {
        let def = &ISLANDS[i as usize];
        let st = &w.islands.list[i as usize];
        let mut spot = |spot: Spot, range: f32, text: &str, act: Act| {
            offer(def.spot(spot).dist_xz(feet), range, text.into(), act)
        };
        if !def.chest.is_empty() && !st.chest_looted.contains(&frame.me) {
            spot(Spot::Chest, INTERACT_RANGE, "Open chest", Act::Req(Request::OpenChest(i)));
        }
        for (k, lit) in st.shrines.iter().enumerate() {
            if !lit {
                let req = Request::ActivateShrine { island: i, shrine: k as u8 };
                spot(Spot::Shrine(k as u8), INTERACT_RANGE, "Light the shrine", Act::Req(req));
            }
        }
        if i == RUSTED_WRECK && st.generator_metal < GENERATOR_METAL {
            let act = Act::Req(Request::DeliverGeneratorMetal);
            spot(Spot::Generator, INTERACT_RANGE, "Feed scrap to the generator", act);
        }
        if i == LIGHTHOUSE_ATOLL && !w.islands.beacon_installed {
            let act = Act::Req(Request::InstallBeacon);
            spot(Spot::Beacon, INTERACT_RANGE, "Install the beacon core", act);
        }
        if st.survivor == SurvivorState::Waiting {
            let act = Act::Req(Request::TalkToSurvivor(i));
            spot(Spot::Survivor, INTERACT_RANGE, "Talk to the survivor", act);
        }
        if i == HOOK_ISLE {
            spot(Spot::Barnaby, SHOP_RANGE, "Talk to Barnaby", Act::Open(Window::Fisherman));
        }
    }
    if w.trader.present {
        let text = "Trade".to_string();
        offer(w.trader.dock.dist_xz(feet), TRADER_RANGE, text, Act::Open(Window::Trader));
    }
    for (id, b) in &w.dynamic.boats {
        let text = format!("Board {}", b.kind.def().name);
        offer(b.pos.dist_xz(feet), BOARD_RANGE, text, Act::Req(Request::BoardBoat(*id)));
    }
    for (id, p) in w.connected() {
        if id != frame.me && p.downed {
            let text = format!("Hold to revive {}", p.name);
            offer(p.pos.dist_xz(feet), REVIVE_RANGE, text, Act::Revive(id));
        }
    }
    best.map(|(_, t)| t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use raft_sim::World;

    #[test]
    fn resting_items_are_left_to_f() {
        let mut w = World::new(1);
        let me = w.join("Ana").unwrap();
        let feet = V3::xz(60.0, 0.0);
        let id = w.spawn_debris(raft_sim::content::ItemKind::Plank, feet);
        let pick = |w: &World| {
            let frame = Frame { world: w, me, events: &[], dt: 0.0, is_host: true, room_code: None, peers: 0 };
            target(&frame, feet).map(|t| t.act)
        };
        assert_eq!(pick(&w), Some(Act::Req(Request::PickUpDebris(id))));
        w.dynamic.debris.get_mut(&id).unwrap().resting = true;
        assert_eq!(pick(&w), None);
    }
}
