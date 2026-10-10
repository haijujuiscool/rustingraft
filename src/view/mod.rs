//! Scene objects that mirror the `World`. Reads `Frame` and `LocalHud`; never sends
//! requests. Copies come from hidden `tpl.*` templates (docs/scene-names.md); a missing
//! raft template falls back to a plain block, so the raft stays visible while the scene
//! grows. Submodules: `ocean` (the faceted sea), `sky` (day/night light and fog),
//! `movers` (debris, enemies, boats, avatars, trader, survivors, island props) and `fx`
//! (event sounds, ambience and particle bursts).

mod fx;
mod movers;
mod ocean;
mod sky;

use crate::shared::{Frame, LocalHud};
use raft_sim::content::{farm, EnemyKind, ItemKind, PlaceableKind, TILE};
use raft_sim::economy::ShredJob;
use raft_sim::enemies::EnemyHome;
use raft_sim::raft::{cell_center, Cell, Raft, Railing, Side, Structure, StructureState};
use raft_sim::{EntityId, World, V3};
use rusting_engine::prelude::{
    CubeSpawn, Entity, GameScene, ParticleCommand, Sound, SoundId, Transform,
};
use rusting_engine::runtime::{effect_preset, MeshRenderer, ParticleEmitter, PointLight};
use std::collections::{BTreeSet, HashMap};
use std::f32::consts::FRAC_PI_2;

#[derive(Default)]
pub struct View {
    tiles: BTreeSet<Cell>,
    structures: BTreeSet<EntityId>,
    railings: BTreeSet<Railing>,
    /// Farms with output waiting: a bobbing marker above each.
    ready: BTreeSet<EntityId>,
    /// Rotation and scale of template parts (`<copy>/tpl.<kind>.grow`,
    /// `<copy>/tpl.shredder.cog`) at first sight.
    parts: HashMap<String, ([f32; 3], [f32; 3])>,
    /// `World::now`, smoothed so waves and bobbing stay smooth between snapshots.
    time: Option<f64>,
    /// Seconds of rendered frames, for flicker and spin.
    clock: f32,
    ocean: ocean::Ocean,
    sky: sky::Sky,
    fire: Fire,
    movers: movers::Movers,
    fx: fx::Fx,
    /// The scene's `Spawn Platform` is gone (despawned once the first tiles exist).
    platform_gone: bool,
}

impl View {
    pub fn sync(&mut self, scene: &mut GameScene, frame: &Frame, _hud: &LocalHud) {
        let w = frame.world;
        let dt = frame.dt.clamp(0.0, 0.25);
        self.clock += dt;
        let now = w.now();
        let t = match self.time {
            // Run on the frame clock and ease toward the world's, which steps per tick
            // on the host and per snapshot on clients.
            Some(t) if (t - now).abs() < 1.0 => {
                let ahead = t + dt as f64;
                ahead + (now - ahead) * (1.0 - (-3.0 * dt as f64).exp())
            }
            _ => now,
        };
        self.time = Some(t);
        let cam = scene.camera_ray().map_or([0.0, 2.0, 0.0], |(origin, _)| origin);
        self.sky.update(scene, &w.clock);
        self.ocean.update(scene, cam, t);
        self.raft(scene, w);
        self.fire.update(scene, w, self.clock);
        self.movers.update(scene, frame, t, dt, cam);
        self.fx.update(scene, frame, dt);
    }

    /// Despawns everything this view spawned (leaving a session).
    pub fn clear(&mut self, scene: &mut GameScene) {
        let none = BTreeSet::new();
        diff(scene, &mut self.tiles, &BTreeSet::new(), tile_name, |_, _| {});
        diff(scene, &mut self.structures, &none, structure_name, |_, _| {});
        diff(scene, &mut self.railings, &BTreeSet::new(), railing_name, |_, _| {});
        diff(scene, &mut self.ready, &none, ready_name, |_, _| {});
        self.fire.clear(scene);
        self.movers.clear(scene);
        self.fx.clear(scene);
        self.ocean.clear(scene);
        self.sky.clear(scene);
        self.parts.clear();
        self.time = None;
    }

    fn raft(&mut self, scene: &mut GameScene, w: &World) {
        let raft = &w.raft;
        diff(scene, &mut self.tiles, &raft.tiles, tile_name, |scene, cell| {
            let color = if (cell.0 + cell.1) % 2 == 0 {
                [0.42, 0.26, 0.13]
            } else {
                [0.37, 0.23, 0.115]
            };
            let block = Block::new([TILE - 0.06, 0.3, TILE - 0.06], color, -0.15);
            spawn(scene, "tpl.raft_floor", &tile_name(cell), cell_center(cell), 0.0, block);
        });
        // The platform only holds the player up until the deck exists.
        if !self.platform_gone && !self.tiles.is_empty() {
            scene.despawn("Spawn Platform");
            self.platform_gone = true;
        }
        let ids: BTreeSet<EntityId> = raft.structures.keys().copied().collect();
        diff(scene, &mut self.structures, &ids, structure_name, |scene, id| {
            let s = &raft.structures[&id];
            let tpl = format!("tpl.{}", snake(&format!("{:?}", s.kind)));
            let name = structure_name(id);
            if !spawn(scene, &tpl, &name, s.center(), s.rot as f32 * FRAC_PI_2, None) {
                // Footprint-sized block, world aligned (the cells are already turned).
                let block = structure_block(s);
                spawn(scene, &tpl, &name, s.center(), 0.0, Some(block));
            }
        });
        diff(scene, &mut self.railings, &raft.railings, railing_name, |scene, r| {
            // The template runs along X, as on a North or South edge.
            let yaw = if matches!(r.side, Side::East | Side::West) {
                FRAC_PI_2
            } else {
                0.0
            };
            let block = Block::new([TILE, 0.7, 0.12], [0.3, 0.19, 0.1], 0.35);
            let pos = Raft::edge_pos(r.cell, r.side);
            spawn(scene, "tpl.railing", &railing_name(r), pos, yaw, block);
        });

        // Farms: the `grow` part rises with the cycle; a marker bobs over waiting output.
        let ready: BTreeSet<EntityId> = raft
            .structures
            .iter()
            .filter(|(_, s)| s.farm().is_some_and(|f| f.ready))
            .map(|(id, _)| *id)
            .collect();
        diff(scene, &mut self.ready, &ready, ready_name, |scene, id| {
            let name = ready_name(id);
            scene.spawn_cube(name.as_str(), Transform::default(), &CubeSpawn::new());
            scene.set_color(&name, [1.0, 0.8, 0.25, 1.0]);
            scene.set_emissive(&name, [2.5, 1.6, 0.3]);
            no_shadow(scene, &name);
        });
        let spin = self.clock * 1.6;
        for (&id, s) in &raft.structures {
            if let Some(f) = s.farm() {
                if self.ready.contains(&id) {
                    if let Some(mut o) = scene.try_object(&ready_name(id)) {
                        let c = s.center();
                        o.set_position([c.x, c.y + 1.7 + 0.12 * (self.clock * 2.5).sin(), c.z]);
                        o.set_rotation([0.6, spin, 0.6]);
                        o.set_scale([0.32; 3]);
                    }
                }
                let cycle = farm(s.kind).map_or(1.0, |d| d.cycle.max(1.0));
                let grown = if f.ready {
                    1.0
                } else if f.running {
                    0.15 + 0.85 * (f.progress / cycle).clamp(0.0, 1.0)
                } else {
                    0.1
                };
                let grow = format!("{}/tpl.{}.grow", structure_name(id), snake(&format!("{:?}", s.kind)));
                self.part(scene, &grow, |rot, scale| {
                    (rot, [scale[0], scale[1] * grown, scale[2]])
                });
            }
            if s.kind == PlaceableKind::Shredder && !w.economy.shredder.queue.is_empty() {
                let turn = (self.clock * 7.0) % std::f32::consts::TAU;
                self.part(scene, &format!("{}/tpl.shredder.cog", structure_name(id)), |rot, scale| {
                    ([rot[0], rot[1] + turn, rot[2]], scale)
                });
            }
        }
    }

    /// Poses a template part from its rotation and scale at first sight.
    fn part(
        &mut self,
        scene: &mut GameScene,
        name: &str,
        pose: impl FnOnce([f32; 3], [f32; 3]) -> ([f32; 3], [f32; 3]),
    ) {
        let Some(mut o) = scene.try_object(name) else {
            return;
        };
        let (rot, scale) = *self
            .parts
            .entry(name.to_string())
            .or_insert((o.rotation(), o.scale()));
        let (rot, scale) = pose(rot, scale);
        o.set_rotation(rot);
        o.set_scale(scale);
    }
}

/// Scenario hooks, set as counters and reset to -1 once applied (host only):
/// `view_skip_to` jumps the clock to that many seconds into the day cycle,
/// `view_fire_level` lights the campfire at that level, `view_enemies` 1 puts an idle
/// Gloomfin off the bow (it lurks 12 m out) and a Crawler on deck, `view_parts` 1 adds a half-grown wood
/// farm beside the shredder and gives the shredder a minute of work. The changed world
/// is for `Session::replace_world`.
pub fn hooks(scene: &mut GameScene, w: &World) -> Option<World> {
    let mut out: Option<World> = None;
    for hook in ["view_skip_to", "view_fire_level", "view_enemies", "view_parts"] {
        let v = scene.counter_or(hook, -1);
        if v < 0 {
            continue;
        }
        scene.set_counter(hook, -1);
        let w = out.get_or_insert_with(|| w.clone());
        match hook {
            "view_skip_to" => w.clock.skip_to(v as f32),
            "view_fire_level" => {
                let fire = &mut w.economy.campfire;
                fire.level = v.clamp(1, 5) as u8;
                fire.fuel = fire.fuel.max(300.0);
            }
            "view_enemies" => {
                let mut ev = Vec::new();
                let fin = w.spawn_enemy(EnemyKind::Gloomfin, V3::new(-0.8, 0.0, -7.0), EnemyHome::Night, &mut ev);
                if let Some(e) = w.dynamic.enemies.get_mut(&fin) {
                    e.yaw = -std::f32::consts::FRAC_PI_2;
                }
                w.spawn_enemy(EnemyKind::Crawler, V3::new(2.6, 0.45, -3.2), EnemyHome::Raft, &mut ev);
            }
            "view_parts" => {
                let id = w.alloc_id();
                let mut farm_here = Structure::new(PlaceableKind::WoodFarm, (-2, -1), 0);
                if let StructureState::Farm(f) = &mut farm_here.state {
                    f.running = true;
                    f.progress = farm(PlaceableKind::WoodFarm).map_or(0.0, |d| d.cycle * 0.5);
                }
                w.raft.structures.insert(id, farm_here);
                let job = ShredJob { kind: ItemKind::Plank, by: None };
                w.economy.shredder.queue.extend([job; 30]);
            }
            _ => {}
        }
    }
    out
}

/// The campfire's flames, embers, smoke, light and crackle. Grows with the level; no
/// fire without fuel.
#[derive(Default)]
struct Fire {
    /// The campfire structure and its copy's scale at level 1.
    base: Option<(EntityId, [f32; 3])>,
    at: Option<V3>,
    shown: Option<(bool, u8)>,
    sound: Option<SoundId>,
}

const FIRE_LIGHT: &str = "raft.fire.light";
const FIRE_FX: [(&str, &str, f32); 3] = [
    ("raft.fire.flames", "fire", 0.1),
    ("raft.fire.embers", "embers", 0.35),
    ("raft.fire.smoke", "smoke", 1.1),
];

impl Fire {
    fn update(&mut self, scene: &mut GameScene, w: &World, clock: f32) {
        let Some((id, s)) = w.raft.find(PlaceableKind::Campfire) else {
            self.clear(scene);
            return;
        };
        let pos = s.center();
        let fire = &w.economy.campfire;
        let (lit, level) = (fire.lit(), fire.level.clamp(1, 5));
        // Flame scale: 0.6 at level 1 up to 1.8 at level 5, a clear step each level.
        let k = 0.3 + 0.3 * level as f32;
        if self.at.is_none() {
            light(scene, FIRE_LIGHT, [1.0, 0.6, 0.3]);
            for (name, preset, _) in FIRE_FX {
                let mut e = effect_preset(preset).map_or_else(ParticleEmitter::default, |p| p.emitter());
                e.autoplay = false;
                emitter(scene, name, e);
            }
        }
        if self.at != Some(pos) {
            self.at = Some(pos);
            if let Some(mut o) = scene.try_object(FIRE_LIGHT) {
                o.set_position([pos.x, pos.y + 0.9, pos.z]);
            }
            for (name, _, lift) in FIRE_FX {
                if let Some(mut o) = scene.try_object(name) {
                    o.set_position([pos.x, pos.y + lift, pos.z]);
                }
            }
            self.shown = None;
        }
        let model = structure_name(id);
        if self.base.map(|(b, _)| b) != Some(id) {
            self.base = scene.try_object(&model).map(|o| (id, o.scale()));
            self.shown = None;
        }
        if self.shown != Some((lit, level)) {
            self.shown = Some((lit, level));
            // Visibly bigger per level: the model, the flames and the smoke.
            if let Some((_, base)) = self.base {
                let grow = 1.0 + 0.1 * (level - 1) as f32;
                if let Some(mut o) = scene.try_object(&model) {
                    o.set_scale(base.map(|v| v * grow));
                }
            }
            for (name, preset, lift) in FIRE_FX {
                let Some(base) = effect_preset(preset).map(|p| p.emitter()) else {
                    continue;
                };
                if let Some(mut o) = scene.try_object(name) {
                    o.set_position([pos.x, pos.y + lift * k, pos.z]);
                }
                let Some(e) = scene.try_object(name).map(|o| o.entity()) else {
                    continue;
                };
                if let Some(mut em) = scene.world().get_mut::<ParticleEmitter>(e) {
                    // Taller (speed), wider (shape, size) and denser (rate) together.
                    em.size = base.size.map(|v| v * k);
                    em.shape_size = base.shape_size.map(|v| v * k);
                    em.speed = base.speed.map(|v| v * k);
                    em.rate = base.rate * k * k;
                    em.max_particles = (base.max_particles as f32 * k * k) as u32 + 1;
                }
                let cmd = if lit { ParticleCommand::Play } else { ParticleCommand::Stop };
                scene.particles(name, cmd);
            }
            match (lit, self.sound) {
                (true, None) => {
                    let sound = Sound {
                        volume: 0.7,
                        looped: true,
                        ..Sound::default()
                    };
                    self.sound = scene.play_sound_on(FIRE_FX[0].0, "sfx/fire_loop.ogg", sound);
                }
                (false, Some(id)) => {
                    scene.stop_sound(id);
                    self.sound = None;
                }
                _ => {}
            }
        }
        let flicker = 1.0
            + 0.12 * (clock * 11.0).sin()
            + 0.08 * (clock * 23.0 + 1.7).sin()
            + 0.05 * (clock * 37.0 + 0.4).sin();
        let intensity = if lit {
            (1400.0 + 500.0 * level as f32) * flicker
        } else {
            0.0
        };
        scene.set_light(FIRE_LIGHT, None, Some(intensity), Some(fire.light_radius()));
    }

    fn clear(&mut self, scene: &mut GameScene) {
        if self.at.is_none() {
            return;
        }
        scene.despawn(FIRE_LIGHT);
        for (name, _, _) in FIRE_FX {
            scene.despawn(name);
        }
        if let Some(id) = self.sound.take() {
            scene.stop_sound(id);
        }
        *self = Fire::default();
    }
}

/// A plain coloured box standing in for a missing template; `lift` is its centre's
/// height above the spawn point.
#[derive(Clone, Copy)]
struct Block {
    size: [f32; 3],
    color: [f32; 3],
    lift: f32,
}

impl Block {
    fn new(size: [f32; 3], color: [f32; 3], lift: f32) -> Option<Block> {
        Some(Block { size, color, lift })
    }
}

fn structure_block(s: &Structure) -> Block {
    let cells = s.cells();
    let span = |f: fn(&Cell) -> i32| {
        let lo = cells.iter().map(f).min().unwrap_or(0);
        let hi = cells.iter().map(f).max().unwrap_or(0);
        (hi - lo + 1) as f32 * TILE * 0.7
    };
    let (h, color) = match s.kind {
        PlaceableKind::Campfire => (0.3, [0.28, 0.27, 0.26]),
        PlaceableKind::Workbench => (0.9, [0.45, 0.3, 0.16]),
        PlaceableKind::Logbook => (1.1, [0.55, 0.42, 0.25]),
        PlaceableKind::Shredder => (1.1, [0.33, 0.36, 0.4]),
        PlaceableKind::WoodFarm => (0.4, [0.22, 0.38, 0.13]),
        PlaceableKind::MetalFarm => (0.4, [0.42, 0.42, 0.46]),
        PlaceableKind::OilFarm => (0.4, [0.12, 0.1, 0.09]),
        PlaceableKind::RadioStation => (1.5, [0.25, 0.32, 0.27]),
    };
    let (sx, sz) = (span(|c| c.0), span(|c| c.1));
    let (sx, sz) = match s.kind {
        PlaceableKind::Logbook | PlaceableKind::RadioStation => (sx * 0.5, sz * 0.5),
        PlaceableKind::Campfire => (sx * 0.75, sz * 0.75),
        _ => (sx, sz),
    };
    Block {
        size: [sx, h, sz],
        color,
        lift: h * 0.5,
    }
}

/// Spawns the keys `want` has and `have` lacks, despawns the reverse.
fn diff<K: Ord + Copy>(
    scene: &mut GameScene,
    have: &mut BTreeSet<K>,
    want: &BTreeSet<K>,
    name: fn(K) -> String,
    mut spawn: impl FnMut(&mut GameScene, K),
) {
    for &gone in have.difference(want) {
        scene.despawn(&name(gone));
    }
    for &new in want.difference(have) {
        spawn(scene, new);
    }
    have.clone_from(want);
}

/// Copies `template` to `pos` (its origin: the deck top at the footprint centre), turned
/// `yaw` on top of the template's own rotation. A missing template spawns `fallback`, or
/// nothing. True when the template was there.
fn spawn(
    scene: &mut GameScene,
    template: &str,
    name: &str,
    pos: V3,
    yaw: f32,
    fallback: Option<Block>,
) -> bool {
    let Some(tpl) = scene.try_object(template) else {
        if let Some(b) = fallback {
            block(scene, name, [pos.x, pos.y + b.lift, pos.z], yaw, b);
        }
        return false;
    };
    let [rx, ry, rz] = tpl.rotation();
    let scale = tpl.scale();
    let transform = Transform {
        position: [pos.x, pos.y, pos.z],
        rotation: [rx, ry + yaw, rz],
        scale,
    };
    if scene.spawn_copy_at_root(template, name, transform).is_some() {
        scene.set_visible(name, true);
    }
    true
}

/// A plain box `b.size` big centred on `center`.
fn block(scene: &mut GameScene, name: &str, center: [f32; 3], yaw: f32, b: Block) -> Entity {
    let t = Transform {
        position: center,
        rotation: [0.0, yaw, 0.0],
        scale: b.size,
    };
    let e = scene.spawn_cube(name, t, &CubeSpawn::new());
    let [r, g, bl] = b.color;
    scene.set_color(name, [r, g, bl, 1.0]);
    e
}

/// A bare object (no mesh) to carry a light, emitter or environment component.
fn bare(scene: &mut GameScene, name: &str) -> Entity {
    let e = scene.spawn_cube(name, Transform::default(), &CubeSpawn::new());
    scene.world().entity_mut(e).remove::<MeshRenderer>();
    e
}

fn light(scene: &mut GameScene, name: &str, color: [f32; 3]) -> Entity {
    let e = bare(scene, name);
    scene.world().entity_mut(e).insert(PointLight {
        color,
        intensity: 0.0,
        range: 1.0,
        shadows: false,
        radius: 0.0,
    });
    e
}

fn emitter(scene: &mut GameScene, name: &str, emitter: ParticleEmitter) -> Entity {
    let e = bare(scene, name);
    scene.world().entity_mut(e).insert(emitter);
    e
}

fn no_shadow(scene: &mut GameScene, name: &str) {
    if let Some(e) = scene.try_object(name).map(|o| o.entity()) {
        if let Some(mut r) = scene.world().get_mut::<MeshRenderer>(e) {
            r.cast_shadows = false;
        }
    }
}

fn tile_name(cell: Cell) -> String {
    format!("raft.tile.{}.{}", cell.0, cell.1)
}

fn structure_name(id: EntityId) -> String {
    format!("raft.structure.{id}")
}

fn railing_name(r: Railing) -> String {
    format!("raft.railing.{}.{}.{:?}", r.cell.0, r.cell.1, r.side)
}

fn ready_name(id: EntityId) -> String {
    format!("raft.structure.{id}.ready")
}

/// `WoodFarm` -> `wood_farm`, for template names from enum variants.
pub fn snake(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() && i > 0 {
            out.push('_');
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}

/// `Fish(Cod)` -> `fish`: a variant's name without its payload, snake cased.
fn variant(debug: &str) -> String {
    snake(debug.split('(').next().unwrap_or(debug))
}

#[cfg(test)]
mod tests {
    use raft_sim::content::{
        BoatKind, EnemyKind, ItemClass, ItemKind, PlaceableKind, ISLANDS, LIGHTHOUSE_ATOLL,
        RUSTED_WRECK,
    };

    /// Every template and scene object the view asks for is in the real scene, so no
    /// fallback block shows. Prints the missing names.
    #[test]
    fn scene_has_every_object_the_view_uses() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/scenes/main.rscene");
        let text = std::fs::read_to_string(path).expect("scene file");
        let mut want: Vec<String> = ["tpl.raft_floor", "tpl.railing", "tpl.avatar", "tpl.trader_boat", "tpl.trader_boat.trader"]
            .map(String::from)
            .to_vec();
        want.extend((1..=7).map(|i| format!("pool.avatar.{i}")));
        use PlaceableKind::*;
        let placeables = [Campfire, Workbench, Logbook, Shredder, WoodFarm, MetalFarm, OilFarm, RadioStation];
        want.extend(placeables.map(|k| format!("tpl.{}", super::snake(&format!("{k:?}")))));
        want.extend(["tpl.wood_farm.grow", "tpl.metal_farm.grow", "tpl.oil_farm.grow", "tpl.shredder.cog"].map(String::from));
        let debris = ItemKind::all().into_iter().filter(|k| k.class() == ItemClass::Debris);
        want.extend(debris.map(|k| format!("tpl.{}", super::variant(&format!("{k:?}")))));
        want.extend(["tpl.held.fish", "tpl.held.bandage", "tpl.held.salve", "tpl.held.flare"].map(String::from));
        use EnemyKind::*;
        let enemies = [Gloomfin, Crawler, ReefCrab, RigBrute, Leviathan];
        want.extend(enemies.map(|k| format!("tpl.enemy.{}", super::variant(&format!("{k:?}")))));
        let boats = [BoatKind::Dinghy, BoatKind::Skiff, BoatKind::MotorLaunch];
        want.extend(boats.map(|k| format!("tpl.boat.{}", super::variant(&format!("{k:?}")))));
        let key = |i: usize| ISLANDS[i].name.to_lowercase().replace(' ', "_");
        for (i, d) in ISLANDS.iter().enumerate() {
            if d.survivor {
                want.push(format!("{}.survivor", key(i)));
            }
            if !d.chest.is_empty() {
                want.extend(["closed", "open"].map(|s| format!("{}.chest.{s}", key(i))));
            }
            for k in 0..d.shrines {
                want.extend(["glow", "light"].map(|s| format!("{}.shrine.{k}.{s}", key(i))));
            }
        }
        let wreck = key(RUSTED_WRECK as usize);
        want.extend(["glow", "light"].map(|s| format!("{wreck}.generator.{s}")));
        let atoll = key(LIGHTHOUSE_ATOLL as usize);
        want.extend(["beacon_core", "beacon_core.light", "lamp", "lamp_glow"].map(|s| format!("{atoll}.{s}")));
        let missing: Vec<&String> = want.iter().filter(|n| !text.contains(&format!("\"name\": \"{n}\""))).collect();
        assert!(missing.is_empty(), "missing from the scene: {missing:?}");
    }

    #[test]
    fn snake_case_names() {
        assert_eq!(super::snake("WoodFarm"), "wood_farm");
        assert_eq!(super::snake("Campfire"), "campfire");
        assert_eq!(super::variant("CookedFish(Cod)"), "cooked_fish");
        assert_eq!(super::variant("ReefCrabCarcass"), "reef_crab_carcass");
    }
}
