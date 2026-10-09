//! `raft_sim::Ground` from the scene's island colliders (class "ground").

use raft_sim::content::ISLANDS;
use rusting_engine::prelude::GameScene;

/// Grid spacing (m).
const STEP: f32 = 2.0;
/// Rays start this high and look down.
const TOP: f32 = 200.0;
/// Non-ground colliders (trees, crates) a ray may pass before giving up.
const MAX_PASSES: usize = 4;

/// One height grid per island. The main menu samples it a little per tick;
/// `finish` samples the rest before a session starts.
pub struct SceneGround {
    islands: Vec<Grid>,
    ground: std::collections::HashSet<String>,
}

struct Grid {
    min_x: f32,
    min_z: f32,
    cols: usize,
    /// Row by row; shorter than `cols * cols` until sampled.
    heights: Vec<Option<f32>>,
}

impl SceneGround {
    pub fn new(scene: &mut GameScene) -> SceneGround {
        let islands = ISLANDS
            .iter()
            .map(|island| {
                let cols = (2.0 * island.radius / STEP).ceil() as usize + 1;
                Grid {
                    min_x: island.x - island.radius,
                    min_z: island.z - island.radius,
                    cols,
                    heights: Vec::with_capacity(cols * cols),
                }
            })
            .collect();
        SceneGround {
            islands,
            ground: scene.in_class("ground").into_iter().collect(),
        }
    }

    /// Samples rows until `budget` has passed; ~15k rays in all, each scans
    /// every collider, which takes seconds.
    pub fn sample_for(&mut self, scene: &GameScene, budget: std::time::Duration) {
        let start = std::time::Instant::now();
        for g in &mut self.islands {
            while g.heights.len() < g.cols * g.cols {
                if start.elapsed() >= budget {
                    return;
                }
                let z = g.min_z + (g.heights.len() / g.cols) as f32 * STEP;
                for col in 0..g.cols {
                    let x = g.min_x + col as f32 * STEP;
                    g.heights.push(sample(scene, &self.ground, x, z));
                }
            }
        }
    }

    pub fn finish(&mut self, scene: &GameScene) {
        self.sample_for(scene, std::time::Duration::MAX);
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
            ground: Default::default(),
        };
        assert_eq!(ground.height_at(10.4, -4.0), Some(1.0));
        assert_eq!(ground.height_at(11.6, -3.5), Some(2.0));
        assert_eq!(ground.height_at(10.0, -2.0), None);
        assert_eq!(ground.height_at(12.0, -2.0), Some(4.0));
        assert_eq!(ground.height_at(20.0, 0.0), None);
    }
}
