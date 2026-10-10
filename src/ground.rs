//! `raft_sim::Ground` from the scene's island colliders (class "ground").

use raft_sim::content::ISLANDS;
use rusting_engine::prelude::GameScene;

/// Grid spacing (m).
const STEP: f32 = 2.0;
/// Rays start this high and look down.
const TOP: f32 = 200.0;
/// Grids reach this far past an island's radius: walkable colliders do too (Hook Isle's
/// dock, wide domes).
const MARGIN: f32 = 10.0;

/// One height grid per island, sampled when made: ~21k rays, about 11 ms in release.
pub struct SceneGround {
    islands: Vec<Grid>,
}

struct Grid {
    min_x: f32,
    min_z: f32,
    cols: usize,
    /// Row by row.
    heights: Vec<Option<f32>>,
}

impl SceneGround {
    pub fn new(scene: &GameScene) -> SceneGround {
        let islands = ISLANDS
            .iter()
            .map(|island| {
                let r = island.radius + MARGIN;
                let cols = (2.0 * r / STEP).ceil() as usize + 1;
                let (min_x, min_z) = (island.x - r, island.z - r);
                let heights = (0..cols * cols)
                    .map(|i| {
                        let x = min_x + (i % cols) as f32 * STEP;
                        let z = min_z + (i / cols) as f32 * STEP;
                        // Highest "ground" surface under (x, z).
                        scene
                            .raycast_only([x, TOP, z], [0.0, -1.0, 0.0], TOP + 50.0, &["ground"])
                            .map(|hit| hit.point[1])
                    })
                    .collect();
                Grid {
                    min_x,
                    min_z,
                    cols,
                    heights,
                }
            })
            .collect();
        SceneGround { islands }
    }
}

impl raft_sim::Ground for SceneGround {
    fn height_at(&self, x: f32, z: f32) -> Option<f32> {
        self.islands.iter().find_map(|g| {
            let col = ((x - g.min_x) / STEP).round();
            let row = ((z - g.min_z) / STEP).round();
            if col < 0.0 || row < 0.0 || col as usize >= g.cols || row as usize >= g.cols {
                return None;
            }
            g.heights.get(row as usize * g.cols + col as usize).copied().flatten()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Grid, SceneGround};
    use raft_sim::Ground;

    #[test]
    fn nearest_sample_inside_the_grid_only() {
        let grid = Grid {
            min_x: 10.0,
            min_z: -4.0,
            cols: 2,
            heights: vec![Some(1.0), Some(2.0), None, Some(4.0)],
        };
        let ground = SceneGround {
            islands: vec![grid],
        };
        assert_eq!(ground.height_at(10.4, -4.0), Some(1.0));
        assert_eq!(ground.height_at(11.6, -3.5), Some(2.0));
        assert_eq!(ground.height_at(10.0, -2.0), None);
        assert_eq!(ground.height_at(12.0, -2.0), Some(4.0));
        assert_eq!(ground.height_at(20.0, 0.0), None);
    }
}
