//! Everything that moves or changes outside the raft deck: debris, enemies, boats, the
//! trader, other players (with name tags, downed markers and flares), survivors and the
//! island props (chests, shrines, generator, beacon). Positions arrive per tick or per
//! snapshot; each object eases toward its target, and floating ones ride `wave` at the
//! view's smoothed time.
//!
//! Copies lose their animation (docs/scene-names.md), so animated things are scene
//! objects used in place: the one-instance templates (`tpl.enemy.rig_brute`,
//! `tpl.enemy.leviathan`, `tpl.trader_boat`), the remote-player pool `pool.avatar.N` and
//! each island's `<island>.survivor`. Released, they are hidden and put back.

use super::ocean::{hash, unit};
use super::{variant, Block};
use crate::player::swim;
use crate::shared::Frame;
use raft_sim::content::{
    BoatKind, EnemyKind, ItemKind, GENERATOR_METAL, ISLANDS, LIGHTHOUSE_ATOLL, RUSTED_WRECK,
    TILE,
};
use raft_sim::debris::wave;
use raft_sim::enemies::{Enemy, EnemyState};
use raft_sim::math::{wrap_angle, yaw_toward};
use raft_sim::raft::cell_center;
use raft_sim::survivors::SurvivorState;
use raft_sim::{EntityId, Event, Target as Hit, World, V3};
use rusting_engine::prelude::{
    AlphaMode, CubeSpawn, Entity, GameScene, MaterialAsset, MaterialModel, Sound, SoundId,
    Transform,
};
use rusting_engine::runtime::{Animation, TweenRepeat, Visibility};
use rusting_engine::text_texture::TextStyle;
use rusting_engine::Handle;
use std::collections::{BTreeSet, HashMap};
use std::f32::consts::{FRAC_PI_2, PI};

/// Templates with animation that exist once: used in place by one mover at a time
/// (a second one gets a still copy).
const ONE_INSTANCE: [&str; 3] = ["tpl.enemy.rig_brute", "tpl.enemy.leviathan", "tpl.trader_boat"];
const AVATAR_POOL: usize = 7;

#[derive(Default)]
pub struct Movers {
    pool: HashMap<String, Mover>,
    /// Object name -> exists, so a missing one is looked up once.
    known: HashMap<String, bool>,
    /// Island prop visibility: (now, the scene's own) per object name.
    shown: HashMap<String, (bool, bool)>,
    /// `hook_isle`, `palm_reef`, ...: island name prefixes in `ISLANDS` order.
    islands: Vec<String>,
    trader: Option<Trip>,
    tag_material: Option<Handle<MaterialAsset>>,
    /// Engine loops on driven boats, with the (rate, volume) last set.
    engines: HashMap<EntityId, (SoundId, (f32, f32))>,
}

struct Mover {
    entity: Entity,
    /// The object copied or used in place, or "" for a fallback block.
    source: String,
    /// Used in place: its transform to put back on release.
    home: Option<Transform>,
    rot: [f32; 3],
    lift: f32,
    pos: V3,
    yaw: f32,
    /// Smoothed ground speed (m/s), for walk/idle clips.
    speed: f32,
    /// Remote players: how far into the swim and tread poses.
    swim: swim::Pose,
    seen: bool,
    /// Last text, glow or clip pushed, so they are set on change only.
    state: String,
}

/// Where a model should be.
#[derive(Clone, Copy, Default)]
struct Target {
    pos: V3,
    yaw: f32,
    /// Rides the waves with its origin this far above the surface.
    float: Option<f32>,
    /// Lies on its back (downed players without a die clip).
    lying: bool,
    /// In the water: lies forward to stroke when moving, else stands up to tread.
    swim: bool,
    /// Places at once instead of easing.
    snap: bool,
    /// Motion for models that do not animate (copies and blocks).
    pose: Pose,
}

/// Offsets on top of the eased transform: metres up and forward, radians.
#[derive(Clone, Copy, Default)]
struct Pose {
    up: f32,
    fwd: f32,
    pitch: f32,
    yaw: f32,
    roll: f32,
}

struct Trip {
    pos: V3,
    yaw: f32,
    dock: V3,
    leaving: bool,
}

const AVATAR: Block = Block { size: [0.6, 1.8, 0.45], color: [0.8, 0.4, 0.2], lift: 0.9 };
const SURVIVOR: Block = Block { size: [0.5, 1.7, 0.4], color: [0.85, 0.75, 0.45], lift: 0.85 };
const PLAYER_COLORS: [[f32; 3]; 8] = [
    [0.85, 0.35, 0.2],
    [0.2, 0.55, 0.85],
    [0.3, 0.75, 0.3],
    [0.85, 0.75, 0.2],
    [0.7, 0.3, 0.75],
    [0.2, 0.75, 0.7],
    [0.9, 0.5, 0.6],
    [0.6, 0.6, 0.6],
];

impl Movers {
    pub fn update(&mut self, scene: &mut GameScene, frame: &Frame, t: f64, dt: f32, cam: [f32; 3]) {
        let w = frame.world;
        if self.islands.is_empty() {
            self.islands = ISLANDS.iter().map(|d| d.name.to_lowercase().replace(' ', "_")).collect();
        }
        for m in self.pool.values_mut() {
            m.seen = false;
        }

        for (id, d) in &w.dynamic.debris {
            let tpls = [format!("tpl.{}", variant(&format!("{:?}", d.kind)))];
            let yaw = unit(hash(*id as i32, 17)) * 3.1;
            let target = Target { pos: d.pos, yaw, float: Some(0.0), ..Target::default() };
            self.model(scene, &format!("debris.{id}"), &tpls, Some(debris_block(d.kind)), target, t, dt);
        }
        for (id, e) in &w.dynamic.enemies {
            let key = format!("enemy.{id}");
            let tpls = [format!("tpl.enemy.{}", variant(&format!("{:?}", e.kind)))];
            // Swimmers sit at y 0 in the sim with their origin at the body centre.
            let float = match e.kind {
                EnemyKind::Leviathan => Some(-2.2),
                EnemyKind::Gloomfin => Some(-0.15),
                _ => None,
            };
            let pose = enemy_pose(*id as i32, e, t);
            let target = Target { pos: e.pos, yaw: e.yaw, float, pose, ..Target::default() };
            self.model(scene, &key, &tpls, Some(enemy_block(e.kind)), target, t, dt);
            let clip = match (e.kind, e.state) {
                (EnemyKind::Leviathan, _) => "swim",
                (EnemyKind::RigBrute, EnemyState::Chase) => "run",
                (EnemyKind::RigBrute, EnemyState::Attack) => "attack",
                (EnemyKind::RigBrute, EnemyState::Flee) => "walk",
                _ => "idle",
            };
            self.clip(scene, &key, clip);
        }
        for (id, b) in &w.dynamic.boats {
            let tpls = [format!("tpl.boat.{}", variant(&format!("{:?}", b.kind)))];
            let hull = match b.kind {
                BoatKind::Dinghy => [1.4, 0.5, 2.6],
                BoatKind::Skiff => [1.8, 0.6, 3.6],
                BoatKind::MotorLaunch => [2.2, 0.8, 4.8],
            };
            let block = Block { size: hull, color: [0.55, 0.4, 0.25], lift: 0.05 };
            let target = Target { pos: b.pos, yaw: b.yaw, float: Some(0.0), ..Target::default() };
            self.model(scene, &format!("boat.{id}"), &tpls, Some(block), target, t, dt);
        }
        self.engines(scene, w);
        self.trader(scene, w, t, dt);
        self.players(scene, frame, t, dt, cam);
        self.survivors(scene, w, t, dt);
        self.islands(scene, frame);

        for e in frame.events {
            if let Event::Damage { target, .. } = e {
                let key = match target {
                    Hit::Enemy(id) => format!("enemy.{id}"),
                    Hit::Player(pid) => format!("avatar.{pid}"),
                };
                if let Some(name) = self.pool.get(&key).map(|m| object(&key, m).to_string()) {
                    scene.flash(&name);
                }
            }
        }

        let gone: Vec<String> = self.pool.iter().filter(|(_, m)| !m.seen).map(|(k, _)| k.clone()).collect();
        for key in gone {
            if let Some(m) = self.pool.remove(&key) {
                release(scene, &key, m);
            }
        }
    }

    pub fn clear(&mut self, scene: &mut GameScene) {
        for (sound, _) in self.engines.values() {
            scene.stop_sound(*sound);
        }
        for (key, m) in std::mem::take(&mut self.pool) {
            release(scene, &key, m);
        }
        for (name, (now, default)) in &self.shown {
            if now != default {
                scene.set_visible(name, *default);
            }
        }
        *self = Movers::default();
    }

    /// An engine loop on each boat with a driver, on the hull so it pans and fades with
    /// distance. Pitch and volume follow speed: the Dinghy putters high and quiet, the
    /// Motor Launch drones low and loud.
    fn engines(&mut self, scene: &mut GameScene, w: &World) {
        let mut want = HashMap::new();
        for (&id, b) in &w.dynamic.boats {
            if w.boat_seats(id).first().copied().flatten().is_none() {
                continue;
            }
            let f = (b.speed.abs() / b.kind.def().speed.max(0.1)).clamp(0.0, 1.0);
            let tone = match b.kind {
                BoatKind::Dinghy => (1.25 + 0.45 * f, 0.25 + 0.25 * f),
                BoatKind::Skiff => (1.0 + 0.4 * f, 0.35 + 0.3 * f),
                BoatKind::MotorLaunch => (0.7 + 0.35 * f, 0.5 + 0.35 * f),
            };
            want.insert(id, tone);
        }
        self.engines.retain(|id, (sound, _)| {
            let keep = want.contains_key(id);
            if !keep {
                scene.stop_sound(*sound);
            }
            keep
        });
        for (id, (rate, volume)) in want {
            match self.engines.get_mut(&id) {
                Some((sound, last)) => {
                    if (last.0 - rate).abs() > 0.01 || (last.1 - volume).abs() > 0.01 {
                        scene.set_sound_rate(*sound, rate, 0.2);
                        scene.set_sound_volume(*sound, volume, 0.2);
                        *last = (rate, volume);
                    }
                }
                None => {
                    let sound = Sound { volume, rate, looped: true, ..Sound::default() };
                    if let Some(s) = scene.play_sound_on(&format!("boat.{id}"), "sfx/boat_engine_loop.ogg", sound) {
                        self.engines.insert(id, (s, (rate, volume)));
                    }
                }
            }
        }
    }

    /// The trader's boat glides in from the open sea on trader days, waits at the
    /// dock, and leaves the other way.
    fn trader(&mut self, scene: &mut GameScene, w: &World, t: f64, dt: f32) {
        match (w.trader.present, &mut self.trader) {
            (true, Some(trip)) => {
                trip.dock = w.trader.dock;
                trip.leaving = false;
            }
            (true, None) => {
                let pos = w.trader.dock + V3::new(140.0, 0.0, 60.0);
                self.trader = Some(Trip { pos, yaw: 0.0, dock: w.trader.dock, leaving: false });
            }
            (false, Some(trip)) => trip.leaving = true,
            (false, None) => {}
        }
        let Some(trip) = &mut self.trader else {
            return;
        };
        let goal = if trip.leaving {
            trip.dock + V3::new(140.0, 0.0, -60.0)
        } else {
            trip.dock
        };
        let to = goal - trip.pos;
        let dist = to.length_xz();
        if trip.leaving && dist < 1.0 {
            self.trader = None;
            return;
        }
        let speed = (dist * 0.5).clamp(0.5, 8.0);
        trip.pos = trip.pos.move_toward(goal.with_y(trip.pos.y), speed * dt);
        let heading = if dist > 1.0 { yaw_toward(to.x, to.z) } else { 0.0 };
        trip.yaw = ease_angle(trip.yaw, heading, 1.0 - (-2.0 * dt).exp());
        let docked = !trip.leaving && dist < 1.0;
        let target = Target { pos: trip.pos, yaw: trip.yaw, float: Some(0.0), snap: true, ..Target::default() };
        let block = Block { size: [3.0, 2.0, 7.0], color: [0.6, 0.3, 0.2], lift: 0.4 };
        self.model(scene, "trader.boat", &["tpl.trader_boat".into()], Some(block), target, t, dt);
        // The trader (a child with its own clips) waves while docked.
        let clip = if docked { "wave" } else { "idle" };
        if let Some(m) = self.pool.get_mut("trader.boat").filter(|m| m.home.is_some() && m.state != clip) {
            m.state = clip.to_string();
            scene.crossfade("tpl.trader_boat.trader", clip, 0.3);
        }
    }

    fn players(&mut self, scene: &mut GameScene, frame: &Frame, t: f64, dt: f32, cam: [f32; 3]) {
        let w = frame.world;
        let pool: Vec<String> = (1..=AVATAR_POOL).map(|i| format!("pool.avatar.{i}")).chain(["tpl.avatar".into()]).collect();
        for (pid, p) in w.connected() {
            if p.flare_until > w.now() {
                let name = format!("flare.{pid}");
                let flicker = 1.0 + 0.15 * ((t * 17.0) as f32 + pid as f32).sin();
                let at = p.pos + V3::new(0.0, 1.6, 0.0);
                self.light(scene, &name, at, [1.0, 0.35, 0.2], 2500.0 * flicker, 12.0);
            }
            if pid == frame.me {
                continue;
            }
            let key = format!("avatar.{pid}");
            let mut block = AVATAR;
            block.color = PLAYER_COLORS[pid as usize % PLAYER_COLORS.len()];
            let swims = p.in_water && !p.downed && p.seat.is_none();
            let target = Target { pos: p.pos, yaw: p.yaw, lying: p.downed, swim: swims, ..Target::default() };
            let Some(at) = self.model(scene, &key, &pool, Some(block), target, t, dt) else {
                continue;
            };
            let speed = self.pool.get(&key).map_or(0.0, |m| m.speed);
            let clip = match (&p.seat, p.downed) {
                (_, true) => "die",
                (Some(seat), _) if seat.index == 0 => "drive",
                (Some(_), _) => "sit",
                _ if swims && speed > swim::STROKE_SPEED => "swim",
                _ if swims => "tread",
                _ if speed > 5.5 || (p.sprinting && speed > 1.0) => "sprint",
                _ if speed > 0.6 => "walk",
                _ => "idle",
            };
            self.clip(scene, &key, clip);
            let head = if p.downed { 1.0 } else { 2.25 };
            self.tag(scene, &format!("{key}.tag"), &p.name, at + V3::new(0.0, head, 0.0), cam);
            if p.downed {
                let bob = 0.15 * (t as f32 * 3.0).sin();
                let spin = (t as f32 * 2.0) % std::f32::consts::TAU;
                self.marker(scene, &format!("{key}.downed"), at + V3::new(0.0, 2.0 + bob, 0.0), spin);
            }
        }
    }

    /// Waiting survivors stand where the scene put them; rescued ones on deck tiles
    /// that carry no structure.
    fn survivors(&mut self, scene: &mut GameScene, w: &World, t: f64, dt: f32) {
        let taken: BTreeSet<_> = w.raft.structures.values().flat_map(|s| s.cells()).collect();
        let free: Vec<_> = w.raft.tiles.iter().filter(|c| !taken.contains(c)).collect();
        for (i, state) in w.islands.list.iter().enumerate() {
            let Some(prefix) = self.islands.get(i) else {
                break;
            };
            let object = format!("{prefix}.survivor");
            let key = format!("survivor.{i}");
            let target = match state.survivor {
                SurvivorState::Hidden => continue,
                SurvivorState::Waiting => {
                    // Only the scene's NPC; the sim's spot is where it already stands.
                    let home = match self.pool.get(&key) {
                        Some(m) => m.home.clone(),
                        None => scene.try_object(&object).map(|o| Transform {
                            position: o.position(),
                            rotation: o.rotation(),
                            scale: o.scale(),
                        }),
                    };
                    let Some(home) = home else {
                        continue;
                    };
                    let [x, y, z] = home.position;
                    Target { pos: V3::new(x, y, z), yaw: home.rotation[1], snap: true, ..Target::default() }
                }
                SurvivorState::Rescued => {
                    let k = w.economy.survivors.iter().position(|&s| s as usize == i).unwrap_or(i);
                    let Some(cell) = free.get((k * 3) % free.len().max(1)) else {
                        continue;
                    };
                    let jitter = V3::new(unit(hash(i as i32, 5)), 0.0, unit(hash(i as i32, 6))) * (TILE * 0.2);
                    Target { pos: cell_center(**cell) + jitter, yaw: unit(hash(i as i32, 7)) * 3.1, ..Target::default() }
                }
            };
            let fallback = (state.survivor == SurvivorState::Rescued).then_some(SURVIVOR);
            self.model(scene, &key, &[object, "tpl.avatar".into()], fallback, target, t, dt);
        }
    }

    /// Chests open per player, shrines glow when active, the generator when filled,
    /// and the beacon core and lighthouse lamp once the core is in.
    fn islands(&mut self, scene: &mut GameScene, frame: &Frame) {
        let w = frame.world;
        for (i, (def, state)) in ISLANDS.iter().zip(&w.islands.list).enumerate() {
            let prefix = self.islands[i].clone();
            if !def.chest.is_empty() {
                let looted = state.chest_looted.contains(&frame.me);
                self.show(scene, &format!("{prefix}.chest.closed"), !looted);
                let open = format!("{prefix}.chest.open");
                if self.show(scene, &open, looted) && looted {
                    open_lid(scene, &open);
                }
            }
            for (k, &on) in state.shrines.iter().enumerate() {
                self.show(scene, &format!("{prefix}.shrine.{k}.glow"), on);
                self.show(scene, &format!("{prefix}.shrine.{k}.light"), on);
            }
        }
        let wreck = &self.islands[RUSTED_WRECK as usize];
        let filled = w.islands.list.get(RUSTED_WRECK as usize).is_some_and(|s| s.generator_metal >= GENERATOR_METAL);
        let names = [format!("{wreck}.generator.glow"), format!("{wreck}.generator.light")];
        for name in names {
            self.show(scene, &name, filled);
        }
        let atoll = &self.islands[LIGHTHOUSE_ATOLL as usize];
        let names = ["beacon_core", "beacon_core.light", "lamp", "lamp_glow"].map(|n| format!("{atoll}.{n}"));
        for name in names {
            self.show(scene, &name, w.islands.beacon_installed);
        }
    }

    /// Sets a scene object's visibility when it changes, remembering the scene's own.
    /// True when it changed after first sight (so not for a loaded save).
    fn show(&mut self, scene: &mut GameScene, name: &str, on: bool) -> bool {
        match self.shown.get_mut(name) {
            Some((now, _)) if *now == on => false,
            Some((now, _)) => {
                *now = on;
                scene.set_visible(name, on);
                true
            }
            None => {
                if !self.exists(scene, name) {
                    return false;
                }
                let Some(e) = scene.try_object(name).map(|o| o.entity()) else {
                    return false;
                };
                let default = scene.world().get::<Visibility>(e).is_none_or(|v| v.visible);
                self.shown.insert(name.to_string(), (on, default));
                if on != default {
                    scene.set_visible(name, on);
                }
                false
            }
        }
    }

    fn exists(&mut self, scene: &mut GameScene, name: &str) -> bool {
        *self
            .known
            .entry(name.to_string())
            .or_insert_with(|| scene.try_object(name).is_some())
    }

    /// Another mover uses `source` in place.
    fn claimed(&self, key: &str, source: &str) -> bool {
        self.pool.iter().any(|(k, m)| k != key && m.home.is_some() && m.source == source)
    }

    /// Shows `key` from the first of `tpls` that exists and is free, else `fallback`,
    /// easing toward `target`. Returns the eased position.
    #[allow(clippy::too_many_arguments)]
    fn model(
        &mut self,
        scene: &mut GameScene,
        key: &str,
        tpls: &[String],
        fallback: Option<Block>,
        target: Target,
        t: f64,
        dt: f32,
    ) -> Option<V3> {
        // Keep what this key shows while it is still a choice (a pool slot stays put).
        let kept = self.pool.get(key).map(|m| m.source.clone()).filter(|s| tpls.contains(s));
        let source = match kept {
            Some(s) => Some(s),
            None => tpls
                .iter()
                .find(|tpl| self.exists(scene, tpl) && !(tpl.starts_with("pool.") && self.claimed(key, tpl)))
                .cloned(),
        };
        if source.is_none() && fallback.is_none() {
            if let Some(m) = self.pool.remove(key) {
                release(scene, key, m);
            }
            return None;
        }
        let source = source.unwrap_or_default();
        if self.pool.get(key).is_some_and(|m| m.source != source) {
            if let Some(m) = self.pool.remove(key) {
                release(scene, key, m);
            }
        }
        if !self.pool.contains_key(key) {
            let in_place = (source.starts_with("pool.")
                || source.ends_with(".survivor")
                || ONE_INSTANCE.contains(&source.as_str()))
                && !self.claimed(key, &source);
            let (entity, home, rot, lift) = if source.is_empty() {
                let b = fallback?;
                let e = super::block(scene, key, target.pos.to_array(), target.yaw, b);
                (e, None, [0.0; 3], b.lift)
            } else if in_place {
                let o = scene.try_object(&source)?;
                let home = Transform { position: o.position(), rotation: o.rotation(), scale: o.scale() };
                let e = o.entity();
                scene.set_visible(&source, true);
                (e, Some(home), [0.0; 3], 0.0)
            } else {
                let tpl = scene.try_object(&source)?;
                let (rot, scale) = (tpl.rotation(), tpl.scale());
                let transform = Transform { position: target.pos.to_array(), rotation: rot, scale };
                let e = scene.spawn_copy_at_root(&source, key, transform)?;
                scene.set_visible(key, true);
                (e, None, rot, 0.0)
            };
            let m = Mover {
                entity,
                source,
                home,
                rot,
                lift,
                pos: target.pos,
                yaw: target.yaw,
                speed: 0.0,
                swim: swim::Pose::default(),
                seen: true,
                state: String::new(),
            };
            self.pool.insert(key.to_string(), m);
        }
        let m = self.pool.get_mut(key)?;
        m.seen = true;
        let jump = target.snap || m.pos.dist(target.pos) > 10.0;
        let k = if jump { 1.0 } else { 1.0 - (-10.0 * dt).exp() };
        let before = m.pos;
        m.pos = m.pos.lerp(target.pos, k);
        // Swimmers stroke the way they move, like the local avatar, not the way they look.
        let face = if target.swim && m.speed > swim::STROKE_SPEED {
            (before.x - m.pos.x).atan2(before.z - m.pos.z)
        } else {
            target.yaw
        };
        m.yaw = ease_angle(m.yaw, face, k);
        if dt > 0.0 && !jump {
            let v = before.with_y(0.0).dist(m.pos.with_y(0.0)) / dt;
            m.speed += (v - m.speed) * (1.0 - (-8.0 * dt).exp());
        }
        let (mut pos, mut pitch, mut roll) = (m.pos, 0.0, 0.0);
        if let Some(offset) = target.float {
            pos.y = wave(pos.x, pos.z, t) + offset;
            // Lean with the slope of the water under it.
            let gx = wave(pos.x + 0.5, pos.z, t) - wave(pos.x - 0.5, pos.z, t);
            let gz = wave(pos.x, pos.z + 0.5, t) - wave(pos.x, pos.z - 0.5, t);
            let (s, c) = m.yaw.sin_cos();
            pitch = (-s * gx - c * gz).atan();
            roll = (c * gx - s * gz).atan();
        }
        // Pool bodies play their die clip instead.
        let lift = if target.lying && m.home.is_none() {
            pitch = -FRAC_PI_2;
            0.25
        } else {
            m.lift
        };
        // In-place objects animate on their own; swimmers lean and rise either way.
        let p = if m.home.is_none() { target.pose } else { Pose::default() };
        m.swim.step(target.swim, m.speed > swim::STROKE_SPEED, dt);
        let (tilt, back, up) = m.swim.offsets(t as f32);
        let p = Pose { up: p.up + up, fwd: p.fwd - back, pitch: p.pitch - tilt, ..p };
        let (s, c) = m.yaw.sin_cos();
        let at = [pos.x - s * p.fwd, pos.y + lift + p.up, pos.z - c * p.fwd];
        let world = scene.world();
        if let Some(mut tr) = world.get_mut::<Transform>(m.entity) {
            tr.position = at;
            tr.rotation = [m.rot[0] + pitch + p.pitch, m.rot[1] + m.yaw + p.yaw, m.rot[2] + roll + p.roll];
        }
        Some(pos)
    }

    /// Crossfades an in-place model to `clip` when it changes (copies do not animate).
    fn clip(&mut self, scene: &mut GameScene, key: &str, clip: &str) {
        if let Some(m) = self.pool.get_mut(key).filter(|m| m.home.is_some() && m.state != clip) {
            m.state = clip.to_string();
            scene.crossfade(&m.source, clip, 0.25);
        }
    }

    /// A pooled point light.
    fn light(&mut self, scene: &mut GameScene, name: &str, at: V3, color: [f32; 3], intensity: f32, range: f32) {
        let m = self.plain(scene, name, |scene| super::light(scene, name, color));
        if let Some(mut tr) = scene.world().get_mut::<Transform>(m) {
            tr.position = at.to_array();
        }
        scene.set_light(name, Some(color), Some(intensity), Some(range));
    }

    /// A pooled, camera-facing name plate.
    fn tag(&mut self, scene: &mut GameScene, name: &str, text: &str, at: V3, cam: [f32; 3]) {
        let material = *self.tag_material.get_or_insert_with(|| {
            scene.create_material(MaterialAsset {
                name: "view.tag".into(),
                model: MaterialModel::Unlit,
                alpha_mode: AlphaMode::Blend,
                base_color: [1.0; 4],
                ..MaterialAsset::default()
            })
        });
        let e = self.plain(scene, name, |scene| {
            let e = scene.spawn_cube(name, Transform::default(), &CubeSpawn::new());
            scene.set_material(name, material);
            super::no_shadow(scene, name);
            e
        });
        let m = self.pool.get_mut(name).expect("pooled above");
        let mut aspect = None;
        if m.state != text {
            m.state = text.to_string();
            let style = TextStyle {
                size: 48.0,
                color: [255, 255, 255, 255],
                background: [20, 30, 40, 160],
                padding: 8,
                ..TextStyle::default()
            };
            aspect = scene.set_text(name, text, style).map(|[w, h]| w as f32 / h.max(1) as f32);
        }
        if let Some(mut tr) = scene.world().get_mut::<Transform>(e) {
            tr.position = at.to_array();
            tr.rotation = [0.0, (cam[0] - at.x).atan2(cam[2] - at.z), 0.0];
            if let Some(a) = aspect {
                tr.scale = [0.3 * a, 0.3, 0.01];
            }
        }
    }

    /// A pooled spinning red cube over a downed player.
    fn marker(&mut self, scene: &mut GameScene, name: &str, at: V3, spin: f32) {
        let e = self.plain(scene, name, |scene| {
            let e = scene.spawn_cube(name, Transform::default(), &CubeSpawn::new());
            scene.set_color(name, [1.0, 0.15, 0.1, 1.0]);
            scene.set_emissive(name, [3.0, 0.3, 0.2]);
            super::no_shadow(scene, name);
            e
        });
        if let Some(mut tr) = scene.world().get_mut::<Transform>(e) {
            *tr = Transform { position: at.to_array(), rotation: [0.6, spin, 0.6], scale: [0.35; 3] };
        }
    }

    /// A pooled object without easing, made by `spawn` when new.
    fn plain(&mut self, scene: &mut GameScene, name: &str, spawn: impl FnOnce(&mut GameScene) -> Entity) -> Entity {
        let m = self.pool.entry(name.to_string()).or_insert_with(|| Mover {
            entity: spawn(scene),
            source: String::new(),
            home: None,
            rot: [0.0; 3],
            lift: 0.0,
            pos: V3::ZERO,
            yaw: 0.0,
            speed: 0.0,
            swim: swim::Pose::default(),
            seen: true,
            state: String::new(),
        });
        m.seen = true;
        m.entity
    }
}

/// Swings the open chest's lid up once with its `open` clip, if it has one; without it
/// the closed/open swap alone shows the loot taken.
fn open_lid(scene: &mut GameScene, name: &str) {
    let Some(e) = scene.try_object(name).map(|o| o.entity()) else {
        return;
    };
    // The scene's clips loop; the lid should stop open.
    let found = scene.world().get_mut::<Animation>(e).is_some_and(|mut a| {
        a.clips.iter_mut().find(|c| c.name == "open").map(|c| c.repeat = TweenRepeat::Once).is_some()
    });
    if found {
        scene.play_animation(name, "open");
    }
}

/// The scene object a mover draws with.
fn object<'a>(key: &'a str, m: &'a Mover) -> &'a str {
    if m.home.is_some() {
        &m.source
    } else {
        key
    }
}

/// Despawns a copy or block; hides an in-place object and puts it back.
fn release(scene: &mut GameScene, key: &str, m: Mover) {
    match m.home {
        Some(home) => {
            if let Some(mut tr) = scene.world().get_mut::<Transform>(m.entity) {
                *tr = home;
            }
            scene.set_visible(&m.source, false);
        }
        None => {
            scene.despawn(key);
        }
    }
}

/// Copies stand in bind pose, so small enemies bob, sway and lunge in code: swimmers
/// wiggle and bob, walkers waddle while moving, both lunge nose-down on each attack
/// and reel while stunned.
fn enemy_pose(id: i32, e: &Enemy, t: f64) -> Pose {
    let def = e.kind.def();
    let tt = t as f32 + 3.0 * unit(hash(id, 23));
    // `attack_ready_at` moves one interval ahead the moment an attack lands.
    let since = (t - (e.attack_ready_at - def.attack_interval as f64)) as f32;
    let lunge = if (0.0..0.45).contains(&since) { (PI * since / 0.45).sin() } else { 0.0 };
    let moving = if matches!(e.state, EnemyState::Chase | EnemyState::Flee) { 1.0 } else { 0.0 };
    let reel = if t < e.stun_until { 0.25 * (5.0 * tt).sin() } else { 0.0 };
    if def.swims {
        Pose {
            up: 0.06 * (1.7 * tt).sin(),
            fwd: 0.8 * lunge,
            pitch: 0.04 * (1.7 * tt).cos() - 0.3 * lunge,
            yaw: (0.06 + 0.1 * moving) * (7.0 * tt).sin(),
            roll: 0.05 * (1.1 * tt).sin() + reel,
        }
    } else {
        let step = moving * (9.0 * tt).sin();
        Pose {
            up: 0.04 * step.abs() + 0.015 * (2.0 * tt).sin(),
            fwd: 0.5 * lunge,
            pitch: -0.35 * lunge,
            yaw: 0.0,
            roll: 0.1 * step + reel,
        }
    }
}

fn ease_angle(from: f32, to: f32, k: f32) -> f32 {
    wrap_angle(from + wrap_angle(to - from) * k)
}

fn debris_block(kind: ItemKind) -> Block {
    let (size, color) = match kind {
        ItemKind::Plank => ([1.2, 0.12, 0.3], [0.55, 0.38, 0.2]),
        ItemKind::PalmBundle => ([0.7, 0.25, 0.5], [0.35, 0.5, 0.2]),
        ItemKind::ScrapChunk => ([0.5, 0.3, 0.5], [0.5, 0.5, 0.52]),
        ItemKind::FloatingCrate => ([0.8, 0.8, 0.8], [0.5, 0.35, 0.18]),
        ItemKind::OilBarrel => ([0.6, 0.9, 0.6], [0.15, 0.15, 0.18]),
        ItemKind::CrawlerCarcass
        | ItemKind::GloomfinCarcass
        | ItemKind::ReefCrabCarcass
        | ItemKind::BruteCarcass => ([0.8, 0.3, 0.5], [0.4, 0.3, 0.32]),
        ItemKind::Fish(_) | ItemKind::CookedFish(_) => ([0.15, 0.15, 0.4], [0.6, 0.65, 0.7]),
        ItemKind::Flare => ([0.12, 0.12, 0.35], [0.9, 0.2, 0.15]),
        ItemKind::BeaconCore => ([0.4, 0.4, 0.4], [0.3, 0.8, 1.0]),
        _ => ([0.3, 0.2, 0.3], [0.9, 0.9, 0.85]),
    };
    Block { size, color, lift: 0.0 }
}

fn enemy_block(kind: EnemyKind) -> Block {
    let (size, color, lift) = match kind {
        EnemyKind::Gloomfin => ([0.9, 0.6, 2.2], [0.12, 0.15, 0.2], 0.0),
        EnemyKind::Crawler => ([0.9, 0.5, 1.2], [0.3, 0.35, 0.25], 0.25),
        EnemyKind::ReefCrab => ([1.0, 0.5, 0.8], [0.75, 0.3, 0.2], 0.25),
        EnemyKind::RigBrute => ([1.2, 2.2, 0.9], [0.4, 0.3, 0.3], 1.1),
        EnemyKind::Leviathan => ([4.0, 2.0, 14.0], [0.08, 0.1, 0.14], 0.0),
    };
    Block { size, color, lift }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raft_sim::enemies::EnemyHome;

    #[test]
    fn crabs_lunge_only_right_after_an_attack() {
        let mut e = Enemy::new(EnemyKind::ReefCrab, V3::new(0.0, 0.0, 0.0), EnemyHome::Night);
        e.state = EnemyState::Attack;
        e.attack_ready_at = 10.0 + e.kind.def().attack_interval as f64;
        assert!(enemy_pose(1, &e, 10.2).fwd > 0.3);
        assert_eq!(enemy_pose(1, &e, 11.0).fwd, 0.0);
    }
}
