//! The local player: controller -> `Request`s, interaction, `LocalHud`. Never draws
//! egui (that is `ui`) and never spawns world mirror objects (that is `view`); its own
//! scene objects are named `local.*` (held tool, harpoon bolt and rope, bobber and
//! line, build ghost).
//!
//! Test counters (set every frame): `player_swimming`, `player_first_person`,
//! `player_slot`, `player_fishing` (0 none, 1 line out, 2 reeling), `player_ghost`
//! (0 none, 1 invalid, 2 valid), `player_dragging` (debris pulled by me),
//! `player_shots`, `player_casts`, `player_place_requests`, `player_boat`,
//! `player_grapple_gap` (cm from the body to the grapple target when the last pull
//! ended, -1 before one).

mod boat;
mod build;
#[cfg(test)]
mod fixtures;
mod interact;
mod reel;
pub mod swim;

use crate::shared::{FishingUi, Frame, LocalHud, Window};
use boat::Drive;
use interact::Act;
use raft_sim::boats::seat_pos;
use raft_sim::content::{
    harpoon, ItemKind, PlaceableKind, RecipeId, CAST_RANGE, DECK_Y, GRAPPLE_SPEED, MELEE_RANGE,
    MELEE_SWING, PLACE_RANGE, REVIVE_SECONDS, SWIM_FAST_SPEED, SWIM_SPEED,
};
use raft_sim::debris::wave;
use raft_sim::math::facing;
use raft_sim::{EntityId, Event, Player, PlayerId, Request, World, V3};
use reel::Reel;
use rusting_engine::prelude::{GameScene, PlayerController, Transform};
use std::collections::HashSet;
use std::f32::consts::PI;
use std::sync::OnceLock;

/// The scene's player controller object.
pub const PLAYER: &str = "Player";
/// Body centre above the feet: capsule half height 0.6 + radius 0.3 in the scene.
pub const FEET_OFFSET: f32 = 0.9;
const CAMERA: &str = "Game Camera";
const AVATAR: &str = "Player Avatar";
/// Feet below this height and off the deck count as in the water.
const WATER_LINE: f32 = 0.2;
const SEND_INTERVAL: f32 = 1.0 / 20.0;
/// First-person camera above the body centre; lying on the deck while downed.
const EYE: [f32; 3] = [0.0, 0.7, 0.0];
const DOWNED_EYE: [f32; 3] = [0.0, -0.5, 0.0];
/// Third-person camera distance behind the shoulder.
const ORBIT: f32 = 5.0;
/// Third-person camera offset (right, up) from the orbit centre: over the right
/// shoulder so the avatar sits left of the crosshair.
const SHOULDER: [f32; 3] = [0.9, 1.2, 0.0];
/// The third-person camera keeps this far off walls.
const CAMERA_GAP: f32 = 0.3;
/// Held tools are drawn this much larger in third person so they read.
const HELD_SCALE: f32 = 1.5;
/// Seconds the avatar keeps facing the aim after a tool is used.
const FACE_HOLD: f32 = 0.8;
const BOLT_SPEED: f32 = 60.0;
/// Seconds the bolt takes to come back.
const BOLT_RETURN: f32 = 0.25;
/// Debris and enemies count as hit within these radii of the aim ray.
const DEBRIS_RADIUS: f32 = 1.0;
const ENEMY_RADIUS: f32 = 1.3;
/// Seconds fishing clicks are ignored after a fishing request (the mirror lags).
const FISH_CLICK_PAUSE: f32 = 0.4;

const HELD: &str = "local.held";
const BOLT: &str = "local.bolt";
const ROPE: &str = "local.rope";
const BOBBER: &str = "local.bobber";
const LINE: &str = "local.line";
const GHOST: &str = "local.ghost";
const GHOST_BOX: &str = "local.ghost/tpl.ghost.box";
const LOCAL_NAMES: [&str; 6] = [HELD, BOLT, ROPE, BOBBER, LINE, GHOST];

/// Raycast hits on these are not surfaces (templates, our own and the view's
/// moving things).
const SKIP: [&str; 7] = ["tpl.", "local.", "avatar.", "pool.", "debris.", "enemy.", PLAYER];

/// The scene's controller settings, read on the first frame of the process: land
/// movement and the third-person camera go back to these.
static BASE: OnceLock<PlayerController> = OnceLock::new();

#[derive(Default)]
pub struct Local {
    pub hud: LocalHud,
    /// The player the controller was last placed for; a new one teleports it to spawn.
    placed: Option<PlayerId>,
    send_timer: f32,
    /// Placement mode: recipe and quarter turns.
    build: Option<(RecipeId, u8)>,
    ghost_valid: Option<bool>,
    /// The reel-in bar and the bite it belongs to (`Fishing::bite_at`).
    reel: Option<Reel>,
    reel_bite: f64,
    fish_pause: f32,
    cancel_sent: bool,
    shot: Option<Shot>,
    swing_left: f32,
    reviving: Option<PlayerId>,
    drive: Option<Drive>,
    /// Out of oil: the throttle stays at 0 until W/S is released.
    oil_lock: bool,
    /// Template shown as `local.held` and whether it hangs off the camera.
    held: Option<(&'static str, bool)>,
    /// `local.*` objects spawned (or tried: a missing template is not retried).
    shown: HashSet<&'static str>,
    anim: &'static str,
    avatar_shown: Option<bool>,
    /// Avatar yaw relative to the body, and how far into the water poses.
    avatar_yaw: f32,
    swim: swim::Pose,
    face_left: f32,
    /// Camera yaw last frame, and whether it turned since the avatar last walked: on the
    /// spot the avatar faces where the camera looks.
    look_yaw: f32,
    face_camera: bool,
    /// Body-to-target distance (cm) when the last grapple ended.
    grapple_gap: Option<i32>,
    shots: i32,
    casts: i32,
    place_requests: i32,
}

struct Shot {
    to: V3,
    aim: Aim,
    age: f32,
    flight: f32,
    /// Age when the bolt started back.
    back: Option<f32>,
    dashed: bool,
}

#[derive(Clone, Copy)]
enum Aim {
    Debris(EntityId),
    Enemy,
    /// Grapple: where the body centre is pulled to.
    Surface(V3),
    Miss,
}

/// This frame's input; all idle while a window has the pointer.
#[derive(Default)]
struct Keys {
    fire: bool,
    aim: bool,
    aim_pressed: bool,
    camera: bool,
    build: bool,
    rotate: bool,
    cancel: bool,
    interact: bool,
    interact_held: bool,
    slot: Option<u8>,
    jump: bool,
    dive: bool,
    sprint: bool,
    /// W - S and D - A.
    forward: f32,
    turn: f32,
}

impl Keys {
    fn read(scene: &GameScene) -> Keys {
        let axis = |a: &str, b: &str| scene.held(a) as i32 as f32 - scene.held(b) as i32 as f32;
        Keys {
            fire: scene.pressed("use"),
            aim: scene.held("aim"),
            aim_pressed: scene.pressed("aim"),
            camera: scene.pressed("camera"),
            build: scene.pressed("build"),
            rotate: scene.pressed("rotate"),
            cancel: scene.pressed("pause"),
            interact: scene.pressed("interact"),
            interact_held: scene.held("interact"),
            slot: (1..=6u8).find(|i| scene.pressed(&format!("slot{i}"))),
            jump: scene.held("player.jump"),
            dive: scene.held("player.crouch"),
            sprint: scene.held("player.sprint"),
            forward: axis("player.forward", "player.back"),
            turn: axis("player.right", "player.left"),
        }
    }
}

impl Local {
    /// `ui_modal`: a window or menu has the pointer, so ignore gameplay input.
    /// `cursor_free`: the cursor was held free (focus loss), so a click only grabs it.
    pub fn update(
        &mut self,
        scene: &mut GameScene,
        frame: &Frame,
        ui_modal: bool,
        cursor_free: bool,
        out: &mut Vec<Request>,
    ) {
        self.hud.open = None;
        let world = frame.world;
        let Some(me) = world.player(frame.me) else {
            return;
        };
        let Some(pc) = scene.player(PLAYER) else {
            return;
        };
        let base = *BASE.get_or_init(|| pc);
        if self.placed != Some(frame.me) {
            self.placed = Some(frame.me);
            for name in LOCAL_NAMES {
                scene.despawn(name);
            }
            teleport(scene, me.pos);
        }
        for event in frame.events {
            match event {
                Event::PlayerTeleport { player, pos } if *player == frame.me => {
                    teleport(scene, *pos)
                }
                Event::Denied { player, reason } if *player == frame.me => {
                    // The sim kept the hull where it was: drive on from its copy.
                    if let Some(d) = &mut self.drive {
                        if matches!(reason.as_str(), "Out of oil" | "Moved too far") {
                            if let Some(b) = world.dynamic.boats.get(&d.boat) {
                                *d = Drive::new(d.boat, b);
                            }
                            d.speed = 0.0;
                            self.oil_lock |= reason == "Out of oil";
                        }
                    }
                    // The fish stays hooked: show the bar again.
                    if reason == "Your sac is full" {
                        self.reel_bite = -1.0;
                    }
                }
                _ => {}
            }
        }
        scene.set_mouse_look(PLAYER, !ui_modal);
        if self.hud.selected_slot == 0 {
            self.hud.selected_slot = 1;
        }

        let Some(body) = scene.try_object(PLAYER).map(|o| V3::from_array(o.position())) else {
            return;
        };
        let mut feet = body - V3::new(0.0, FEET_OFFSET, 0.0);
        let mut k = if ui_modal { Keys::default() } else { Keys::read(scene) };
        k.fire &= !cursor_free;
        let now = world.now();
        // A hitch must not move the hull past the sim's MAX_MOVE in one send.
        let dt = frame.dt.min(0.1);

        // ---- boats: seat 0 drives
        match me.seat {
            Some(s) if s.index == 0 => {
                if self.drive.as_ref().map(|d| d.boat) != Some(s.boat) {
                    self.drive = world.dynamic.boats.get(&s.boat).map(|b| Drive::new(s.boat, b));
                }
                self.oil_lock &= k.forward != 0.0;
                let throttle = if self.oil_lock { 0.0 } else { k.forward };
                if let Some(d) = &mut self.drive {
                    d.step(throttle, k.turn, dt, &world.raft);
                }
            }
            _ => self.drive = None,
        }
        if let Some(s) = me.seat {
            let driven = self.drive.as_ref().map(Drive::hull);
            if let Some(hull) = driven.as_ref().or(world.dynamic.boats.get(&s.boat)) {
                feet = seat_pos(hull, s.index);
                put_feet(scene, feet);
            }
        }

        // ---- water
        let seated = me.seat.is_some();
        let surface = wave(feet.x, feet.z, now);
        let in_water = !seated
            && feet.y < WATER_LINE
            && !world.raft.on_deck(feet)
            && !(pc.grounded && feet.y > -0.6);
        if in_water && feet.y < -8.0 {
            // ponytail: hard floor for anything that slipped past the float.
            feet.y = surface - swim::FLOAT_DEPTH;
            teleport(scene, feet);
        }
        self.hud.swimming = in_water;

        if k.camera {
            self.hud.camera_first_person = !self.hud.camera_first_person;
        }
        let first = self.hud.camera_first_person;
        let frozen = me.downed || seated || ui_modal;
        let vertical = in_water
            .then(|| swim::vertical_speed(feet.y, surface, k.jump, k.dive, pc.vertical_speed, dt));
        let eye = if me.downed { DOWNED_EYE } else { EYE };
        let (offset, orbit) = if first { (eye, 0.0) } else { third_person(scene, &pc, &base, body) };
        let lift = if first { 0.0 } else { base.camera_height };
        let origin = body + frame_of(pc.yaw, offset) + V3::new(0.0, lift, 0.0);
        let (sp, cp) = pc.pitch.sin_cos();
        let dir = V3::new(-pc.yaw.sin() * cp, sp, -pc.yaw.cos() * cp);
        scene.set_player(PLAYER, |c| {
            c.walk_speed = match (frozen, in_water) {
                (true, _) => 0.0,
                (false, true) => SWIM_SPEED,
                (false, false) => base.walk_speed,
            };
            c.sprint_multiplier = if in_water {
                SWIM_FAST_SPEED / SWIM_SPEED
            } else {
                base.sprint_multiplier
            };
            c.jump_speed = if frozen || in_water { 0.0 } else { base.jump_speed };
            c.gravity = if in_water || seated { 0.0 } else { base.gravity };
            c.collision_mask = if seated { 0 } else { base.collision_mask };
            if let Some(v) = vertical {
                c.vertical_speed = v;
            } else if seated {
                c.vertical_speed = 0.0;
            }
            c.camera_distance = orbit;
            c.camera_offset = offset;
            // The avatar's facing is ours (`pose_avatar`).
            c.turn_speed = 0.0;
        });
        if self.avatar_shown != Some(!first) {
            self.avatar_shown = Some(!first);
            scene.set_visible(AVATAR, !first);
        }

        // Downed: the sim keeps the last position, so the body must not travel.
        if me.downed && pc.dash_left > 0.0 {
            scene.dash(PLAYER, [0.0; 3], 0.0);
        }
        // Climb out: Space at a ledge (raft edge, rock) boosts up and over it.
        if in_water && k.jump && !me.downed && pc.dash_left <= 0.0 {
            let fwd = facing(pc.yaw);
            let from = feet + fwd * 0.75 + V3::new(0.0, 2.8, 0.0);
            if let Some((top, normal)) = ray(scene, from, V3::new(0.0, -1.0, 0.0), 4.0) {
                let rise = top.y - feet.y;
                if rise > 0.3 && rise < 2.8 && normal.y > 0.6 {
                    let v = fwd * 1.5 + V3::new(0.0, 5.0, 0.0);
                    scene.dash(PLAYER, v.to_array(), (rise + 0.35) / 5.0);
                }
            }
        }

        // ---- send the body
        self.send_timer -= dt;
        if self.send_timer <= 0.0 {
            self.send_timer = (self.send_timer + SEND_INTERVAL).max(0.0);
            if let Some(d) = &self.drive {
                out.push(Request::BoatTransform {
                    pos: d.pos,
                    yaw: d.yaw,
                    speed: d.speed,
                });
            } else if !seated && !me.downed {
                out.push(Request::PlayerTransform {
                    pos: feet,
                    yaw: pc.yaw,
                    in_water,
                    sprinting: k.sprint,
                });
            }
        }

        // ---- hotbar
        let busy = me.downed || seated;
        if let Some(slot) = k.slot {
            if slot != self.hud.selected_slot {
                self.hud.selected_slot = slot;
                self.build = None;
            }
        }
        let slot = self.hud.selected_slot;
        let hand = body
            + frame_of(pc.yaw, if first { [0.3, 0.42, -0.45] } else { [0.3, -0.05, -0.3] });
        self.hud.aiming = k.aim && !busy && self.build.is_none() && matches!(slot, 1 | 6);

        // ---- build mode
        if self.build.is_none() {
            self.hud.ghost = None;
        }
        if let Some((recipe, rot)) = self.build {
            if busy || k.cancel || k.aim_pressed || k.build {
                // The ghost stays in the hud this frame so ui does not also take Esc.
                self.build = None;
            } else {
                let rot = if k.rotate { (rot + 1) % 4 } else { rot };
                self.build = Some((recipe, rot));
                let aim = deck_point(origin, dir, feet).unwrap_or(feet + facing(pc.yaw) * 3.0);
                let g = build::ghost(world, recipe, rot, aim, feet);
                if k.fire {
                    out.push(build::request(&g));
                    self.place_requests += 1;
                }
                self.hud.ghost = Some(g);
            }
        } else if k.build && !busy && world.near_structure(frame.me, PlaceableKind::Workbench).is_some()
        {
            self.hud.open = Some(Window::Workbench);
        }
        let building = self.build.is_some();

        // ---- tools
        let tools = !busy && !building;
        if tools && k.fire && slot == 1 && self.shot.is_none() && now >= me.harpoon_ready_at {
            self.fire(scene, world, me, feet, origin, dir, hand, out);
        }
        self.swing_left = (self.swing_left - dt).max(0.0);
        if tools && k.fire && slot == 3 && self.swing_left <= 0.0 && now >= me.melee_ready_at {
            self.swing_left = MELEE_SWING;
            let fwd = facing(pc.yaw);
            let hit = world
                .dynamic
                .enemies
                .iter()
                .filter(|(_, e)| {
                    let to = (e.pos - feet).with_y(0.0);
                    let reach = MELEE_RANGE.max(e.kind.def().attack_range);
                    to.length_xz() <= reach && to.normalize().dot(fwd) > 0.5
                })
                .min_by(|a, b| a.1.pos.dist_xz(feet).total_cmp(&b.1.pos.dist_xz(feet)));
            if let Some((id, _)) = hit {
                out.push(Request::MeleeHit(*id));
            }
        }
        if tools && k.fire && slot == 4 && me.food.is_some() {
            out.push(Request::Eat);
        }
        if tools && k.fire && slot == 5 && me.heal.is_some() {
            out.push(Request::Heal);
        }
        self.fishing(me, now, dt, tools && slot == 6, k.fire, origin, dir, feet, out);
        self.step_shot(scene, world, frame.me, hand, &pc, dt);
        if tools && k.fire {
            self.face_left = FACE_HOLD;
        }
        // Walking keeps the walk facing.
        let walking = pc.velocity[0].hypot(pc.velocity[2]) >= 0.1;
        self.face_camera = !walking && (self.face_camera || (pc.yaw - self.look_yaw).abs() > 1e-4);
        self.look_yaw = pc.yaw;
        self.face_left = (self.face_left - dt).max(0.0);
        let face_aim = !busy
            && (k.aim
                || building
                || self.face_left > 0.0
                || self.face_camera
                || self.swing_left > 0.0
                || self.shot.is_some()
                || me.fishing.is_some());

        // ---- interact (E), revive (hold E)
        let target = if building || me.downed { None } else { interact::target(frame, feet) };
        let revive = match &target {
            Some(t) => match t.act {
                Act::Revive(pid) if k.interact_held => Some(pid),
                _ => None,
            },
            None => None,
        };
        if revive != self.reviving {
            if self.reviving.is_some() {
                out.push(Request::StopRevive);
            }
            if let Some(pid) = revive {
                out.push(Request::StartRevive(pid));
            }
            self.reviving = revive;
        }
        self.hud.prompt = target.as_ref().map(|t| match t.act {
            Act::Revive(_) if self.reviving.is_some() => {
                let pct = (me.revive_progress / REVIVE_SECONDS * 100.0).min(100.0);
                format!("E  {}  {pct:.0}%", t.text)
            }
            _ => format!("E  {}", t.text),
        });
        if k.interact && !me.downed {
            match target.map(|t| t.act) {
                Some(Act::Req(req)) => out.push(req),
                Some(Act::Open(w)) => self.hud.open = Some(w),
                _ => {}
            }
        }

        // ---- local visuals
        self.show_held(scene, me, first, busy || building);
        self.show_ghost(scene);
        self.show_line(scene, me, now, hand);
        self.pose_avatar(scene, &pc, &base, face_aim, in_water && !me.downed, now, dt);
        let clip = self.clip(me, &pc, in_water, k.sprint);
        if clip != self.anim {
            self.anim = clip;
            scene.crossfade(AVATAR, clip, 0.15);
        }

        let dragging = world.dynamic.debris.values().filter(|d| d.dragged_by == Some(frame.me));
        let fishing = match (&me.fishing, &self.reel) {
            (None, _) => 0,
            (Some(_), None) => 1,
            (Some(_), Some(_)) => 2,
        };
        let ghost = self.hud.ghost.as_ref().map_or(0, |g| 1 + g.valid as i32);
        let counters = [
            ("player_swimming", in_water as i32),
            ("player_first_person", first as i32),
            ("player_slot", slot as i32),
            ("player_fishing", fishing),
            ("player_ghost", ghost),
            ("player_dragging", dragging.count() as i32),
            ("player_shots", self.shots),
            ("player_casts", self.casts),
            ("player_place_requests", self.place_requests),
            ("player_boat", seated as i32),
            ("player_grapple_gap", self.grapple_gap.unwrap_or(-1)),
        ];
        for (name, value) in counters {
            scene.set_counter(name, value);
        }
    }

    /// Leaving a session (menu, or a new one starting): our `local.*` objects go and the
    /// avatar hides until the next `update` shows it again.
    pub fn clear(&mut self, scene: &mut GameScene) {
        for name in LOCAL_NAMES {
            scene.despawn(name);
        }
        scene.set_visible(AVATAR, false);
        *self = Local::default();
    }

    /// Game over: `update` no longer runs, so it never played "die" if everyone went
    /// down on one tick (solo).
    pub fn lie_down(&mut self, scene: &mut GameScene) {
        if self.anim != "die" {
            self.anim = "die";
            scene.crossfade(AVATAR, "die", 0.15);
        }
    }

    /// Called for `UiAction::BeginPlacement`.
    pub fn begin_placement(&mut self, recipe: RecipeId) {
        if build::placeable(recipe) {
            self.build = Some((recipe, 0));
        }
    }

    /// Harpoon: the nearest debris or enemy on the aim ray is pulled or hit; a solid
    /// surface grapples; nothing in range is a miss. The bolt flies either way.
    #[allow(clippy::too_many_arguments)]
    fn fire(
        &mut self,
        scene: &mut GameScene,
        world: &World,
        me: &Player,
        feet: V3,
        origin: V3,
        dir: V3,
        hand: V3,
        out: &mut Vec<Request>,
    ) {
        let range = harpoon(me.gear.harpoon).range;
        // The third-person orbit centre sits a little behind the feet.
        let reach = range + 3.0;
        let mut best: Option<(f32, Aim, V3)> = None;
        let mut take = |t: f32, aim: Aim, at: V3| {
            if t <= reach
                && feet.dist_xz(at) <= range
                && best.as_ref().is_none_or(|(b, ..)| t < *b)
            {
                best = Some((t, aim, at));
            }
        };
        for (id, d) in &world.dynamic.debris {
            if d.dragged_by.is_none() {
                if let Some(t) = ray_sphere(origin, dir, d.pos, DEBRIS_RADIUS) {
                    take(t, Aim::Debris(*id), d.pos);
                }
            }
        }
        let mut enemies = Vec::new();
        for (id, e) in &world.dynamic.enemies {
            let at = e.pos + V3::new(0.0, 0.4, 0.0);
            if let Some(t) = ray_sphere(origin, dir, at, ENEMY_RADIUS) {
                take(t, Aim::Enemy, at);
                enemies.push((t, *id));
            }
        }
        if let Some((point, normal)) = ray(scene, origin, dir, reach) {
            let lift = if normal.y > 0.6 { 0.9 } else { 0.0 };
            let body = point + normal * 0.5 + V3::new(0.0, lift, 0.0);
            take((point - origin).length(), Aim::Surface(body), point);
        }
        let (aim, to) = match best {
            Some((t, aim, at)) => {
                match aim {
                    Aim::Debris(id) => out.push(Request::HarpoonDebris(id)),
                    Aim::Enemy => {
                        if let Some((_, id)) = enemies.iter().find(|(e, _)| *e == t) {
                            out.push(Request::HarpoonEnemy(*id));
                        }
                    }
                    _ => {}
                }
                (aim, at)
            }
            None => (Aim::Miss, origin + dir * range),
        };
        self.shots += 1;
        show(scene, &mut self.shown, true, "tpl.harpoon_bolt", BOLT);
        show(scene, &mut self.shown, true, "tpl.rope", ROPE);
        self.shot = Some(Shot {
            to,
            aim,
            age: 0.0,
            flight: (hand.dist(to) / BOLT_SPEED).max(0.05),
            back: None,
            dashed: false,
        });
    }

    /// Bolt out, attached (dragging debris / grappling), and back.
    fn step_shot(
        &mut self,
        scene: &mut GameScene,
        world: &World,
        me: PlayerId,
        hand: V3,
        pc: &PlayerController,
        dt: f32,
    ) {
        let Some(s) = &mut self.shot else {
            return;
        };
        s.age += dt;
        let mut pos = s.to;
        if s.age < s.flight {
            pos = hand.lerp(s.to, s.age / s.flight);
        } else if s.back.is_none() {
            let held = s.age - s.flight;
            let done = match s.aim {
                Aim::Debris(id) => match world.dynamic.debris.get(&id) {
                    Some(d) if d.dragged_by == Some(me) => {
                        s.to = d.pos;
                        pos = d.pos;
                        held > 10.0
                    }
                    // The mirror may not show the drag yet.
                    _ => held > 0.5,
                },
                Aim::Surface(target) => {
                    if !s.dashed {
                        s.dashed = true;
                        if !world.player(me).is_some_and(|p| p.downed) {
                            let from = scene
                                .try_object(PLAYER)
                                .map_or(hand, |o| V3::from_array(o.position()));
                            let way = target - from;
                            let secs = (way.length() / GRAPPLE_SPEED).min(2.0);
                            scene.dash(PLAYER, (way.normalize() * GRAPPLE_SPEED).to_array(), secs);
                        }
                        false
                    } else if held > 0.1 && pc.dash_left <= 0.0 || held > 2.5 {
                        if let Some(o) = scene.try_object(PLAYER) {
                            let gap = V3::from_array(o.position()).dist(target);
                            self.grapple_gap = Some((gap * 100.0) as i32);
                        }
                        true
                    } else {
                        false
                    }
                }
                Aim::Enemy | Aim::Miss => true,
            };
            if done {
                s.back = Some(s.age);
            }
        }
        if let Some(back) = s.back {
            let t = (s.age - back) / BOLT_RETURN;
            pos = s.to.lerp(hand, t.min(1.0));
            if t >= 1.0 {
                self.shot = None;
                show(scene, &mut self.shown, false, "", BOLT);
                show(scene, &mut self.shown, false, "", ROPE);
                return;
            }
        }
        let away = pos + (pos - hand).normalize();
        if let Some(mut o) = scene.try_object(BOLT) {
            o.set_position(pos.to_array()).look_at(away.to_array());
        }
        stretch(scene, ROPE, hand, pos, 1.0);
    }

    /// Rod: click casts, the bar runs after a bite, a click reels in (in the zone =
    /// caught); a click before the bite cancels. Leaving slot 6 cancels.
    #[allow(clippy::too_many_arguments)]
    fn fishing(
        &mut self,
        me: &Player,
        now: f64,
        dt: f32,
        rod: bool,
        click: bool,
        origin: V3,
        dir: V3,
        feet: V3,
        out: &mut Vec<Request>,
    ) {
        self.fish_pause = (self.fish_pause - dt).max(0.0);
        self.hud.fishing = FishingUi::default();
        let Some(f) = &me.fishing else {
            self.reel = None;
            self.cancel_sent = false;
            if rod && click && self.fish_pause <= 0.0 {
                let mut at = water_point(origin, dir).unwrap_or(feet + facing(me.yaw) * 10.0);
                let off = (at - feet).with_y(0.0);
                if off.length_xz() > CAST_RANGE - 0.5 {
                    at = feet + off.normalize() * (CAST_RANGE - 0.5);
                }
                out.push(Request::CastLine { at: at.with_y(0.0) });
                self.casts += 1;
                self.fish_pause = FISH_CLICK_PAUSE;
            }
            return;
        };
        if !rod {
            if !self.cancel_sent {
                self.cancel_sent = true;
                self.reel = None;
                out.push(Request::CancelFishing);
            }
            return;
        }
        if let Some(fish) = f.hooked {
            if self.reel.is_none() && self.reel_bite != f.bite_at {
                self.reel_bite = f.bite_at;
                let roll = ((now * 7.31).fract()) as f32;
                self.reel = Some(Reel::new(fish, roll));
            }
        }
        if let Some(r) = &mut self.reel {
            r.step(dt);
        }
        self.hud.fishing = match &self.reel {
            Some(r) => FishingUi {
                reeling: true,
                marker: r.marker,
                zone: r.zone,
            },
            None => FishingUi::default(),
        };
        if click && self.fish_pause <= 0.0 {
            self.fish_pause = FISH_CLICK_PAUSE;
            out.push(match (self.reel.take(), f.hooked) {
                (Some(r), _) => Request::ReelIn { success: r.hit() },
                (None, Some(_)) => Request::ReelIn { success: false },
                (None, None) => Request::CancelFishing,
            });
        }
    }

    fn show_held(&mut self, scene: &mut GameScene, me: &Player, first: bool, hide: bool) {
        let tier = |t: u8, names: &[&'static str]| names[(t.max(1) as usize - 1).min(names.len() - 1)];
        let want = if hide {
            None
        } else {
            match self.hud.selected_slot {
                1 => Some("tpl.held.harpoon"),
                2 => Some("tpl.held.sac"),
                3 => Some(tier(
                    me.gear.melee,
                    &["tpl.held.club", "tpl.held.machete", "tpl.held.cutlass", "tpl.held.abyss_blade"],
                )),
                4 => me.food.as_ref().map(|s| match s.kind {
                    ItemKind::Fish(_) | ItemKind::CookedFish(_) => "tpl.held.fish",
                    _ => "tpl.held.food",
                }),
                5 => me.heal.as_ref().map(|s| match s.kind {
                    ItemKind::Bandage => "tpl.held.bandage",
                    _ => "tpl.held.salve",
                }),
                6 => Some(tier(me.gear.rod, &["tpl.held.rod_twig", "tpl.held.rod_reed", "tpl.held.rod_carbon"])),
                _ => None,
            }
        };
        let key = want.map(|t| (t, first));
        if key != self.held {
            self.held = key;
            scene.despawn(HELD);
            if let Some((template, first)) = key {
                if spawn(scene, template, HELD, V3::ZERO) {
                    scene.reparent(HELD, Some(if first { CAMERA } else { AVATAR }));
                }
            }
        }
        let Some((template, first)) = self.held else {
            return;
        };
        // Grip in the right hand: below and right of the eye, or the avatar's hand.
        let at = if first { [0.3, -0.28, -0.45] } else { [0.45, 0.75, -0.3] };
        let tilt = if template.contains("rod") { 0.6 } else { 0.0 };
        let swing = if self.swing_left > 0.0 {
            -1.4 * (PI * (1.0 - self.swing_left / MELEE_SWING)).sin()
        } else {
            0.0
        };
        let size = if first { 1.0 } else { HELD_SCALE };
        if let Some(mut o) = scene.try_object(HELD) {
            o.set_position(at).set_rotation([tilt + swing, 0.0, 0.0]).set_scale([size; 3]);
        }
    }

    fn show_ghost(&mut self, scene: &mut GameScene) {
        let want = self.hud.ghost.is_some() && self.build.is_some();
        if want && !self.shown.contains(GHOST) {
            self.ghost_valid = None;
        }
        show(scene, &mut self.shown, want, "tpl.ghost", GHOST);
        let Some(g) = self.hud.ghost.as_ref().filter(|_| want) else {
            return;
        };
        let (centre, [sx, sz], yaw) = build::shape(g);
        let (y, sy) = if g.side.is_some() { (DECK_Y + 0.5, 5.0) } else { (DECK_Y + 0.1, 1.0) };
        if let Some(mut o) = scene.try_object(GHOST) {
            o.set_position([centre.x, y, centre.z])
                .set_rotation([0.0, yaw, 0.0])
                .set_scale([sx, sy, sz]);
        }
        if self.ghost_valid != Some(g.valid) {
            self.ghost_valid = Some(g.valid);
            let (color, glow) = if g.valid {
                ([0.3, 1.0, 0.4, 0.35], [0.1, 0.5, 0.15])
            } else {
                ([1.0, 0.2, 0.2, 0.35], [0.5, 0.05, 0.05])
            };
            scene.set_color(GHOST_BOX, color);
            scene.set_emissive(GHOST_BOX, glow);
        }
    }

    /// Bobber on the waves and the line from the rod tip.
    fn show_line(&mut self, scene: &mut GameScene, me: &Player, now: f64, hand: V3) {
        let at = me.fishing.as_ref().map(|f| f.at);
        show(scene, &mut self.shown, at.is_some(), "tpl.bobber", BOBBER);
        show(scene, &mut self.shown, at.is_some(), "tpl.rope", LINE);
        let Some(at) = at else {
            return;
        };
        let bob = at.with_y(wave(at.x, at.z, now));
        if let Some(mut o) = scene.try_object(BOBBER) {
            o.set_position(bob.to_array());
        }
        let fwd = facing(me.yaw);
        let tip = hand + fwd * 1.4 + V3::new(0.0, 0.9, 0.0);
        stretch(scene, LINE, tip, bob, 0.3);
    }

    /// Avatar facing and swim pose. It faces the aim while a tool is in use or after the
    /// camera turned on the spot, else the walk direction; in the water it lies forward to
    /// stroke or stands up to tread.
    #[allow(clippy::too_many_arguments)]
    fn pose_avatar(
        &mut self,
        scene: &mut GameScene,
        pc: &PlayerController,
        base: &PlayerController,
        face_aim: bool,
        swimming: bool,
        now: f64,
        dt: f32,
    ) {
        let [vx, _, vz] = pc.velocity;
        let moving = vx.hypot(vz) >= 0.1;
        let want = if face_aim {
            Some(0.0)
        } else if moving {
            // Model forward is -Z, as for yaw.
            Some((-vx).atan2(-vz) - pc.yaw)
        } else {
            None
        };
        if let Some(want) = want {
            let turn = if base.turn_speed > 0.0 { base.turn_speed } else { 10.0 };
            let step = turn * dt;
            let off = (want - self.avatar_yaw + PI).rem_euclid(2.0 * PI) - PI;
            self.avatar_yaw += off.clamp(-step, step);
        }
        // The climb-out dash is not a stroke.
        self.swim.step(swimming, vx.hypot(vz) > swim::STROKE_SPEED && pc.dash_left <= 0.0, dt);
        let (tilt, back, up) = self.swim.offsets((now % 1000.0) as f32);
        let yaw = self.avatar_yaw;
        if let Some(mut o) = scene.try_object(AVATAR) {
            o.set_position([yaw.sin() * back, up - FEET_OFFSET, yaw.cos() * back])
                .set_rotation([-tilt, yaw, 0.0]);
        }
    }

    fn clip(&self, me: &Player, pc: &PlayerController, in_water: bool, sprint: bool) -> &'static str {
        let moving = pc.velocity[0].hypot(pc.velocity[2]) > 0.5;
        if me.downed {
            "die"
        } else if let Some(s) = me.seat {
            if s.index == 0 {
                "drive"
            } else {
                "sit"
            }
        } else if self.swing_left > 0.0 {
            "attack-melee-right"
        } else if in_water {
            // `pose_avatar` lays the body down for "swim".
            if pc.velocity[0].hypot(pc.velocity[2]) > swim::STROKE_SPEED && pc.dash_left <= 0.0 {
                "swim"
            } else {
                "tread"
            }
        } else if !pc.grounded && pc.dash_left <= 0.0 {
            if pc.vertical_speed > 0.0 {
                "jump"
            } else {
                "fall"
            }
        } else if moving {
            if sprint {
                "sprint"
            } else {
                "walk"
            }
        } else if self.held.is_some() {
            "holding-right"
        } else {
            "idle"
        }
    }
}

/// Puts the controller's feet at `pos` and stops its fall.
fn teleport(scene: &mut GameScene, pos: V3) {
    put_feet(scene, pos);
    scene.set_player(PLAYER, |pc| pc.vertical_speed = 0.0);
}

fn put_feet(scene: &mut GameScene, feet: V3) {
    if let Some(mut body) = scene.try_object(PLAYER) {
        body.set_position([feet.x, feet.y + FEET_OFFSET, feet.z]);
    }
}

/// `local` ([x right, y up, z back]) in the body frame turned by `yaw`.
fn frame_of(yaw: f32, [x, y, z]: [f32; 3]) -> V3 {
    let (s, c) = yaw.sin_cos();
    V3::new(x * c + z * s, y, -x * s + z * c)
}

/// Third-person camera offset and distance, pulled in so neither the shoulder nor the
/// camera sits inside geometry (the engine sweeps only from the orbit centre, without
/// the offset). Set every frame, the first one included.
fn third_person(
    scene: &GameScene,
    pc: &PlayerController,
    base: &PlayerController,
    body: V3,
) -> ([f32; 3], f32) {
    let centre = body + V3::new(0.0, base.camera_height, 0.0);
    let side = frame_of(pc.yaw, SHOULDER);
    let reach = side.length();
    let k = ray(scene, centre, side.normalize(), reach + CAMERA_GAP)
        .map_or(1.0, |(p, _)| ((p.dist(centre) - CAMERA_GAP) / reach).clamp(0.0, 1.0));
    let pivot = centre + side * k;
    let (sp, cp) = pc.pitch.sin_cos();
    let back = V3::new(pc.yaw.sin() * cp, -sp, pc.yaw.cos() * cp);
    let distance = ray(scene, pivot, back, ORBIT + CAMERA_GAP)
        .map_or(ORBIT, |(p, _)| (p.dist(pivot) - CAMERA_GAP).max(0.2));
    (SHOULDER.map(|v| v * k), distance)
}

/// The nearest scene surface on the ray (point, normal), past templates, our own
/// objects and moving things.
fn ray(scene: &GameScene, origin: V3, dir: V3, max: f32) -> Option<(V3, V3)> {
    let mut from = origin;
    let mut left = max;
    for _ in 0..8 {
        let hit = scene.raycast(from.to_array(), dir.to_array(), left)?;
        let point = V3::from_array(hit.point);
        if !SKIP.iter().any(|p| hit.name.starts_with(p)) {
            return Some((point, V3::from_array(hit.normal)));
        }
        left -= hit.distance + 0.01;
        from = point + dir * 0.01;
        if left <= 0.0 {
            return None;
        }
    }
    None
}

/// Distance along the ray to the sphere, if the ray passes through it.
fn ray_sphere(origin: V3, dir: V3, centre: V3, r: f32) -> Option<f32> {
    let t = (centre - origin).dot(dir);
    (t > 0.0 && (origin + dir * t).dist(centre) <= r).then_some(t)
}

/// Where the ray meets the sea (y 0) in front of the origin.
fn water_point(origin: V3, dir: V3) -> Option<V3> {
    let t = -origin.y / dir.y;
    (dir.y < -0.01 && t > 0.0).then(|| origin + dir * t)
}

/// Where the ray meets the deck plane in front of the origin, at most PLACE_RANGE
/// from the feet.
fn deck_point(origin: V3, dir: V3, feet: V3) -> Option<V3> {
    let t = (DECK_Y - origin.y) / dir.y;
    if dir.y > -0.02 || t <= 0.0 {
        return None;
    }
    let p = origin + dir * t;
    let off = (p - feet).with_y(0.0);
    Some(if off.length_xz() > PLACE_RANGE {
        feet.with_y(DECK_Y) + off.normalize() * PLACE_RANGE
    } else {
        p
    })
}

/// A visible copy of `template` at the root named `name`, replacing an old one.
fn spawn(scene: &mut GameScene, template: &str, name: &str, at: V3) -> bool {
    scene.despawn(name);
    if scene
        .spawn_copy_at_root(template, name, Transform::new(at.to_array()))
        .is_none()
    {
        return false;
    }
    scene.set_visible(name, true);
    true
}

/// Spawns or despawns `name` to match `want`, once per change (a missing template
/// is not retried every frame).
fn show(
    scene: &mut GameScene,
    shown: &mut HashSet<&'static str>,
    want: bool,
    template: &str,
    name: &'static str,
) {
    if want && shown.insert(name) {
        spawn(scene, template, name, V3::new(0.0, -500.0, 0.0));
    } else if !want && shown.remove(name) {
        scene.despawn(name);
    }
}

/// Lays the 1 m rope template from `a` to `b`.
fn stretch(scene: &mut GameScene, name: &str, a: V3, b: V3, thick: f32) {
    let len = a.dist(b);
    if let Some(mut o) = scene.try_object(name) {
        o.set_position(((a + b) * 0.5).to_array())
            .look_at(b.to_array())
            .set_scale([thick, thick, len.max(0.01)]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rays_meet_water_and_deck() {
        let down = V3::new(0.0, -1.0, -1.0).normalize();
        let w = water_point(V3::new(0.0, 2.0, 0.0), down).unwrap();
        assert!((w.z + 2.0).abs() < 1e-4 && w.y.abs() < 1e-4);
        // An eye under the plane (first person, swimming) has nothing in front.
        assert!(water_point(V3::new(0.0, -0.5, 0.0), down).is_none());
        assert!(deck_point(V3::new(0.0, -0.5, 0.0), down, V3::ZERO).is_none());
        // Clamped to PLACE_RANGE from the feet.
        let flat = V3::new(0.0, -0.01, -1.0).normalize();
        let far = V3::new(0.0, -0.05, -1.0).normalize();
        assert!(deck_point(V3::new(0.0, 2.0, 0.0), flat, V3::ZERO).is_none());
        let p = deck_point(V3::new(0.0, 2.0, 0.0), far, V3::ZERO).unwrap();
        assert!((p.z + PLACE_RANGE).abs() < 1e-3 && (p.y - DECK_Y).abs() < 1e-4);
        assert!(ray_sphere(V3::ZERO, V3::new(0.0, 0.0, -1.0), V3::new(0.5, 0.0, -5.0), 1.0)
            .is_some_and(|t| (t - 5.0).abs() < 1e-4));
        assert!(ray_sphere(V3::ZERO, V3::new(0.0, 0.0, -1.0), V3::new(0.0, 0.0, 5.0), 1.0)
            .is_none());
        // yaw +90 deg: body forward is -X, right is -Z.
        let r = frame_of(PI / 2.0, [1.0, 0.0, -1.0]);
        assert!((r.x + 1.0).abs() < 1e-4 && (r.z + 1.0).abs() < 1e-4);
    }
}
