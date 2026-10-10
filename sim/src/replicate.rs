//! Replication helpers (spec 3.2). Slow sections travel as JSON when their serialized
//! form changes; the fast `DynamicSnapshot` carries quantized transforms at 15 Hz.
//! Clients keep a mirror `World` and feed both into it.

use crate::boats::Boat;
use crate::content::{BoatKind, EnemyKind, ItemKind, ISLANDS, QUESTS};
use crate::debris::Debris;
use crate::enemies::{Enemy, EnemyHome, EnemyState};
use crate::islands::{Islands, Quests};
use crate::math::{wrap_angle, V3};
use crate::players::Player;
use crate::world::{EntityId, PlayerId, World};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum SectionId {
    Clock,
    Raft,
    Economy,
    Islands,
    Quests,
    Trader,
    /// Names and connection state of every player (the full entry goes only to its owner).
    Roster,
}

impl SectionId {
    pub const ALL: [SectionId; 7] = [
        SectionId::Clock,
        SectionId::Raft,
        SectionId::Economy,
        SectionId::Islands,
        SectionId::Quests,
        SectionId::Trader,
        SectionId::Roster,
    ];
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RosterEntry {
    pub name: String,
    pub connected: bool,
}

/// Fast section: other players' and dynamic objects' transforms.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DynamicSnapshot {
    pub players: Vec<PlayerPose>,
    pub debris: Vec<DebrisPose>,
    pub enemies: Vec<EnemyPose>,
    pub boats: Vec<BoatPose>,
}

/// Positions in centimetres, yaw in 1/32767 of a half turn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlayerPose {
    pub id: PlayerId,
    pub pos: [i32; 3],
    pub yaw: i16,
    pub health: u8,
    pub downed: bool,
    pub in_water: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DebrisPose {
    pub id: EntityId,
    pub kind: ItemKind,
    pub pos: [i32; 3],
    /// `Debris::resting`: the view draws it lying still instead of bobbing.
    #[serde(default)]
    pub resting: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EnemyPose {
    pub id: EntityId,
    pub kind: EnemyKind,
    pub pos: [i32; 3],
    pub yaw: i16,
    pub hp: u16,
    pub max_hp: u16,
    pub state: EnemyState,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoatPose {
    pub id: EntityId,
    pub kind: BoatKind,
    pub pos: [i32; 3],
    pub yaw: i16,
}

pub fn qpos(v: V3) -> [i32; 3] {
    let q = |f: f32| (f * 100.0).round() as i32;
    [q(v.x), q(v.y), q(v.z)]
}

pub fn dpos(p: [i32; 3]) -> V3 {
    V3::new(
        p[0] as f32 / 100.0,
        p[1] as f32 / 100.0,
        p[2] as f32 / 100.0,
    )
}

pub fn qyaw(yaw: f32) -> i16 {
    (wrap_angle(yaw) / std::f32::consts::PI * 32767.0).round() as i16
}

pub fn dyaw(q: i16) -> f32 {
    q as f32 / 32767.0 * std::f32::consts::PI
}

fn to_value<T: Serialize>(t: &T) -> Value {
    serde_json::to_value(t).expect("world types always serialize")
}

fn from_value<T: for<'de> Deserialize<'de>>(v: Value) -> Result<T, String> {
    serde_json::from_value(v).map_err(|e| e.to_string())
}

/// The systems index these tables by id, so they must match the content.
fn check_islands(i: &Islands) -> Result<(), String> {
    let shrines_ok = i
        .list
        .iter()
        .zip(&ISLANDS)
        .all(|(s, d)| s.shrines.len() == d.shrines as usize);
    if i.list.len() != ISLANDS.len() || !shrines_ok {
        return Err("The data does not match this game's islands".into());
    }
    Ok(())
}

fn check_quests(q: &Quests) -> Result<(), String> {
    if q.entries.len() != QUESTS.len() {
        return Err("The data does not match this game's quests".into());
    }
    Ok(())
}

impl World {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("world types always serialize")
    }

    /// Parses a world and refuses one whose island or quest tables do not match the
    /// content (the systems index them by id).
    pub fn from_json(json: &str) -> Result<World, String> {
        let w: World = serde_json::from_str(json).map_err(|e| e.to_string())?;
        check_islands(&w.islands)?;
        check_quests(&w.quests)?;
        Ok(w)
    }

    pub fn section(&self, id: SectionId) -> Value {
        match id {
            SectionId::Clock => to_value(&self.clock),
            SectionId::Raft => to_value(&self.raft),
            SectionId::Economy => to_value(&self.economy),
            SectionId::Islands => to_value(&self.islands),
            SectionId::Quests => to_value(&self.quests),
            SectionId::Trader => to_value(&self.trader),
            SectionId::Roster => to_value(&self.roster()),
        }
    }

    /// Client side: replaces a section with what the host sent.
    pub fn apply_section(&mut self, id: SectionId, v: Value) -> Result<(), String> {
        match id {
            SectionId::Clock => self.clock = from_value(v)?,
            SectionId::Raft => self.raft = from_value(v)?,
            SectionId::Economy => self.economy = from_value(v)?,
            SectionId::Islands => {
                let islands = from_value(v)?;
                check_islands(&islands)?;
                self.islands = islands;
            }
            SectionId::Quests => {
                let quests = from_value(v)?;
                check_quests(&quests)?;
                self.quests = quests;
            }
            SectionId::Trader => self.trader = from_value(v)?,
            SectionId::Roster => {
                let roster: BTreeMap<PlayerId, RosterEntry> = from_value(v)?;
                self.players.retain(|id, _| roster.contains_key(id));
                for (id, r) in roster {
                    let p = self
                        .players
                        .entry(id)
                        .or_insert_with(|| Player::new(&r.name, V3::ZERO));
                    p.name = r.name;
                    p.connected = r.connected;
                }
            }
        }
        Ok(())
    }

    pub fn roster(&self) -> BTreeMap<PlayerId, RosterEntry> {
        self.players
            .iter()
            .map(|(id, p)| {
                (
                    *id,
                    RosterEntry {
                        name: p.name.clone(),
                        connected: p.connected,
                    },
                )
            })
            .collect()
    }

    /// The full entry sent only to its owner.
    pub fn player_entry(&self, id: PlayerId) -> Option<Value> {
        self.players.get(&id).map(to_value)
    }

    /// Client side: replaces the local player's entry.
    pub fn apply_player_entry(&mut self, id: PlayerId, v: Value) -> Result<(), String> {
        let p: Player = from_value(v)?;
        self.players.insert(id, p);
        Ok(())
    }

    pub fn snapshot(&self) -> DynamicSnapshot {
        let d = &self.dynamic;
        DynamicSnapshot {
            players: self
                .connected()
                .map(|(id, p)| PlayerPose {
                    id,
                    pos: qpos(p.pos),
                    yaw: qyaw(p.yaw),
                    health: p.health.clamp(0.0, 255.0).round() as u8,
                    downed: p.downed,
                    in_water: p.in_water,
                })
                .collect(),
            debris: d
                .debris
                .iter()
                .map(|(id, x)| DebrisPose {
                    id: *id,
                    kind: x.kind,
                    pos: qpos(x.pos),
                    resting: x.resting,
                })
                .collect(),
            enemies: d
                .enemies
                .iter()
                .map(|(id, e)| EnemyPose {
                    id: *id,
                    kind: e.kind,
                    pos: qpos(e.pos),
                    yaw: qyaw(e.yaw),
                    hp: e.hp.clamp(0.0, 65535.0).ceil() as u16,
                    max_hp: e.max_hp.clamp(0.0, 65535.0).ceil() as u16,
                    state: e.state,
                })
                .collect(),
            boats: d
                .boats
                .iter()
                .map(|(id, b)| BoatPose {
                    id: *id,
                    kind: b.kind,
                    pos: qpos(b.pos),
                    yaw: qyaw(b.yaw),
                })
                .collect(),
        }
    }

    /// Client side: moves the mirror to the snapshot. Dynamic objects missing from it are
    /// removed; the local player's own pose is skipped (the client owns its movement).
    pub fn apply_snapshot(&mut self, s: &DynamicSnapshot, local: Option<PlayerId>) {
        for pose in s.players.iter().filter(|p| Some(p.id) != local) {
            let p = self
                .players
                .entry(pose.id)
                .or_insert_with(|| Player::new("", V3::ZERO));
            p.pos = dpos(pose.pos);
            p.yaw = dyaw(pose.yaw);
            p.health = pose.health as f32;
            p.downed = pose.downed;
            p.in_water = pose.in_water;
        }
        let now = self.clock.elapsed;
        let d = &mut self.dynamic;
        sync(
            &mut d.debris,
            &s.debris,
            |x| x.id,
            |x| Debris {
                resting: x.resting,
                ..Debris::new(x.kind, dpos(x.pos), now)
            },
            |o, x| {
                o.kind = x.kind;
                o.pos = dpos(x.pos);
                o.resting = x.resting;
            },
        );
        sync(
            &mut d.enemies,
            &s.enemies,
            |x| x.id,
            |x| Enemy::new(x.kind, dpos(x.pos), EnemyHome::Night),
            |o, x| {
                o.kind = x.kind;
                o.pos = dpos(x.pos);
                o.yaw = dyaw(x.yaw);
                o.hp = x.hp as f32;
                o.max_hp = x.max_hp as f32;
                o.state = x.state;
            },
        );
        sync(
            &mut d.boats,
            &s.boats,
            |x| x.id,
            |x| Boat::new(x.kind, dpos(x.pos)),
            |o, x| {
                o.kind = x.kind;
                o.pos = dpos(x.pos);
                o.yaw = dyaw(x.yaw);
            },
        );
    }
}

fn sync<T, P>(
    map: &mut BTreeMap<EntityId, T>,
    poses: &[P],
    id: impl Fn(&P) -> EntityId,
    make: impl Fn(&P) -> T,
    update: impl Fn(&mut T, &P),
) {
    let mut seen = std::collections::BTreeSet::new();
    for p in poses {
        let k = id(p);
        seen.insert(k);
        update(map.entry(k).or_insert_with(|| make(p)), p);
    }
    map.retain(|k, _| seen.contains(k));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::EnemyKind;
    use crate::world::FlatGround;

    #[test]
    fn quantization_round_trips() {
        let v = V3::new(-1234.567, 0.45, 89.001);
        assert!(dpos(qpos(v)).dist(v) < 0.01);
        for yaw in [-3.1, -1.0, 0.0, 2.5, 3.1] {
            assert!((dyaw(qyaw(yaw)) - yaw).abs() < 1e-3, "{yaw}");
        }
        assert!((dyaw(qyaw(7.0)) - wrap_angle(7.0)).abs() < 1e-3);
    }

    #[test]
    fn sections_and_snapshot_mirror_the_host() {
        let mut host = World::new(5);
        let a = host.join("Ana").unwrap();
        let b = host.join("Bo").unwrap();
        host.step(1.0 / 60.0, &FlatGround::WATER);
        let mut ev = Vec::new();
        host.spawn_enemy(
            EnemyKind::Crawler,
            V3::new(1.0, 0.45, 2.0),
            EnemyHome::Raft,
            &mut ev,
        );
        host.spawn_debris(ItemKind::Plank, V3::new(-30.0, 0.0, 4.0));
        host.spawn_boat(BoatKind::Skiff, host.raft.dock_pos());

        let mut client = World::new(0);
        client.players.clear();
        for id in SectionId::ALL {
            let wire = serde_json::to_string(&host.section(id)).unwrap();
            client
                .apply_section(id, serde_json::from_str(&wire).unwrap())
                .unwrap();
        }
        client
            .apply_player_entry(b, host.player_entry(b).unwrap())
            .unwrap();
        let wire = serde_json::to_string(&host.snapshot()).unwrap();
        let snap: DynamicSnapshot = serde_json::from_str(&wire).unwrap();
        client.apply_snapshot(&snap, Some(b));

        assert_eq!(client.clock, host.clock);
        assert_eq!(client.raft, host.raft);
        assert_eq!(client.player(b), host.player(b));
        assert_eq!(client.player(a).unwrap().name, "Ana");
        assert!(
            client
                .player(a)
                .unwrap()
                .pos
                .dist(host.player(a).unwrap().pos)
                < 0.01
        );
        assert_eq!(client.dynamic.enemies.len(), 1);
        assert_eq!(client.dynamic.debris.len(), 1);
        assert_eq!(client.dynamic.boats.len(), 1);

        host.dynamic.debris.clear();
        client.apply_snapshot(&host.snapshot(), Some(b));
        assert!(client.dynamic.debris.is_empty());
        assert!(client.apply_section(SectionId::Clock, Value::Null).is_err());
        // Tables indexed by id must match the content, or the mirror would panic later.
        let mut bad = host.islands.clone();
        bad.list.pop();
        let v = serde_json::to_value(&bad).unwrap();
        assert!(client.apply_section(SectionId::Islands, v).is_err());
        assert_eq!(client.islands.list.len(), ISLANDS.len());
        let v = serde_json::json!({ "entries": [] });
        assert!(client.apply_section(SectionId::Quests, v).is_err());
    }
}
