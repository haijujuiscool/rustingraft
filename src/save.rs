//! World and settings files in the engine's user data folder.

use crate::shared::Settings;
use rusting_engine::prelude::GameScene;
use rusting_engine::serde_json;

pub const WORLD_KEY: &str = "world.json";
pub const SETTINGS_KEY: &str = "settings.json";

/// The saved settings; a missing or malformed token is replaced and saved.
pub fn load_settings(scene: &GameScene) -> Settings {
    let mut settings: Settings = scene
        .load_data(SETTINGS_KEY)
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    if settings.token.len() != 32 || !settings.token.bytes().all(|b| b.is_ascii_hexdigit()) {
        settings.token = raft_net::new_token();
        save_settings(scene, &settings);
    }
    settings
}

pub fn save_settings(scene: &GameScene, settings: &Settings) {
    let text = serde_json::to_string_pretty(settings).expect("settings serialize");
    if let Err(err) = scene.save_data(SETTINGS_KEY, &text) {
        eprintln!("settings not saved: {err}");
    }
}

pub fn has_world(scene: &GameScene) -> bool {
    scene.load_data(WORLD_KEY).is_some()
}

/// The saved world and its raw text (for `Session::restore_owners`), or why there is none.
pub fn load_world(scene: &GameScene) -> Result<(raft_sim::World, String), String> {
    let text = scene.load_data(WORLD_KEY).ok_or("no saved world")?;
    Ok((raft_sim::World::load(&text)?, text))
}

pub fn save_world(scene: &GameScene, json: &str) -> Result<(), String> {
    scene
        .save_data(WORLD_KEY, json)
        .map_err(|err| err.to_string())
}
