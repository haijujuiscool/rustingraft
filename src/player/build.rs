//! Build mode: where the ghost of a placeable recipe goes and whether the sim would
//! accept it (the same checks as `economy::place` / `place_railing`).

use crate::shared::Ghost;
use raft_sim::content::{island_at, RecipeId, RecipeOutput, PLACE_RANGE, TILE};
use raft_sim::raft::{cell_at, cell_center, Raft, Structure};
use raft_sim::{Request, World, V3};

/// Recipes placed on the raft instead of crafted.
pub fn placeable(recipe: RecipeId) -> bool {
    matches!(
        recipe.def().output,
        RecipeOutput::Floor | RecipeOutput::Railing | RecipeOutput::Structure(_)
    )
}

/// The ghost for `recipe` aimed at deck point `aim`, the player's feet at `feet`.
pub fn ghost(world: &World, recipe: RecipeId, rot: u8, aim: V3, feet: V3) -> Ghost {
    let raft = &world.raft;
    let def = recipe.def();
    let mut side = None;
    let (cell, fits, target) = match def.output {
        RecipeOutput::Floor => {
            let cell = cell_at(aim.x, aim.z);
            let c = cell_center(cell);
            (cell, raft.can_add_tile(cell).is_ok() && island_at(c.x, c.z).is_none(), c)
        }
        RecipeOutput::Structure(kind) => {
            let (w, d) = kind.footprint();
            let (w, d) = if rot % 2 == 1 { (d, w) } else { (w, d) };
            // The footprint centred on the aim point: its min corner cell.
            let cell = cell_at(
                aim.x - (w - 1) as f32 * TILE * 0.5,
                aim.z - (d - 1) as f32 * TILE * 0.5,
            );
            let one_radio =
                kind != raft_sim::content::PlaceableKind::RadioStation || raft.count(kind) == 0;
            let fits = raft.can_place(kind, cell, rot).is_ok() && one_radio;
            (cell, fits, Structure::new(kind, cell, rot).center())
        }
        _ => {
            // Railing: the outer edge nearest the aim point.
            let edge = raft.outer_edges().into_iter().min_by(|a, b| {
                let da = Raft::edge_pos(a.0, a.1).dist_xz(aim);
                let db = Raft::edge_pos(b.0, b.1).dist_xz(aim);
                da.total_cmp(&db)
            });
            match edge {
                Some((cell, s)) => {
                    side = Some(s);
                    let fits = raft.can_add_railing(cell, s).is_ok();
                    (cell, fits, Raft::edge_pos(cell, s))
                }
                None => (cell_at(aim.x, aim.z), false, aim),
            }
        }
    };
    let affordable = world.economy.unlocked(recipe) && world.economy.stock.covers(&def.cost);
    let valid = fits && affordable && feet.dist_xz(target) <= PLACE_RANGE;
    Ghost {
        recipe,
        cell,
        rot,
        side,
        valid,
    }
}

pub fn request(g: &Ghost) -> Request {
    match g.side {
        Some(side) => Request::PlaceRailing { cell: g.cell, side },
        None => Request::Place {
            recipe: g.recipe,
            cell: g.cell,
            rot: g.rot,
        },
    }
}

/// Ghost box placement: (centre on the deck, size in tiles along x and z, yaw).
pub fn shape(g: &Ghost) -> (V3, [f32; 2], f32) {
    use raft_sim::raft::Side;
    match (g.recipe.def().output, g.side) {
        (RecipeOutput::Structure(kind), _) => {
            let (w, d) = kind.footprint();
            let size = if g.rot % 2 == 1 { [d as f32, w as f32] } else { [w as f32, d as f32] };
            (Structure::new(kind, g.cell, g.rot).center(), size, 0.0)
        }
        (_, Some(side)) => {
            let yaw = match side {
                Side::East | Side::West => std::f32::consts::FRAC_PI_2,
                _ => 0.0,
            };
            (Raft::edge_pos(g.cell, side), [1.0, 0.1], yaw)
        }
        _ => (cell_center(g.cell), [1.0, 1.0], 0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raft_sim::content::PlaceableKind;
    use raft_sim::raft::Side;

    #[test]
    fn cells_and_validity() {
        let mut w = World::new(1);
        let feet = V3::new(3.0, 0.45, 1.0);
        // Next to the start raft (cells -2..1), but no wood yet.
        let g = ghost(&w, RecipeId::RaftFloor, 0, V3::xz(5.0, 1.0), feet);
        assert_eq!((g.cell, g.valid), ((2, 0), false));
        w.economy.stock.wood = 10;
        assert!(ghost(&w, RecipeId::RaftFloor, 0, V3::xz(5.0, 1.0), feet).valid);
        // Too far from the player, and not touching the raft.
        assert!(!ghost(&w, RecipeId::RaftFloor, 0, V3::xz(5.0, 1.0), V3::xz(20.0, 1.0)).valid);
        assert!(!ghost(&w, RecipeId::RaftFloor, 0, V3::xz(9.0, 1.0), feet).valid);
        assert!(matches!(
            request(&ghost(&w, RecipeId::RaftFloor, 0, V3::xz(5.0, 1.0), feet)),
            Request::Place { cell: (2, 0), rot: 0, .. }
        ));

        // A 2x1 farm centred on the aim point; a quarter turn makes it 1x2.
        let farm = PlaceableKind::WoodFarm;
        let aim = V3::xz(1.8, -1.5);
        let at = |rot| ghost(&w, RecipeId::WoodFarm, rot, aim, feet).cell;
        assert_eq!(at(0), (0, -1));
        assert_eq!(at(1), (0, -2));
        assert_eq!(Structure::new(farm, at(0), 0).center().x, 2.0);

        // Railing on the nearest outer edge.
        let g = ghost(&w, RecipeId::Railing, 0, V3::xz(4.3, 1.0), feet);
        assert_eq!((g.cell, g.side), ((1, 0), Some(Side::East)));
        assert!(matches!(request(&g), Request::PlaceRailing { cell: (1, 0), side: Side::East }));
    }
}
