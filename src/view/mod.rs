//! Scene objects that mirror the `World`. Reads `Frame` and `LocalHud`; never sends
//! requests. Copies come from hidden `tpl.*` templates (docs/scene-names.md); a missing
//! template is skipped, so the scene can grow while the game runs.

use crate::shared::{Frame, LocalHud};
use raft_sim::raft::{cell_center, Cell, Raft, Railing, Side};
use raft_sim::{EntityId, V3};
use rusting_engine::prelude::{GameScene, Transform};
use std::collections::BTreeSet;
use std::f32::consts::FRAC_PI_2;

#[derive(Default)]
pub struct View {
    tiles: BTreeSet<Cell>,
    structures: BTreeSet<EntityId>,
    railings: BTreeSet<Railing>,
}

impl View {
    pub fn sync(&mut self, scene: &mut GameScene, frame: &Frame, _hud: &LocalHud) {
        let raft = &frame.world.raft;
        diff(
            scene,
            &mut self.tiles,
            &raft.tiles,
            tile_name,
            |scene, cell| {
                spawn(
                    scene,
                    "tpl.raft_floor",
                    &tile_name(cell),
                    cell_center(cell),
                    0.0,
                );
            },
        );
        let ids: BTreeSet<EntityId> = raft.structures.keys().copied().collect();
        diff(
            scene,
            &mut self.structures,
            &ids,
            structure_name,
            |scene, id| {
                let s = &raft.structures[&id];
                let tpl = format!("tpl.{}", snake(&format!("{:?}", s.kind)));
                spawn(
                    scene,
                    &tpl,
                    &structure_name(id),
                    s.center(),
                    s.rot as f32 * FRAC_PI_2,
                );
            },
        );
        diff(
            scene,
            &mut self.railings,
            &raft.railings,
            railing_name,
            |scene, r| {
                // The template runs along X, as on a North or South edge.
                let yaw = if matches!(r.side, Side::East | Side::West) {
                    FRAC_PI_2
                } else {
                    0.0
                };
                spawn(
                    scene,
                    "tpl.railing",
                    &railing_name(r),
                    Raft::edge_pos(r.cell, r.side),
                    yaw,
                );
            },
        );
        // TODO(view): ocean mesh following the camera (spec 4), debris, enemies, boats,
        // other players' avatars (tpl.avatar), trader boat, held tool, harpoon bolt,
        // bobber and rope, placement ghost from `_hud.ghost`, day/night lights and sky,
        // fx from `frame.events` (splash, campfire, hit), sounds from `Event::Sound`.
    }

    /// Despawns everything this view spawned (leaving a session).
    pub fn clear(&mut self, scene: &mut GameScene) {
        diff(
            scene,
            &mut self.tiles,
            &BTreeSet::new(),
            tile_name,
            |_, _| {},
        );
        diff(
            scene,
            &mut self.structures,
            &BTreeSet::new(),
            structure_name,
            |_, _| {},
        );
        diff(
            scene,
            &mut self.railings,
            &BTreeSet::new(),
            railing_name,
            |_, _| {},
        );
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
/// `yaw` on top of the template's own rotation. Missing templates spawn nothing.
fn spawn(scene: &mut GameScene, template: &str, name: &str, pos: V3, yaw: f32) {
    let Some(tpl) = scene.try_object(template) else {
        // Fails too, but warns once with the nearest names.
        let _ = scene.spawn_copy_at_root(template, name, Transform::default());
        return;
    };
    let [rx, ry, rz] = tpl.rotation();
    let scale = tpl.scale();
    let transform = Transform {
        position: [pos.x, pos.y, pos.z],
        rotation: [rx, ry + yaw, rz],
        scale,
    };
    if scene
        .spawn_copy_at_root(template, name, transform)
        .is_some()
    {
        scene.set_visible(name, true);
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

#[cfg(test)]
mod tests {
    #[test]
    fn snake_case_names() {
        assert_eq!(super::snake("WoodFarm"), "wood_farm");
        assert_eq!(super::snake("Campfire"), "campfire");
    }
}
