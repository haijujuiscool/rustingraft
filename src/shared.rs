//! Types passed between `app`, `view`, `ui` and `player`.
//!
//! Append-only: add fields and variants, never rename or remove one, so the three
//! client modules can grow in parallel. New `LocalHud` fields need a `Default`.

use raft_sim::content::RecipeId;
use raft_sim::raft::{Cell, Side};
use raft_sim::{Event, PlayerId, Request, World};
use serde::{Deserialize, Serialize};

/// Everything a module may read about the running game this frame.
pub struct Frame<'a> {
    /// The local mirror (the authoritative world on the host).
    pub world: &'a World,
    pub me: PlayerId,
    /// Events from the fixed ticks since the previous frame, in order.
    pub events: &'a [Event],
    /// Seconds since the previous frame.
    pub dt: f32,
    pub is_host: bool,
    /// Relay room code while hosting online.
    pub room_code: Option<&'a str>,
    /// Other connected players.
    pub peers: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppPhase {
    MainMenu,
    /// Waiting for the relay (host) or the first world snapshot (join).
    Connecting,
    Playing,
    /// Every player is downed. Host/solo picks "Load last save" or "Main menu".
    GameOver,
}

/// What `Ui::draw` gets.
pub struct UiState<'a> {
    pub phase: AppPhase,
    /// Some while a world exists (Playing, GameOver).
    pub frame: Option<&'a Frame<'a>>,
    pub hud: &'a LocalHud,
    pub settings: &'a Settings,
    /// A world save exists (enables "Load world" / "Load last save").
    pub has_save: bool,
    /// Why the last session ended, or a start error, for the main menu.
    pub notice: Option<&'a str>,
}

/// Buttons and menus: `ui` returns these, `app` carries them out.
#[allow(dead_code)] // Request, BeginPlacement and SetSettings wait for their ui windows.
#[derive(Clone, Debug, PartialEq)]
pub enum UiAction {
    /// Forward to the session (craft, eat, equip, buy, ...).
    Request(Request),
    /// Main menu: new solo world.
    StartSolo,
    /// Main menu: solo from the save.
    LoadWorld,
    Host {
        relay: String,
        name: String,
        /// Host the saved world instead of a new one.
        load: bool,
    },
    Join {
        relay: String,
        code: String,
        name: String,
    },
    /// Hand a build-menu choice to the player's placement mode.
    BeginPlacement(RecipeId),
    /// Replace and save the settings.
    SetSettings(Settings),
    /// Game over (host/solo): reload the save.
    LoadLastSave,
    /// Leave the session for the main menu.
    ToMenu,
    Quit,
}

/// Local-only state the player module publishes for `ui` (HUD) and `view` (ghost,
/// held tool, camera-dependent visuals). Written by `player` only.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LocalHud {
    /// Interaction prompt at the screen centre, such as "E  Pick up Plank".
    pub prompt: Option<String>,
    /// V toggles; false = third person.
    pub camera_first_person: bool,
    /// Holding right mouse with the harpoon or rod.
    pub aiming: bool,
    /// Hotbar slot 1..=6: harpoon, sac, melee, food, heal, fishing rod (spec 6.1).
    pub selected_slot: u8,
    pub swimming: bool,
    pub fishing: FishingUi,
    /// Placement mode ghost (B menu choice).
    pub ghost: Option<Ghost>,
    /// Set by `player` only on the frame the player asks for a window (E on its
    /// object, B near the workbench); `ui` opens it. Tab (sac) and Esc (pause) are
    /// read by `ui` itself.
    pub open: Option<Window>,
    /// Second prompt under `prompt`: "F  Put Plank in sac" or "F  Take out Bait".
    pub stash_prompt: Option<String>,
}

/// Windows `ui` shows on `LocalHud.open`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Window {
    /// Crafting list (B or E at the workbench).
    Workbench,
    /// Level, upgrade points, fuel; feed wood or oil.
    Campfire,
    /// Quests and "Save world" for its coin price.
    Logbook,
    /// Barnaby's shop: sell fish, buy upgrades, quests 1-3.
    Fisherman,
    /// The wandering trader's 5 offers.
    Trader,
    /// Survivor signals.
    Radio,
}

/// The reel-in timing bar after `Event::FishBite`; positions in 0..1.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FishingUi {
    pub reeling: bool,
    pub marker: f32,
    pub zone: [f32; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Ghost {
    pub recipe: RecipeId,
    pub cell: Cell,
    pub rot: u8,
    /// Railings only.
    pub side: Option<Side>,
    pub valid: bool,
}

/// Saved under `save::SETTINGS_KEY`. Missing fields load as their defaults.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub relay: String,
    pub name: String,
    /// Master volume 0..1.
    pub volume: f32,
    /// Multiplier on the controller's base look sensitivity.
    pub mouse_sensitivity: f32,
    pub render_scale: f32,
    /// Frame rate cap; 0 is unlimited (the engine default).
    pub max_fps: u32,
    /// The FPS counter in the bottom-right corner.
    pub show_fps: bool,
    /// This install's identity for `Session::join`; `save::load_settings` makes one.
    pub token: String,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            relay: "127.0.0.1:27777".into(),
            name: "Sailor".into(),
            volume: 1.0,
            mouse_sensitivity: 1.0,
            render_scale: 1.0,
            max_fps: 0,
            show_fps: true,
            token: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Settings;

    #[test]
    fn old_settings_load_with_fps_defaults() {
        let old = r#"{"relay": "1.2.3.4:5", "name": "Ann", "volume": 0.5, "token": "t"}"#;
        let s: Settings = rusting_engine::serde_json::from_str(old).unwrap();
        assert_eq!((s.name.as_str(), s.max_fps, s.show_fps), ("Ann", 0, true));
    }
}
