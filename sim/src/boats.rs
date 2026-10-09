//! Boats (spec 11). Seats live on the players (`Player::seat`); seat 0 drives.
//!
//! - BoardBoat: within BOARD_RANGE of the hull; the driver seat if free, else the first
//!   free passenger seat, else Denied. Clears fishing; the player snaps to `seat_pos`.
//! - LeaveBoat: onto the nearest raft tile within DISEMBARK_RAFT_RANGE, else ashore when
//!   the hull is within DISEMBARK_SHORE_RANGE of an island, else into the water beside
//!   the hull (PlayerTeleport). The driver leaving stops the boat.
//! - BoatTransform: driver only; finite, within MAX_COORD and MAX_MOVE of the hull;
//!   speed clamped to the boat's speed; pushed out of island radii (+ hull radius). A
//!   Motor Launch with no paid driving left and no stockpile oil is Denied ("Out of
//!   oil") while speed != 0.
//! - `step`: seated players follow their boat (pos/yaw at their seat, not in the water);
//!   boats without a connected driver stop. A moving Motor Launch buys `oil_interval`
//!   seconds of driving for 1 stockpile oil whenever its paid time runs out, else stops.

use crate::content::{
    island_near, BoatKind, BOARD_RANGE, BOAT_ROW_SPACING, BOAT_SEAT_Y, DISEMBARK_RAFT_RANGE,
    DISEMBARK_SHORE_RANGE, ISLANDS,
};
use crate::math::{facing, wrap_angle, V3};
use crate::msg::{deny, toast, Event, Request};
use crate::players::{Seat, MAX_COORD, MAX_MOVE};
use crate::raft::cell_center;
use crate::world::{EntityId, Ground, PlayerId, World};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Boat {
    pub kind: BoatKind,
    pub pos: V3,
    pub yaw: f32,
    /// Forward speed reported by the driver (m/s).
    pub speed: f32,
    /// Seconds of paid driving left (Motor Launch); 1 oil buys `oil_interval` more.
    pub fuel_timer: f32,
}

impl Boat {
    pub fn new(kind: BoatKind, pos: V3) -> Boat {
        Boat {
            kind,
            pos,
            yaw: 0.0,
            speed: 0.0,
            fuel_timer: 0.0,
        }
    }
}

impl World {
    /// Who sits where on a boat (index 0 = driver), derived from `Player::seat`.
    pub fn boat_seats(&self, boat: EntityId) -> Vec<Option<PlayerId>> {
        let n = self
            .dynamic
            .boats
            .get(&boat)
            .map_or(0, |b| b.kind.def().seats as usize);
        let mut seats = vec![None; n];
        for (id, p) in self.connected() {
            if let Some(s) = p.seat.filter(|s| s.boat == boat && (s.index as usize) < n) {
                seats[s.index as usize] = Some(id);
            }
        }
        seats
    }
}

/// Where seat `index` sits: rows of two along the hull, the driver's row at the stern.
pub fn seat_pos(boat: &Boat, index: u8) -> V3 {
    let rows = boat.kind.def().seats.div_ceil(2) as f32;
    let along = ((index / 2) as f32 - (rows - 1.0) * 0.5) * BOAT_ROW_SPACING;
    let side = if index.is_multiple_of(2) { -0.4 } else { 0.4 };
    let fwd = facing(boat.yaw);
    let right = V3::xz(-fwd.z, fwd.x);
    (boat.pos + fwd * along + right * side).with_y(BOAT_SEAT_Y)
}

/// Snaps every player seated on `id` to their seat.
fn seat_players(w: &mut World, id: EntityId) {
    let Some(b) = w.dynamic.boats.get(&id) else {
        return;
    };
    for p in w.players.values_mut() {
        if let Some(s) = p.seat.filter(|s| s.boat == id) {
            p.pos = seat_pos(b, s.index);
            p.yaw = b.yaw;
            p.in_water = false;
        }
    }
}

/// Where a player leaving `b` lands, and whether that is in the water.
fn disembark(w: &World, b: &Boat, g: &dyn Ground) -> (V3, bool) {
    let deck = w
        .raft
        .tiles
        .iter()
        .map(|&c| cell_center(c))
        .filter(|c| c.dist_xz(b.pos) <= DISEMBARK_RAFT_RANGE)
        .min_by(|a, c| a.dist_xz(b.pos).total_cmp(&c.dist_xz(b.pos)));
    if let Some(tile) = deck {
        return (tile, false);
    }
    let hull = b.kind.def().radius;
    if let Some(i) = island_near(b.pos.x, b.pos.z, hull + DISEMBARK_SHORE_RANGE) {
        let def = &ISLANDS[i as usize];
        let dir = (b.pos - def.center()).with_y(0.0).normalize();
        let at = def.center() + dir * (def.radius - 1.0).max(0.0);
        if let Some(h) = g.height_at(at.x, at.z) {
            return (at.with_y(h), false);
        }
    }
    let fwd = facing(b.yaw);
    (
        b.pos.with_y(0.0) + V3::xz(-fwd.z, fwd.x) * (hull + 0.5),
        true,
    )
}

pub(crate) fn step(w: &mut World, dt: f32, _g: &dyn Ground, ev: &mut Vec<Event>) {
    let boats = &w.dynamic.boats;
    for p in w.players.values_mut() {
        let Some(s) = p.seat else { continue };
        match boats.get(&s.boat) {
            Some(b) if s.index < b.kind.def().seats => {
                p.pos = seat_pos(b, s.index);
                p.yaw = b.yaw;
                p.in_water = false;
            }
            _ => p.seat = None,
        }
    }
    let drivers: BTreeMap<EntityId, PlayerId> = w
        .connected()
        .filter_map(|(id, p)| p.seat.filter(|s| s.index == 0).map(|s| (s.boat, id)))
        .collect();
    for (id, b) in w.dynamic.boats.iter_mut() {
        let Some(&driver) = drivers.get(id) else {
            b.speed = 0.0;
            continue;
        };
        let Some(interval) = b.kind.def().oil_interval else {
            continue;
        };
        if b.speed == 0.0 {
            continue;
        }
        if b.fuel_timer <= 0.0 {
            if w.economy.stock.oil == 0 {
                b.speed = 0.0;
                toast(ev, Some(driver), "Out of oil: refill the stockpile");
                continue;
            }
            w.economy.stock.oil -= 1;
            b.fuel_timer += interval;
        }
        b.fuel_timer -= dt;
    }
}

pub(crate) fn handle(
    w: &mut World,
    pid: PlayerId,
    req: Request,
    g: &dyn Ground,
    ev: &mut Vec<Event>,
) {
    let Some(p) = w.players.get(&pid) else {
        return;
    };
    match req {
        Request::BoardBoat(id) => {
            let Some(b) = w.dynamic.boats.get(&id) else {
                return deny(ev, pid, "There is no boat here");
            };
            if p.seat.is_some() {
                return deny(ev, pid, "You are already in a boat");
            }
            if p.pos.dist_xz(b.pos) > BOARD_RANGE {
                return deny(ev, pid, "Get closer to the boat");
            }
            let Some(index) = w.boat_seats(id).iter().position(Option::is_none) else {
                return deny(ev, pid, "The boat is full");
            };
            let p = w.players.get_mut(&pid).expect("checked above");
            p.seat = Some(Seat {
                boat: id,
                index: index as u8,
            });
            p.fishing = None;
            p.sprinting = false;
            seat_players(w, id);
        }
        Request::LeaveBoat => {
            let Some(seat) = p.seat else {
                return deny(ev, pid, "You are not in a boat");
            };
            let (pos, in_water) = match w.dynamic.boats.get(&seat.boat) {
                Some(b) => disembark(w, b, g),
                None => (p.pos, true),
            };
            let p = w.players.get_mut(&pid).expect("checked above");
            p.seat = None;
            p.pos = pos;
            p.in_water = in_water;
            if seat.index == 0 {
                if let Some(b) = w.dynamic.boats.get_mut(&seat.boat) {
                    b.speed = 0.0;
                }
            }
            ev.push(Event::PlayerTeleport { player: pid, pos });
        }
        Request::BoatTransform { pos, yaw, speed } => {
            let Some(seat) = p.seat.filter(|s| s.index == 0) else {
                return deny(ev, pid, "Only the driver steers");
            };
            let Some(b) = w.dynamic.boats.get(&seat.boat) else {
                return deny(ev, pid, "There is no boat here");
            };
            if !(pos.is_finite() && yaw.is_finite() && speed.is_finite())
                || pos.x.abs() > MAX_COORD
                || pos.y.abs() > MAX_COORD
                || pos.z.abs() > MAX_COORD
            {
                return deny(ev, pid, "Bad transform");
            }
            if pos.dist(b.pos) > MAX_MOVE {
                return deny(ev, pid, "Moved too far");
            }
            let def = b.kind.def();
            let speed = speed.clamp(-def.speed, def.speed);
            if def.oil_interval.is_some()
                && speed != 0.0
                && b.fuel_timer <= 0.0
                && w.economy.stock.oil == 0
            {
                return deny(ev, pid, "Out of oil");
            }
            let mut pos = pos;
            for isl in &ISLANDS {
                let min = isl.radius + def.radius;
                let away = (pos - isl.center()).with_y(0.0);
                if away.length_xz() < min {
                    let dir = away.normalize();
                    let dir = if dir == V3::ZERO {
                        V3::xz(1.0, 0.0)
                    } else {
                        dir
                    };
                    pos = (isl.center() + dir * min).with_y(pos.y);
                }
            }
            let b = w.dynamic.boats.get_mut(&seat.boat).expect("checked above");
            b.pos = pos;
            b.yaw = wrap_angle(yaw);
            b.speed = speed;
            seat_players(w, seat.boat);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{island, HOOK_ISLE};
    use crate::world::{FlatGround, IslandGround};

    const G: FlatGround = FlatGround::WATER;

    fn denied(ev: &[Event]) -> bool {
        matches!(ev, [Event::Denied { .. }])
    }

    /// A boat 30 m east of the raft (open water) with Ana swimming beside it.
    fn setup(kind: BoatKind) -> (World, PlayerId, EntityId) {
        let mut w = World::new(1);
        let a = w.join("Ana").unwrap();
        let b = w.spawn_boat(kind, V3::xz(30.0, 0.0));
        w.player_mut(a).unwrap().pos = V3::xz(32.0, 0.0);
        (w, a, b)
    }

    fn drive(pos: V3, speed: f32) -> Request {
        Request::BoatTransform {
            pos,
            yaw: 0.5,
            speed,
        }
    }

    #[test]
    fn seats_derive_from_players() {
        let mut w = World::new(1);
        let a = w.join("Ana").unwrap();
        let b = w.spawn_boat(BoatKind::Dinghy, V3::ZERO);
        w.player_mut(a).unwrap().seat = Some(Seat { boat: b, index: 1 });
        assert_eq!(w.boat_seats(b), vec![None, Some(a)]);
        w.leave(a);
        assert_eq!(w.boat_seats(b), vec![None, None]);
    }

    #[test]
    fn seat_layout() {
        let mut boat = Boat::new(BoatKind::MotorLaunch, V3::xz(10.0, 0.0));
        let seats: Vec<V3> = (0..8).map(|i| seat_pos(&boat, i)).collect();
        // Yaw 0 faces -Z: the driver's row is at the stern (+Z), passengers ahead.
        assert!(seats[0].z > seats[7].z && seats[0].x < seats[1].x);
        assert!((seats[0].z - seats[2].z - BOAT_ROW_SPACING).abs() < 1e-4);
        assert!(seats.iter().all(|s| s.y == BOAT_SEAT_Y));
        boat.yaw = 1.0;
        assert!(
            (seat_pos(&boat, 3).dist_xz(boat.pos) - seats[3].dist_xz(V3::xz(10.0, 0.0))).abs()
                < 1e-4
        );
    }

    #[test]
    fn board_drive_and_leave_in_open_water() {
        let (mut w, a, b) = setup(BoatKind::Dinghy);
        let c = w.join("Cy").unwrap();
        let d = w.join("Di").unwrap();
        w.player_mut(c).unwrap().pos = V3::xz(29.0, 1.0);
        w.player_mut(d).unwrap().pos = V3::xz(30.0, -1.0);
        assert!(denied(&w.apply(a, Request::BoardBoat(999), &G)));
        assert!(denied(&w.apply(a, Request::LeaveBoat, &G)));
        w.player_mut(a).unwrap().fishing = None;
        assert!(w.apply(a, Request::BoardBoat(b), &G).is_empty());
        assert!(
            denied(&w.apply(a, Request::BoardBoat(b), &G)),
            "already aboard"
        );
        w.apply(c, Request::BoardBoat(b), &G);
        assert_eq!(w.boat_seats(b), vec![Some(a), Some(c)]);
        assert!(denied(&w.apply(d, Request::BoardBoat(b), &G)), "full");
        let pa = w.player(a).unwrap();
        assert_eq!(pa.pos, seat_pos(&w.dynamic.boats[&b], 0));
        assert!(!pa.in_water);

        // Only the driver steers; speed is clamped; garbage changes nothing.
        assert!(denied(&w.apply(c, drive(V3::xz(40.0, 0.0), 1.0), &G)));
        let to = V3::xz(50.0, 10.0);
        assert!(w.apply(a, drive(to, 100.0), &G).is_empty());
        let boat = w.dynamic.boats[&b].clone();
        assert_eq!((boat.pos, boat.speed, boat.yaw), (to, 9.0, 0.5));
        assert_eq!(w.player(c).unwrap().pos, seat_pos(&boat, 1));
        let before = w.clone();
        assert!(denied(&w.apply(a, drive(V3::xz(f32::NAN, 0.0), 1.0), &G)));
        assert!(denied(&w.apply(a, drive(V3::xz(1e9, 0.0), 1.0), &G)));
        assert!(denied(&w.apply(a, drive(to + V3::xz(51.0, 0.0), 1.0), &G)));
        assert!(denied(&w.apply(
            a,
            Request::BoatTransform {
                pos: to,
                yaw: f32::INFINITY,
                speed: 1.0
            },
            &G
        )));
        assert_eq!(w, before);

        // The passenger drops into the water beside the hull; the boat keeps going.
        let ev = w.apply(c, Request::LeaveBoat, &G);
        let pc = w.player(c).unwrap();
        assert!(pc.seat.is_none() && pc.in_water);
        assert_eq!(
            ev,
            vec![Event::PlayerTeleport {
                player: c,
                pos: pc.pos
            }]
        );
        assert!((pc.pos.dist_xz(to) - 2.0).abs() < 1e-3);
        assert_eq!(w.dynamic.boats[&b].speed, 9.0);
        // The driver leaving stops it.
        w.apply(a, Request::LeaveBoat, &G);
        assert_eq!(w.dynamic.boats[&b].speed, 0.0);
    }

    #[test]
    fn leaving_near_the_raft_or_a_shore() {
        let (mut w, a, b) = setup(BoatKind::Skiff);
        w.apply(a, Request::BoardBoat(b), &G);
        // Next to the east edge of the start raft (tiles span x -4..4).
        w.apply(a, drive(V3::xz(7.0, 1.0), 0.0), &G);
        w.apply(a, Request::LeaveBoat, &G);
        let p = w.player(a).unwrap();
        assert!(w.raft.on_deck(p.pos) && !p.in_water, "{:?}", p.pos);
        assert_eq!(p.pos, cell_center((1, 0)));

        // Near Hook Isle: ashore on the near side.
        let hook = island(HOOK_ISLE);
        let off = hook.center() + V3::xz(hook.radius + 3.0, 0.0);
        w.dynamic.boats.get_mut(&b).unwrap().pos = off;
        w.player_mut(a).unwrap().pos = off;
        w.apply(a, Request::BoardBoat(b), &G);
        w.apply(a, Request::LeaveBoat, &IslandGround);
        let p = w.player(a).unwrap();
        assert!(!p.in_water && p.pos.y == 1.0);
        assert!((p.pos.dist_xz(hook.center()) - (hook.radius - 1.0)).abs() < 1e-2);
    }

    #[test]
    fn boats_are_pushed_out_of_islands() {
        let (mut w, a, b) = setup(BoatKind::Dinghy);
        w.apply(a, Request::BoardBoat(b), &G);
        let hook = island(HOOK_ISLE);
        w.apply(a, drive(hook.center() + V3::xz(0.0, 5.0), 3.0), &G);
        let pos = w.dynamic.boats[&b].pos;
        assert!((pos.dist_xz(hook.center()) - hook.radius - 1.5).abs() < 1e-2);
        assert!(pos.z > hook.center().z);
        w.apply(a, drive(hook.center(), 3.0), &G);
        assert!(w.dynamic.boats[&b].pos.dist_xz(hook.center()) > hook.radius);
    }

    #[test]
    fn passengers_follow_and_stop_without_driver() {
        let (mut w, a, b) = setup(BoatKind::Skiff);
        w.apply(a, Request::BoardBoat(b), &G);
        w.apply(a, drive(V3::xz(60.0, 0.0), 5.0), &G);
        // The host moves the boat (e.g. mooring): seated players follow on the next step.
        w.dynamic.boats.get_mut(&b).unwrap().pos = V3::xz(70.0, 0.0);
        w.step(0.1, &G);
        assert_eq!(w.player(a).unwrap().pos, seat_pos(&w.dynamic.boats[&b], 0));
        assert_eq!(w.dynamic.boats[&b].speed, 5.0);
        w.leave(a);
        w.step(0.1, &G);
        assert_eq!(w.dynamic.boats[&b].speed, 0.0);
        // A vanished boat frees its seats.
        let a = w.join("Ana").unwrap();
        w.player_mut(a).unwrap().pos = V3::xz(70.0, 0.0);
        w.apply(a, Request::BoardBoat(b), &G);
        w.dynamic.boats.remove(&b);
        w.step(0.1, &G);
        assert!(w.player(a).unwrap().seat.is_none());
    }

    #[test]
    fn motor_launch_burns_one_oil_per_minute_of_driving() {
        let (mut w, a, b) = setup(BoatKind::MotorLaunch);
        w.economy.stock.oil = 0;
        // The run crosses into night: a flare keeps the Gloomfin from downing Ana.
        w.player_mut(a).unwrap().flare_until = 1e9;
        w.apply(a, Request::BoardBoat(b), &G);
        let before = w.clone();
        let ev = w.apply(a, drive(V3::xz(31.0, 0.0), 10.0), &G);
        assert!(matches!(&ev[..], [Event::Denied { reason, .. }] if reason == "Out of oil"));
        assert_eq!(w, before);
        assert!(
            w.apply(a, drive(V3::xz(30.0, 0.0), 0.0), &G).is_empty(),
            "idle is free"
        );

        w.economy.stock.oil = 2;
        w.apply(a, drive(V3::xz(31.0, 0.0), 10.0), &G);
        w.step(0.25, &G);
        assert_eq!(w.economy.stock.oil, 1);
        // Idling burns nothing.
        w.apply(a, drive(V3::xz(31.0, 0.0), 0.0), &G);
        for _ in 0..800 {
            w.step(0.25, &G);
        }
        assert_eq!(w.economy.stock.oil, 1);
        w.apply(a, drive(V3::xz(31.0, 0.0), 10.0), &G);
        for _ in 0..239 {
            w.step(0.25, &G);
        }
        assert_eq!(w.economy.stock.oil, 1, "60 s per oil");
        w.step(0.25, &G);
        assert_eq!(w.economy.stock.oil, 0);
        for _ in 0..239 {
            w.step(0.25, &G);
        }
        assert_eq!(w.dynamic.boats[&b].speed, 10.0);
        let ev = w.step(0.25, &G);
        assert_eq!(w.dynamic.boats[&b].speed, 0.0);
        assert!(ev
            .iter()
            .any(|e| matches!(e, Event::Toast { player: Some(p), .. } if *p == a)));
        assert!(denied(&w.apply(a, drive(V3::xz(32.0, 0.0), 10.0), &G)));
        // The rowing boats need no oil.
        let (mut w, a, b) = setup(BoatKind::Dinghy);
        w.economy.stock.oil = 0;
        w.apply(a, Request::BoardBoat(b), &G);
        assert!(w.apply(a, drive(V3::xz(31.0, 0.0), 5.0), &G).is_empty());
    }
}
