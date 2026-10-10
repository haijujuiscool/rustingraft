//! The faceted sea: a 120 m grid of flat-shaded, jittered triangles around the camera
//! that follows `raft_sim::debris::wave` (so floating things ride it), inside a
//! low-poly ring out to the fog. The grid moves in 12 m steps, and its vertices are
//! hashed by world position, so the facets stay put while the camera moves. The ring's
//! inner band swells gently (rebuilt per frame); its swell fades to nothing at both the
//! near grid's flat rim and the still outer ring, so the meshes meet without a seam.
//! Around each island the water turns turquoise over its sunken sand: every vertex's
//! colour is a deep-to-shallow shade picked by world position.

use raft_sim::content::ISLANDS;
use raft_sim::debris::wave;
use rusting_engine::assets::MeshVertex;
use rusting_engine::prelude::{AssetServer, CubeSpawn, GameScene, MaterialAsset, Transform};
use rusting_engine::runtime::MeshRenderer;
use rusting_engine::{Handle, MeshAsset};
use std::f64::consts::TAU;

const NEAR: &str = "view.ocean";
const FAR: &str = "view.ocean_far";
const BAND_NAME: &str = "view.ocean_band";
/// Near grid: cells per side and cell size; it snaps by `SNAP` (4 cells).
const CELLS: i32 = 40;
const STEP: f32 = 3.0;
const SNAP: f32 = 12.0;
/// Far ring: 12 m cells out to `RING` cells, around a hole the near grid fills.
const RING: i32 = 32;
const HOLE: i32 = 5;
/// Ring cells inside this many from the centre swell; the rest stay still.
const BAND: i32 = 16;
/// Flat skirt beyond the ring, past the camera's far plane.
const SKIRT: f32 = 3000.0;
/// Open sea and island shallows (linear).
const DEEP: [f32; 3] = [0.008, 0.065, 0.1];
const SHALLOW: [f32; 3] = [0.05, 0.42, 0.4];

#[derive(Default)]
pub struct Ocean {
    /// Near grid, still ring, swelling band.
    meshes: Option<[Handle<MeshAsset>; 3]>,
    center: Option<(i32, i32)>,
    grid: Vec<[f32; 3]>,
}

impl Ocean {
    pub fn update(&mut self, scene: &mut GameScene, cam: [f32; 3], t: f64) {
        let [near, far, band] = *self.meshes.get_or_insert_with(|| {
            let material = scene.create_material(MaterialAsset {
                name: "view.ocean".into(),
                base_color: [1.0; 4],
                roughness: 0.35,
                metallic: 0.0,
                ..MaterialAsset::default()
            });
            [NEAR, FAR, BAND_NAME].map(|name| sea(scene, name, material))
        });
        let c = ((cam[0] / SNAP).round() as i32, (cam[2] / SNAP).round() as i32);
        let (ox, oz) = (c.0 as f32 * SNAP, c.1 as f32 * SNAP);
        if self.center != Some(c) {
            self.center = Some(c);
            for name in [NEAR, FAR, BAND_NAME] {
                if let Some(mut o) = scene.try_object(name) {
                    o.set_position([ox, 0.0, oz]);
                }
            }
            if let Some(mesh) = scene.world().resource_mut::<AssetServer>().meshes.get_mut(far) {
                ring(mesh, c, None);
            }
        }
        if let Some(mesh) = scene.world().resource_mut::<AssetServer>().meshes.get_mut(band) {
            ring(mesh, c, Some(t));
        }

        // Heights: the sim's wave plus a little per-vertex chop, fading out to the rim.
        let half = CELLS / 2;
        let chop = ((t * 1.4) % TAU) as f32;
        let rim = half as f32 * STEP;
        self.grid.clear();
        for b in 0..=CELLS {
            for a in 0..=CELLS {
                let h = hash(c.0 * 4 - half + a, c.1 * 4 - half + b);
                let inner = a > 0 && b > 0 && a < CELLS && b < CELLS;
                let j = if inner { 0.9 } else { 0.0 };
                let x = (a - half) as f32 * STEP + j * unit(h);
                let z = (b - half) as f32 * STEP + j * unit(h >> 16);
                let fade = ((rim - x.abs().max(z.abs())) / 18.0).clamp(0.0, 1.0);
                let y = wave(ox + x, oz + z, t) + 0.07 * (chop + 3.0 * unit(h >> 8)).sin();
                self.grid.push([x, y * fade, z]);
            }
        }
        let mut assets = scene.world().resource_mut::<AssetServer>();
        let Some(mesh) = assets.meshes.get_mut(near) else {
            return;
        };
        mesh.vertices.clear();
        let n = CELLS + 1;
        for b in 0..CELLS {
            for a in 0..CELLS {
                let p = |da: i32, db: i32| self.grid[((b + db) * n + a + da) as usize];
                let flip = hash(c.0 * 4 + a, c.1 * 4 + b + 977) & 1 == 1;
                quad(&mut mesh.vertices, [ox, oz], [p(0, 0), p(1, 0), p(0, 1), p(1, 1)], flip);
            }
        }
        if mesh.indices.len() != mesh.vertices.len() {
            mesh.indices = (0..mesh.vertices.len() as u32).collect();
        }
    }

    pub fn clear(&mut self, scene: &mut GameScene) {
        if let Some(meshes) = self.meshes.take() {
            for name in [NEAR, FAR, BAND_NAME] {
                scene.despawn(name);
            }
            let mut assets = scene.world().resource_mut::<AssetServer>();
            for h in meshes {
                if let Some(m) = assets.meshes.get_mut(h) {
                    *m = MeshAsset::default();
                }
            }
        }
        *self = Ocean::default();
    }
}

/// A shadowless object drawing its own, still empty, mesh.
fn sea(scene: &mut GameScene, name: &str, material: Handle<MaterialAsset>) -> Handle<MeshAsset> {
    let mesh = scene
        .world()
        .resource_mut::<AssetServer>()
        .meshes
        .insert(MeshAsset::default());
    let e = scene.spawn_cube(name, Transform::default(), &CubeSpawn::new());
    if let Some(mut r) = scene.world().get_mut::<MeshRenderer>(e) {
        r.mesh = mesh;
        r.material = material;
        r.cast_shadows = false;
    }
    mesh
}

/// The far ring around the near grid's hole: with `t`, the swelling band cells at that
/// time; without, the still cells beyond it and the flat skirt.
fn ring(mesh: &mut MeshAsset, c: (i32, i32), t: Option<f64>) {
    let o = [c.0 as f32 * SNAP, c.1 as f32 * SNAP];
    let phase = |rate: f64| t.map_or(0.0, |t| ((t * rate) % TAU) as f32);
    let (slow, roll) = (phase(0.9), phase(0.8));
    let p = |i: i32, j: i32| {
        let r = i.abs().max(j.abs());
        let k = if r <= HOLE || r == RING { 0.0 } else { 1.0 };
        let (wi, wj) = (c.0 + i, c.1 + j);
        let h = hash(wi + 7919, wj);
        let (x, z) = (i as f32 * SNAP + 3.5 * k * unit(h), j as f32 * SNAP + 3.5 * k * unit(h >> 16));
        let mut y = 0.3 * k * unit(h >> 8);
        if t.is_some() {
            // Zero on the hole's edge (the near grid's flat rim) and on the band's edge.
            let amp = ((r - HOLE) as f32 / 2.0).min((BAND - r) as f32 / 4.0).clamp(0.0, 1.0);
            let along = 0.09 * (wi as f32 + 0.6 * wj as f32) * SNAP;
            y += amp * (0.22 * (slow + 3.0 * unit(h >> 4)).sin() + 0.18 * (along + roll).sin());
        }
        [x, y, z]
    };
    let v = &mut mesh.vertices;
    v.clear();
    for j in -RING..RING {
        for i in -RING..RING {
            let hole = (-HOLE..HOLE).contains(&i) && (-HOLE..HOLE).contains(&j);
            let band = (-BAND..BAND).contains(&i) && (-BAND..BAND).contains(&j);
            if hole || band != t.is_some() {
                continue;
            }
            let flip = hash(c.0 + i, c.1 + j + 31) & 1 == 1;
            quad(v, o, [p(i, j), p(i + 1, j), p(i, j + 1), p(i + 1, j + 1)], flip);
        }
    }
    if t.is_none() {
        let (a, s) = (RING as f32 * SNAP, SKIRT);
        for [x0, z0, x1, z1] in [[-s, -s, s, -a], [-s, a, s, s], [-s, -a, -a, a], [a, -a, s, a]] {
            let q = [[x0, 0.0, z0], [x1, 0.0, z0], [x0, 0.0, z1], [x1, 0.0, z1]];
            quad(v, o, q, false);
        }
    }
    if mesh.indices.len() != mesh.vertices.len() {
        mesh.indices = (0..mesh.vertices.len() as u32).collect();
    }
}

/// Two flat-shaded triangles over corners `[x0z0, x1z0, x0z1, x1z1]` (relative to the
/// world point `o`), split along either diagonal; both wind to face +Y.
fn quad(v: &mut Vec<MeshVertex>, o: [f32; 2], [p00, p10, p01, p11]: [[f32; 3]; 4], flip: bool) {
    if flip {
        tri(v, o, p00, p01, p11);
        tri(v, o, p00, p11, p10);
    } else {
        tri(v, o, p00, p01, p10);
        tri(v, o, p10, p01, p11);
    }
}

fn tri(v: &mut Vec<MeshVertex>, o: [f32; 2], a: [f32; 3], b: [f32; 3], c: [f32; 3]) {
    let (e1, e2) = ([0, 1, 2].map(|i| b[i] - a[i]), [0, 1, 2].map(|i| c[i] - a[i]));
    let n = [
        e1[1] * e2[2] - e1[2] * e2[1],
        e1[2] * e2[0] - e1[0] * e2[2],
        e1[0] * e2[1] - e1[1] * e2[0],
    ];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
    let normal = n.map(|x| x / len);
    for position in [a, b, c] {
        v.push(MeshVertex {
            position,
            normal,
            uv: [0.0; 2],
            tangent: [1.0, 0.0, 0.0, 1.0],
            color: shade(shallow(o[0] + position[0], o[1] + position[2])),
        });
    }
}

/// 1 over an island's sand (to its radius), easing to 0 at 1.4 × it.
fn shallow(x: f32, z: f32) -> f32 {
    ISLANDS.iter().fold(0.0, |most, d| {
        let (dx, dz) = (x - d.x, z - d.z);
        let reach = 1.4 * d.radius;
        if dx.abs() > reach || dz.abs() > reach {
            return most;
        }
        let k = ((reach - (dx * dx + dz * dz).sqrt()) / (0.4 * d.radius)).clamp(0.0, 1.0);
        most.max(k * k * (3.0 - 2.0 * k))
    })
}

/// The vertex colour (linear, like the material's) for shallowness `s`.
fn shade(s: f32) -> [f32; 4] {
    let [r, g, b] = [0, 1, 2].map(|k| DEEP[k] + (SHALLOW[k] - DEEP[k]) * s);
    [r, g, b, 1.0]
}

pub(super) fn hash(x: i32, z: i32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343) ^ (z as u32).wrapping_mul(0xd816_3841);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^ (h >> 15)
}

/// -1..1 from the low 16 bits.
pub(super) fn unit(h: u32) -> f32 {
    (h & 0xffff) as f32 / 32767.5 - 1.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn band_meets_still_ring_and_flat_rim() {
        let (mut still, mut band) = (MeshAsset::default(), MeshAsset::default());
        ring(&mut still, (3, -2), None);
        ring(&mut band, (3, -2), Some(1234.5));
        let key = |p: [f32; 3]| (p[0].round() as i32, p[2].round() as i32);
        let edge = BAND as f32 * SNAP;
        let still_y: HashMap<_, _> = still.vertices.iter().map(|v| (key(v.position), v.position[1])).collect();
        let mut shared = 0;
        for v in &band.vertices {
            let [x, y, z] = v.position;
            let r = x.abs().max(z.abs());
            if r <= HOLE as f32 * SNAP + 0.01 {
                assert_eq!(y, 0.0, "hole edge must stay flat");
            }
            if (r - edge).abs() < 4.0 {
                if let Some(&sy) = still_y.get(&key(v.position)) {
                    assert_eq!(y, sy);
                    shared += 1;
                }
            }
        }
        assert!(shared > 0);
        assert!(band.vertices.iter().any(|v| v.position[1].abs() > 0.1), "the band swells");
    }

    #[test]
    fn facets_face_up() {
        let mut v = Vec::new();
        let q = [[0.0, 0.0, 0.0], [1.0, 0.1, 0.0], [0.0, -0.1, 1.0], [1.0, 0.0, 1.0]];
        quad(&mut v, [0.0, 0.0], q, false);
        quad(&mut v, [0.0, 0.0], q, true);
        assert!(v.iter().all(|v| v.normal[1] > 0.9));
        assert!((-1.0..=1.0).contains(&unit(hash(-3, 7))));
    }

    #[test]
    fn shallows_ring_each_island() {
        let d = &ISLANDS[0];
        assert_eq!(shallow(d.x + 0.5 * d.radius, d.z), 1.0);
        assert!((0.0..1.0).contains(&shallow(d.x + 1.2 * d.radius, d.z)));
        assert_eq!(shallow(d.x + 1.45 * d.radius, d.z), 0.0);
        assert_eq!(shade(0.0)[..3], DEEP);
        assert!((0..3).all(|k| (shade(1.0)[k] - SHALLOW[k]).abs() < 1e-6));
        // Vertices are shaded by world position: `o` plus the local corner.
        let mut v = Vec::new();
        let q = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 1.0]];
        quad(&mut v, [d.x, d.z], q, false);
        quad(&mut v, [1.0e5, 1.0e5], q, false);
        assert!(v[..6].iter().all(|v| v.color == shade(1.0)));
        assert!(v[6..].iter().all(|v| v.color == [DEEP[0], DEEP[1], DEEP[2], 1.0]));
    }
}
