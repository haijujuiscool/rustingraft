//! The local player: controller -> `Request`s, interaction, `LocalHud`. Never draws
//! egui (that is `ui`) and never spawns world mirror objects (that is `view`).

use crate::shared::{Frame, LocalHud};
use raft_sim::content::{PlaceableKind, RecipeId, DEBRIS_PICKUP_RANGE, INTERACT_RANGE};
use raft_sim::{Event, PlayerId, Request, V3};
use rusting_engine::prelude::GameScene;

/// The scene's player controller object.
pub const PLAYER: &str = "Player";
/// Body centre above the feet: capsule half height 0.6 + radius 0.3 in the scene.
pub const FEET_OFFSET: f32 = 0.9;
/// Feet below this height and off the deck count as in the water.
const WATER_LINE: f32 = 0.2;
const SEND_INTERVAL: f32 = 1.0 / 20.0;

#[derive(Default)]
pub struct Local {
    pub hud: LocalHud,
    /// The player the controller was last placed for; a new one teleports it to spawn.
    placed: Option<PlayerId>,
    send_timer: f32,
}

impl Local {
    /// `ui_modal`: a window or menu has the pointer, so ignore gameplay input.
    pub fn update(
        &mut self,
        scene: &mut GameScene,
        frame: &Frame,
        ui_modal: bool,
        out: &mut Vec<Request>,
    ) {
        let world = frame.world;
        let Some(me) = world.player(frame.me) else {
            return;
        };
        if self.placed != Some(frame.me) {
            self.placed = Some(frame.me);
            teleport(scene, me.pos);
        }
        for event in frame.events {
            if let Event::PlayerTeleport { player, pos } = event {
                if *player == frame.me {
                    teleport(scene, *pos);
                }
            }
        }
        scene.set_mouse_look(PLAYER, !ui_modal);

        let Some(body) = scene.try_object(PLAYER).map(|o| o.position()) else {
            return;
        };
        let pc = scene.player(PLAYER);
        let feet = V3::new(body[0], body[1] - FEET_OFFSET, body[2]);
        let in_water = feet.y < WATER_LINE && !world.raft.on_deck(feet);
        self.hud.swimming = in_water;

        self.send_timer -= frame.dt;
        if self.send_timer <= 0.0 {
            self.send_timer += SEND_INTERVAL;
            self.send_timer = self.send_timer.max(0.0);
            out.push(Request::PlayerTransform {
                pos: feet,
                yaw: pc.map_or(0.0, |pc| pc.yaw),
                in_water,
                sprinting: scene.held("player.sprint"),
            });
        }

        let target = interact_target(frame, feet);
        self.hud.prompt = target.as_ref().map(|(text, _)| format!("E  {text}"));
        if !ui_modal && scene.pressed("interact") {
            if let Some((_, req)) = target {
                out.push(req);
            }
        }
        // TODO(player): swimming (controller gravity/speed in water, climbing back on),
        // camera toggle (V, `hud.camera_first_person`), hotbar slot1..slot6, harpoon
        // (use/aim, HarpoonDebris/HarpoonEnemy), melee, fishing (CastLine, FishBite
        // minigame -> ReelIn, `hud.fishing`), revive, eat/heal, placement mode with
        // `hud.ghost` (B, R, Place/PlaceRailing), boats (BoardBoat/BoatTransform),
        // island interactions (chests, shrines, Barnaby, survivors).
    }

    /// Called for `UiAction::BeginPlacement`.
    pub fn begin_placement(&mut self, recipe: RecipeId) {
        // TODO(player): enter placement mode and fill `hud.ghost`.
        let _ = recipe;
    }
}

/// The nearest thing E would act on, with its prompt text.
fn interact_target(frame: &Frame, feet: V3) -> Option<(String, Request)> {
    let world = frame.world;
    let debris = world
        .dynamic
        .debris
        .iter()
        .map(|(id, d)| {
            (
                d.pos.dist_xz(feet),
                format!("Pick up {:?}", d.kind),
                Request::PickUpDebris(*id),
            )
        })
        .filter(|(dist, ..)| *dist <= DEBRIS_PICKUP_RANGE);
    let structures = world.raft.structures.values().filter_map(|s| {
        let (text, req) = match s.kind {
            PlaceableKind::Campfire => ("Feed campfire", Request::FeedCampfire { oil: false }),
            PlaceableKind::Shredder => ("Shred debris", Request::DepositShredder),
            PlaceableKind::Logbook => ("Save world", Request::SaveWorld),
            _ => return None,
        };
        let dist = s.center().dist_xz(feet);
        (dist <= INTERACT_RANGE).then(|| (dist, text.to_string(), req))
    });
    debris
        .chain(structures)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, text, req)| (text, req))
}

/// Puts the controller's feet at `pos` and stops its fall.
fn teleport(scene: &mut GameScene, pos: V3) {
    if let Some(mut body) = scene.try_object(PLAYER) {
        body.set_position([pos.x, pos.y + FEET_OFFSET, pos.z]);
    }
    scene.set_player(PLAYER, |pc| pc.vertical_speed = 0.0);
}
