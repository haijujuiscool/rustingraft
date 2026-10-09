//! `raft_sim::Ground` from the scene's island colliders (class "ground").

use raft_sim::content::ISLANDS;
use rusting_engine::prelude::GameScene;

/// Grid spacing (m).
const STEP: f32 = 2.0;
/// Rays start this high and look down.
const TOP: f32 = 200.0;
/// Non-ground colliders (trees, crates) a ray may pass before giving up.
const MAX_PASSES: usize = 4;

/// One height grid per island, sampled once after the scene loads.
pub struct SceneGround {
    islands: Vec<Grid>,
}

struct Grid {
    min_x: f32,
    min_z: f32,
    cols: usize,
    heights: Vec<Option<f32>>,
}

impl SceneGround {
    // ponytail: ~15k rays once at start (each scans every collider); sample per
    // island on first approach if startup gets slow.
    pub fn build(scene: &mut GameScene) -> SceneGround {
        let ground: std::collections::HashSet<String> =
            scene.in_class("ground").into_iter().collect();
        let islands = ISLANDS
            .iter()
            .map(|island| {
                let cols = (2.0 * island.radius / STEP).ceil() as usize + 1;
                let min_x = island.x - island.radius;
                let min_z = island.z - island.radius;
                let mut heights = Vec::with_capacity(cols * cols);
                for row in 0..cols {
                    for col in 0..cols {
                        let x = min_x + col as f32 * STEP;
                        let z = min_z + row as f32 * STEP;
                        heights.push(sample(scene, &ground, x, z));
                    }
                }
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

/// Highest "ground" surface under (x, z).
fn sample(
    scene: &GameScene,
    ground: &std::collections::HashSet<String>,
    x: f32,
    z: f32,
) -> Option<f32> {
    if ground.is_empty() {
        return None;
    }
    let mut y = TOP;
    for _ in 0..MAX_PASSES {
        let hit = scene.raycast([x, y, z], [0.0, -1.0, 0.0], y + 50.0)?;
        if ground.contains(&hit.name) {
            return Some(hit.point[1]);
        }
        y = hit.point[1] - 0.01;
    }
    None
}

impl raft_sim::Ground for SceneGround {
    fn height_at(&self, x: f32, z: f32) -> Option<f32> {
        self.islands.iter().find_map(|g| {
            let col = ((x - g.min_x) / STEP).round();
            let row = ((z - g.min_z) / STEP).round();
            if col < 0.0 || row < 0.0 || col as usize >= g.cols || row as usize >= g.cols {
                return None;
            }
            g.heights[row as usize * g.cols + col as usize]
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
