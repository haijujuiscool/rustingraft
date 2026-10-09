//! Day and night from `World::clock`: the sun (or moon) on the scene's directional
//! light, sky and ambient light, distance fog, background and exposure. The scene's
//! own environment components are driven when they exist; missing ones are added on
//! `view.env`.

use raft_sim::Clock;
use rusting_engine::bevy_ecs::query::Without;
use rusting_engine::prelude::{Component, Entity, GameScene, Transform, World as BevyWorld};
use rusting_engine::runtime::{
    AmbientLight, DirectionalLight, Fog, PostVolume, RenderSettings, SceneBackground,
    ShadowQuality, SkyLight, ToneMapper, ToneMapping,
};
use std::f32::consts::{FRAC_PI_2, PI};

const ENV: &str = "view.env";
const SUN: &str = "view.sun";
const DAY: f32 = 300.0;
const CYCLE: f32 = 360.0;

#[derive(Default)]
pub struct Sky {
    sun: Option<Entity>,
    /// `view.env` / `view.sun` were spawned here.
    spawned: bool,
}

/// The look at one moment of the cycle; the frame blends the two around it.
#[derive(Clone, Copy)]
struct Look {
    t: f32,
    sun: [f32; 3],
    lux: f32,
    sky: [f32; 3],
    sky_i: f32,
    ground: [f32; 3],
    ambient: [f32; 3],
    fog: [f32; 3],
    density: f32,
    scatter: f32,
    zenith: [f32; 3],
    exposure: f32,
}

#[rustfmt::skip]
const LOOKS: [Look; 8] = [
    // Dawn: pink sun on the horizon.
    Look { t: 0.0, sun: [1.0, 0.55, 0.45], lux: 15e3, sky: [0.5, 0.45, 0.7], sky_i: 0.3,
        ground: [0.12, 0.12, 0.16], ambient: [0.6, 0.55, 0.75], fog: [0.62, 0.45, 0.52],
        density: 0.0032, scatter: 0.7, zenith: [0.25, 0.25, 0.5], exposure: 1.3 },
    Look { t: 15.0, sun: [1.0, 0.8, 0.65], lux: 70e3, sky: [0.55, 0.62, 0.9], sky_i: 0.38,
        ground: [0.22, 0.26, 0.3], ambient: [0.75, 0.8, 0.95], fog: [0.75, 0.68, 0.72],
        density: 0.003, scatter: 0.5, zenith: [0.3, 0.45, 0.75], exposure: 1.1 },
    // Day: the scene's own look (warm key, cool sky fill).
    Look { t: 40.0, sun: [1.0, 0.94, 0.84], lux: 100e3, sky: [0.55, 0.7, 1.0], sky_i: 0.4,
        ground: [0.32, 0.31, 0.27], ambient: [0.85, 0.9, 1.0], fog: [0.6, 0.74, 0.9],
        density: 0.0026, scatter: 0.35, zenith: [0.36, 0.58, 0.86], exposure: 1.0 },
    Look { t: 260.0, sun: [1.0, 0.94, 0.84], lux: 100e3, sky: [0.55, 0.7, 1.0], sky_i: 0.4,
        ground: [0.32, 0.31, 0.27], ambient: [0.85, 0.9, 1.0], fog: [0.6, 0.74, 0.9],
        density: 0.0026, scatter: 0.35, zenith: [0.36, 0.58, 0.86], exposure: 1.0 },
    // Golden hour, then dusk.
    Look { t: 285.0, sun: [1.0, 0.58, 0.3], lux: 60e3, sky: [0.9, 0.6, 0.45], sky_i: 0.35,
        ground: [0.25, 0.2, 0.18], ambient: [0.85, 0.68, 0.6], fog: [0.95, 0.6, 0.4],
        density: 0.003, scatter: 0.9, zenith: [0.4, 0.35, 0.6], exposure: 1.15 },
    Look { t: 300.0, sun: [0.9, 0.4, 0.3], lux: 15e3, sky: [0.32, 0.28, 0.5], sky_i: 0.3,
        ground: [0.08, 0.07, 0.1], ambient: [0.45, 0.42, 0.62], fog: [0.28, 0.2, 0.34],
        density: 0.003, scatter: 0.6, zenith: [0.07, 0.07, 0.2], exposure: 1.3 },
    // Night: dark blue moonlight, bright enough to read the deck.
    Look { t: 310.0, sun: [0.55, 0.68, 1.0], lux: 10e3, sky: [0.1, 0.16, 0.35], sky_i: 0.3,
        ground: [0.03, 0.04, 0.07], ambient: [0.3, 0.38, 0.65], fog: [0.016, 0.028, 0.06],
        density: 0.0032, scatter: 0.3, zenith: [0.004, 0.008, 0.022], exposure: 1.6 },
    Look { t: 350.0, sun: [0.55, 0.68, 1.0], lux: 10e3, sky: [0.1, 0.16, 0.35], sky_i: 0.3,
        ground: [0.03, 0.04, 0.07], ambient: [0.3, 0.38, 0.65], fog: [0.016, 0.028, 0.06],
        density: 0.0032, scatter: 0.3, zenith: [0.004, 0.008, 0.022], exposure: 1.6 },
];

impl Sky {
    pub fn update(&mut self, scene: &mut GameScene, clock: &Clock) {
        let t = clock.time.rem_euclid(CYCLE);
        let look = look_at(t);
        // The sun crosses east (+X) to west by the south over the day, the moon by night.
        let (elev, azim) = if t < DAY {
            let u = t / DAY;
            ((PI * u).sin() * 62f32.to_radians(), FRAC_PI_2 - PI * u + 0.4)
        } else {
            let v = (t - DAY) / (CYCLE - DAY);
            ((PI * v).sin() * 50f32.to_radians(), FRAC_PI_2 - PI * v - 0.3)
        };
        let up = smoothstep(0.0, 10f32.to_radians(), elev);
        let sun = self.sun(scene);
        let world = scene.world();
        if let Some(mut tr) = world.get_mut::<Transform>(sun) {
            tr.rotation = [-elev.max(0.05), azim, 0.0];
        }
        if let Some(mut light) = world.get_mut::<DirectionalLight>(sun) {
            light.color = look.sun;
            light.illuminance = look.lux * up;
            light.shadows = true;
        }
        if !self.spawned {
            self.spawned = true;
            ensure(scene);
            // Crisp, stair-stepped shadow edges suit the low-poly look.
            scene.set_hard_shadows(true);
        }
        let world = scene.world();
        for mut a in world.query::<&mut AmbientLight>().iter_mut(world) {
            a.color = look.ambient;
            a.intensity = 0.08;
        }
        for mut s in world.query::<&mut SkyLight>().iter_mut(world) {
            s.sky_color = look.sky;
            s.ground_color = look.ground;
            s.intensity = look.sky_i;
        }
        for mut f in world
            .query_filtered::<&mut Fog, Without<PostVolume>>()
            .iter_mut(world)
        {
            *f = Fog {
                color: look.fog,
                density: look.density,
                height: 0.0,
                height_falloff: 0.012,
                sun_scatter: look.scatter,
                sky_affect: 0.7,
            };
        }
        for mut m in world.query::<&mut ToneMapping>().iter_mut(world) {
            m.mapper = ToneMapper::Aces;
            m.exposure = look.exposure;
        }
        let [r, g, b] = look.zenith;
        for mut bg in world.query::<&mut SceneBackground>().iter_mut(world) {
            bg.color = [r, g, b, 1.0];
        }
        if let Some(mut s) = world.get_resource_mut::<RenderSettings>() {
            if s.shadows != ShadowQuality::Low {
                s.shadows = ShadowQuality::Low;
            }
            s.background_color = [r, g, b, 1.0];
        }
    }

    /// The scene's `Sun`, else its first directional light, else one of ours.
    fn sun(&mut self, scene: &mut GameScene) -> Entity {
        if let Some(e) = self.sun {
            if scene.world().get::<DirectionalLight>(e).is_some() {
                return e;
            }
        }
        let named = scene.try_object("Sun").map(|o| o.entity());
        let world = scene.world();
        let found = named
            .filter(|&e| world.get::<DirectionalLight>(e).is_some())
            .or_else(|| {
                world
                    .query::<(Entity, &DirectionalLight)>()
                    .iter(world)
                    .map(|(e, _)| e)
                    .next()
            });
        let e = found.unwrap_or_else(|| {
            let e = super::bare(scene, SUN);
            scene.world().entity_mut(e).insert(DirectionalLight::default());
            e
        });
        self.sun = Some(e);
        e
    }

    pub fn clear(&mut self, scene: &mut GameScene) {
        if self.spawned {
            scene.despawn(ENV);
            scene.despawn(SUN);
        }
        *self = Sky::default();
    }
}

/// Adds the environment components the scene lacks to `view.env`.
fn ensure(scene: &mut GameScene) {
    let e = super::bare(scene, ENV);
    let world = scene.world();
    if !has::<AmbientLight>(world) {
        world.entity_mut(e).insert(AmbientLight::default());
    }
    if !has::<SkyLight>(world) {
        world.entity_mut(e).insert(SkyLight::default());
    }
    let fog = world
        .query_filtered::<&Fog, Without<PostVolume>>()
        .iter(world)
        .next()
        .is_some();
    if !fog {
        world.entity_mut(e).insert(Fog::default());
    }
    if !has::<ToneMapping>(world) {
        world.entity_mut(e).insert(ToneMapping::default());
    }
    if !has::<SceneBackground>(world) {
        world.entity_mut(e).insert(SceneBackground::default());
    }
}

fn has<C: Component>(world: &mut BevyWorld) -> bool {
    world.query::<&C>().iter(world).next().is_some()
}

fn look_at(t: f32) -> Look {
    let i = LOOKS.iter().rposition(|l| l.t <= t).unwrap_or(0);
    let a = LOOKS[i];
    let (b, end) = match LOOKS.get(i + 1) {
        Some(b) => (*b, b.t),
        None => (LOOKS[0], CYCLE),
    };
    let k = ((t - a.t) / (end - a.t).max(1e-3)).clamp(0.0, 1.0);
    let mix = |x: [f32; 3], y: [f32; 3]| [0, 1, 2].map(|j| x[j] + (y[j] - x[j]) * k);
    let lerp = |x: f32, y: f32| x + (y - x) * k;
    Look {
        t,
        sun: mix(a.sun, b.sun),
        lux: lerp(a.lux, b.lux),
        sky: mix(a.sky, b.sky),
        sky_i: lerp(a.sky_i, b.sky_i),
        ground: mix(a.ground, b.ground),
        ambient: mix(a.ambient, b.ambient),
        fog: mix(a.fog, b.fog),
        density: lerp(a.density, b.density),
        scatter: lerp(a.scatter, b.scatter),
        zenith: mix(a.zenith, b.zenith),
        exposure: lerp(a.exposure, b.exposure),
    }
}

fn smoothstep(lo: f32, hi: f32, x: f32) -> f32 {
    let k = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    k * k * (3.0 - 2.0 * k)
}

#[cfg(test)]
mod tests {
    #[test]
    fn looks_blend_and_wrap() {
        let noon = super::look_at(150.0);
        assert_eq!(noon.lux, 100e3);
        let late = super::look_at(355.0);
        // Halfway from the night look back to dawn.
        assert!((late.lux - 12.5e3).abs() < 1.0, "{}", late.lux);
    }
}
