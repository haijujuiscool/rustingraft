//! Survivors (spec 8, 12). Six islands have `IslandDef::survivor`.
//!
//! - `step`: once a Radio Station stands on the raft, every Hidden survivor becomes
//!   Waiting (SurvivorSignal per island, one Radio sound).
//! - TalkToSurvivor(island): within INTERACT_RANGE of Spot::Survivor while Waiting ->
//!   Rescued; the island id joins `Economy::survivors` (they now run the farms);
//!   SurvivorRescued.

use crate::content::{IslandId, PlaceableKind, Spot, INTERACT_RANGE, ISLANDS};
use crate::msg::{deny, sound, toast, Event, Request, SoundKind};
use crate::world::{Ground, PlayerId, World};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SurvivorState {
    /// No radio yet (or the island has no survivor).
    Hidden,
    /// Signal on the radio; waiting on the island.
    Waiting,
    /// Lives on the raft.
    Rescued,
}

pub(crate) fn step(w: &mut World, _dt: f32, _g: &dyn Ground, ev: &mut Vec<Event>) {
    let Some(radio) = w.structure_pos(PlaceableKind::RadioStation) else {
        return;
    };
    let before = ev.len();
    for (id, (st, def)) in w.islands.list.iter_mut().zip(&ISLANDS).enumerate() {
        if def.survivor && st.survivor == SurvivorState::Hidden {
            st.survivor = SurvivorState::Waiting;
            ev.push(Event::SurvivorSignal(id as IslandId));
        }
    }
    if ev.len() > before {
        sound(ev, SoundKind::Radio, radio);
        toast(ev, None, "The radio picked up survivors' signals");
    }
}

pub(crate) fn handle(
    w: &mut World,
    pid: PlayerId,
    req: Request,
    _g: &dyn Ground,
    ev: &mut Vec<Event>,
) {
    let Request::TalkToSurvivor(isl) = req else {
        return;
    };
    if let Err(reason) = rescue(w, pid, isl, ev) {
        deny(ev, pid, reason);
    }
}

fn rescue(
    w: &mut World,
    pid: PlayerId,
    isl: IslandId,
    ev: &mut Vec<Event>,
) -> Result<(), &'static str> {
    let def = ISLANDS.get(isl as usize).ok_or("No such island")?;
    let st = w.islands.get(isl).ok_or("No such island")?;
    match st.survivor {
        SurvivorState::Waiting => {}
        SurvivorState::Rescued => return Err("Already rescued"),
        SurvivorState::Hidden if def.survivor => return Err("Build a Radio Station first"),
        SurvivorState::Hidden => return Err("No one is stranded here"),
    }
    if !w.near_spot(pid, isl, Spot::Survivor, INTERACT_RANGE) {
        return Err("Walk up to the survivor");
    }
    if let Some(st) = w.islands.get_mut(isl) {
        st.survivor = SurvivorState::Rescued;
    }
    w.economy.survivors.push(isl);
    ev.push(Event::SurvivorRescued(isl));
    let name = w.player(pid).map_or("", |p| p.name.as_str());
    toast(
        ev,
        None,
        format!("{name} rescued the survivor of {}", def.name),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{island, HOOK_ISLE, PALM_REEF};
    use crate::raft::Structure;
    use crate::world::IslandGround;

    const G: IslandGround = IslandGround;

    fn add_radio(w: &mut World) {
        let id = w.alloc_id();
        w.raft
            .structures
            .insert(id, Structure::new(PlaceableKind::RadioStation, (-1, -1), 0));
    }

    fn denied(w: &mut World, a: PlayerId, isl: IslandId) -> String {
        let before = w.clone();
        let ev = w.apply(a, Request::TalkToSurvivor(isl), &G);
        assert_eq!(*w, before);
        match &ev[..] {
            [Event::Denied { reason, .. }] => reason.clone(),
            _ => panic!("expected one Denied, got {ev:?}"),
        }
    }

    #[test]
    fn signals_appear_once_with_a_radio() {
        let mut w = World::new(1);
        let ev = w.step(0.1, &G);
        assert!(!ev.iter().any(|e| matches!(e, Event::SurvivorSignal(_))));
        add_radio(&mut w);
        let ev = w.step(0.1, &G);
        let signals: Vec<IslandId> = ev
            .iter()
            .filter_map(|e| match e {
                Event::SurvivorSignal(i) => Some(*i),
                _ => None,
            })
            .collect();
        assert_eq!(signals.len(), 6);
        assert!(signals.iter().all(|i| island(*i).survivor));
        let waiting = w
            .islands
            .list
            .iter()
            .filter(|s| s.survivor == SurvivorState::Waiting)
            .count();
        assert_eq!(waiting, 6);
        assert!(!w
            .step(0.1, &G)
            .iter()
            .any(|e| matches!(e, Event::SurvivorSignal(_))));
    }

    #[test]
    fn talking_rescues_waiting_survivors() {
        let mut w = World::new(1);
        let a = w.join("Ana").unwrap();
        let spot = island(PALM_REEF).spot(Spot::Survivor).with_y(1.0);
        w.player_mut(a).unwrap().pos = spot;
        assert_eq!(denied(&mut w, a, PALM_REEF), "Build a Radio Station first");
        assert_eq!(denied(&mut w, a, HOOK_ISLE), "No one is stranded here");
        assert_eq!(denied(&mut w, a, 255), "No such island");
        add_radio(&mut w);
        w.step(0.1, &G);
        w.player_mut(a).unwrap().pos = spot + crate::math::V3::xz(3.5, 0.0);
        assert_eq!(denied(&mut w, a, PALM_REEF), "Walk up to the survivor");
        w.player_mut(a).unwrap().pos = spot;
        let ev = w.apply(a, Request::TalkToSurvivor(PALM_REEF), &G);
        assert!(ev.contains(&Event::SurvivorRescued(PALM_REEF)), "{ev:?}");
        assert_eq!(
            w.islands.get(PALM_REEF).unwrap().survivor,
            SurvivorState::Rescued
        );
        assert_eq!(w.economy.survivors, vec![PALM_REEF]);
        assert_eq!(denied(&mut w, a, PALM_REEF), "Already rescued");
    }
}
