//! Sounds and one-shot particle bursts from the frame's events, plus the ocean and
//! wind ambience. `Event::Sound` plays at its world position (the listener is the
//! active camera); the app applies the volume setting to the master bus.

use super::ocean::{hash, unit};
use crate::shared::Frame;
use raft_sim::{Event, SoundKind, V3};
use rusting_engine::prelude::{GameScene, Sound, SoundId};
use rusting_engine::runtime::{
    ColorKey, EmitterShape, ParticleBlend, ParticleBurst, ParticleEmitter, ParticleSprite,
};
use std::collections::VecDeque;
use std::f32::consts::PI;

/// Bursts alive at once; the oldest goes first.
const MAX_BURSTS: usize = 24;

#[derive(Default)]
pub struct Fx {
    ambience: Vec<SoundId>,
    wind: Option<SoundId>,
    night: Option<bool>,
    bursts: VecDeque<(String, f32)>,
    count: u32,
    clock: f32,
    next_creak: f32,
}

#[derive(Clone, Copy)]
enum Burst {
    Splash,
    Hit,
    Pickup,
    Kill,
    LevelUp,
}

impl Fx {
    pub fn update(&mut self, scene: &mut GameScene, frame: &Frame, dt: f32) {
        let w = frame.world;
        self.clock += dt;
        if self.ambience.is_empty() {
            let ocean = loop_sound(scene, "sfx/ocean_loop.wav", 0.075);
            let wind = loop_sound(scene, "sfx/wind_loop.wav", 0.1);
            let lap = loop_sound(scene, "sfx/water_lap_loop.ogg", 0.06);
            self.ambience = vec![ocean, wind, lap];
            self.wind = Some(wind);
            self.next_creak = self.clock + 6.0;
        }
        let night = w.clock.is_night();
        if self.night != Some(night) {
            self.night = Some(night);
            if let Some(id) = self.wind {
                scene.set_sound_volume(id, if night { 0.22 } else { 0.1 }, 2.0);
            }
        }
        // The raft creaks now and then.
        if self.clock >= self.next_creak {
            self.count += 1;
            let h = hash(self.count as i32, 41);
            self.next_creak = self.clock + 9.0 + 6.0 * unit(h);
            let at = w.raft.center() + V3::new(3.0 * unit(h >> 4), 0.3, 3.0 * unit(h >> 12));
            self.play(scene, "sfx/creak.ogg", 0.3, Some(at));
        }

        for e in frame.events {
            match e {
                Event::Sound { kind, pos } => {
                    let (clips, volume, positional) = clips(*kind);
                    self.count += 1;
                    let clip = clips[self.count as usize % clips.len()];
                    self.play(scene, &format!("sfx/{clip}"), volume, positional.then_some(*pos));
                    let burst = match kind {
                        SoundKind::Splash | SoundKind::FishCatch | SoundKind::FishEscape => Some(Burst::Splash),
                        SoundKind::Pickup => Some(Burst::Pickup),
                        _ => None,
                    };
                    if let Some(b) = burst {
                        self.burst(scene, b, *pos);
                    }
                }
                Event::Damage { pos, .. } => self.burst(scene, Burst::Hit, *pos + V3::new(0.0, 1.0, 0.0)),
                Event::EnemyKilled { pos, .. } => self.burst(scene, Burst::Kill, *pos + V3::new(0.0, 0.5, 0.0)),
                Event::CampfireLevelUp(_) => {
                    if let Some(at) = w.campfire_pos() {
                        self.burst(scene, Burst::LevelUp, at + V3::new(0.0, 0.5, 0.0));
                    }
                }
                Event::NightStarted => self.play(scene, "sfx/night_sting.ogg", 0.6, None),
                _ => {}
            }
        }

        while self.bursts.front().is_some_and(|(_, end)| *end <= self.clock) {
            if let Some((name, _)) = self.bursts.pop_front() {
                scene.despawn(&name);
            }
        }
    }

    pub fn clear(&mut self, scene: &mut GameScene) {
        for id in self.ambience.drain(..) {
            scene.stop_sound(id);
        }
        for (name, _) in self.bursts.drain(..) {
            scene.despawn(&name);
        }
        *self = Fx::default();
    }

    fn play(&mut self, scene: &mut GameScene, clip: &str, volume: f32, at: Option<V3>) {
        let rate = 1.0 + 0.06 * unit(hash(self.count as i32, 9));
        let sound = Sound {
            volume,
            rate,
            position: at.map(V3::to_array),
            ..Sound::default()
        };
        scene.play_sound_with(clip, sound);
    }

    fn burst(&mut self, scene: &mut GameScene, kind: Burst, at: V3) {
        if self.bursts.len() >= MAX_BURSTS {
            if let Some((name, _)) = self.bursts.pop_front() {
                scene.despawn(&name);
            }
        }
        self.count += 1;
        let name = format!("fx.{}", self.count);
        let e = emitter(kind);
        let end = self.clock + e.lifetime[1] + 0.3;
        super::emitter(scene, &name, e);
        if let Some(mut o) = scene.try_object(&name) {
            o.set_position(at.to_array());
        }
        self.bursts.push_back((name, end));
    }
}

fn loop_sound(scene: &mut GameScene, clip: &str, volume: f32) -> SoundId {
    let sound = Sound {
        volume,
        looped: true,
        ..Sound::default()
    };
    scene.play_sound_with(clip, sound)
}

/// Clips cycled per kind, their volume, and whether they play where they happen.
fn clips(kind: SoundKind) -> (&'static [&'static str], f32, bool) {
    use SoundKind as S;
    match kind {
        S::HarpoonHit => (&["harpoon_hit_1.ogg", "harpoon_hit_2.ogg"], 0.8, true),
        S::Splash => (&["splash_1.ogg", "splash_2.ogg", "splash_3.ogg", "splash_4.ogg"], 0.7, true),
        S::Pickup => (&["pickup_1.ogg", "pickup_2.ogg"], 0.7, true),
        S::Shred => (&["shredder_1.ogg", "shredder_2.ogg", "shredder_3.ogg"], 0.6, true),
        S::CampfireFeed => (&["fire_crackle.ogg"], 0.8, true),
        S::CampfireLevelUp => (&["quest_complete.ogg"], 0.8, true),
        S::CampfireOut => (&["whoosh.wav"], 0.5, true),
        S::Craft => (&["craft_1.ogg", "craft_2.ogg", "craft_3.ogg"], 0.7, true),
        S::Place => (&["build_place_1.ogg", "build_place_2.ogg"], 0.8, true),
        S::Coins => (&["coin_1.ogg", "coin_2.ogg"], 0.7, true),
        S::Eat => (&["pickup_2.ogg"], 0.6, true),
        S::Heal => (&["equip.ogg"], 0.6, true),
        S::Hurt => (&["player_hurt_1.ogg", "player_hurt_2.ogg"], 0.8, true),
        S::Downed => (&["player_hurt_2.ogg"], 1.0, true),
        S::Revive => (&["ui_confirm.ogg"], 0.8, true),
        S::FishBite => (&["fishing_bite.wav", "fishing_bite_2.ogg"], 0.8, true),
        S::FishCatch => (&["fishing_catch.wav"], 0.8, true),
        S::FishEscape => (&["splash_2.ogg"], 0.6, true),
        S::EnemyHit => (&["enemy_hurt_1.ogg", "enemy_hurt_2.ogg", "enemy_hurt_3.ogg", "enemy_hurt_4.ogg"], 0.8, true),
        S::EnemyDie => (&["enemy_death_1.ogg", "enemy_death_2.ogg"], 0.9, true),
        S::Bite => (&["melee_hit_1.ogg", "melee_hit_2.ogg", "melee_hit_3.ogg"], 0.8, true),
        S::ChestOpen => (&["chest_open.ogg"], 0.8, true),
        S::Shrine => (&["bell.ogg"], 0.8, true),
        S::Generator => (&["build_metal.ogg"], 0.8, true),
        S::TraderHorn => (&["bell.ogg"], 1.0, true),
        S::Radio => (&["radio_static.ogg"], 0.7, true),
        S::Save => (&["logbook_close.ogg"], 0.7, true),
        S::Flare => (&["whoosh.wav"], 0.8, true),
        S::BossRoar => (&["leviathan_rumble.ogg"], 1.0, true),
        S::Denied => (&["ui_error.ogg"], 0.4, false),
    }
}

fn emitter(kind: Burst) -> ParticleEmitter {
    let fade = |c: [f32; 3], a: f32| {
        vec![
            ColorKey { t: 0.0, color: [c[0], c[1], c[2], a] },
            ColorKey { t: 1.0, color: [c[0], c[1], c[2], 0.0] },
        ]
    };
    let base = ParticleEmitter {
        rate: 0.0,
        duration: 0.1,
        looping: false,
        fade_in: 0.0,
        fade_out: 0.4,
        shape: EmitterShape::Sphere,
        shape_size: [0.2; 3],
        ..ParticleEmitter::default()
    };
    let (count, e) = match kind {
        Burst::Splash => (24, ParticleEmitter {
            shape: EmitterShape::Circle,
            shape_size: [0.4; 3],
            lifetime: [0.5, 0.9],
            speed: [2.0, 4.0],
            spread: 0.6,
            gravity: 9.0,
            size: [0.08, 0.16],
            color_over_life: fade([0.85, 0.93, 1.0], 0.9),
            sprite: ParticleSprite::Disc,
            ..base
        }),
        Burst::Hit => (14, ParticleEmitter {
            lifetime: [0.2, 0.4],
            speed: [2.0, 5.0],
            spread: PI,
            gravity: 4.0,
            size: [0.05, 0.1],
            color_over_life: fade([1.0, 0.55, 0.15], 1.0),
            emissive: 2.5,
            blend: ParticleBlend::Additive,
            sprite: ParticleSprite::Disc,
            ..base
        }),
        Burst::Pickup => (12, ParticleEmitter {
            lifetime: [0.5, 0.8],
            speed: [0.5, 1.5],
            spread: 0.8,
            gravity: -1.0,
            size: [0.06, 0.12],
            color_over_life: fade([1.0, 0.85, 0.3], 1.0),
            emissive: 2.0,
            blend: ParticleBlend::Additive,
            ..base
        }),
        Burst::Kill => (40, ParticleEmitter {
            lifetime: [0.6, 1.2],
            speed: [2.0, 5.0],
            spread: PI,
            gravity: 3.0,
            drag: 1.0,
            size: [0.1, 0.25],
            color_over_life: fade([0.55, 0.12, 0.1], 0.9),
            ..base
        }),
        Burst::LevelUp => (60, ParticleEmitter {
            shape: EmitterShape::Circle,
            shape_size: [0.5; 3],
            lifetime: [1.0, 2.0],
            speed: [2.0, 4.0],
            spread: 0.5,
            gravity: -0.5,
            drag: 0.5,
            size: [0.05, 0.1],
            color_over_life: fade([1.0, 0.55, 0.15], 1.0),
            emissive: 3.0,
            blend: ParticleBlend::Additive,
            sprite: ParticleSprite::Disc,
            ..base
        }),
    };
    ParticleEmitter {
        bursts: vec![ParticleBurst { time: 0.0, count }],
        max_particles: count,
        ..e
    }
}
