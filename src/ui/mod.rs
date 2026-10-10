//! egui menus, HUD and windows. Reads `UiState`, returns `UiAction`s; never moves scene
//! objects (that is `view` and `player`).
//!
//! - `menu`: main menu, connecting, pause, settings, leave/quit confirmation, game over.
//! - `hud`: vitals, hotbar, compass, clock, prompts, toasts, fishing bar, build hints,
//!   FPS counter (also over menus).
//! - `windows`: sac, workbench, campfire, logbook, Barnaby, trader, radio.
//!
//! Test hooks (counters): set `ui_open` to 1..=6 to open a `Window` (Workbench,
//! Campfire, Logbook, Fisherman, Trader, Radio in that order), 7 for the sac, 8 for
//! the pause menu. `ui_window` reads what is open: 0 nothing, 1..=6 a `Window`, 7 sac,
//! 8 pause menu, 9 settings, 10 leave/quit confirmation. `fps_shown` is 1 while the FPS
//! counter is on. `ui_force_game_over` 1 (in
//! app.rs) shows the game over screen. tests/fixtures/ui_world.json fills every window;
//! `windows::tests::write_ui_world` writes it.

mod hud;
mod menu;
mod style;
mod windows;

use crate::shared::{AppPhase, Settings, UiAction, UiState, Window};
use rusting_engine::prelude::GameScene;
use std::time::Instant;

const TOAST_SECONDS: f32 = 4.0;
const MAX_TOASTS: usize = 5;

/// A game window over the HUD.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Open {
    Sac,
    Win(Window),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Confirm {
    Leave,
    Quit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToastKind {
    Info,
    Denied,
    Gained,
}

struct Toast {
    text: String,
    kind: ToastKind,
    left: f32,
    /// Identical toasts in a row merge into one "x3".
    count: u32,
}

/// Display names of the scene's bindings, read once.
#[derive(Default)]
struct Keys {
    interact: String,
    sac: String,
    build: String,
    rotate: String,
    camera: String,
    pause: String,
    use_tool: String,
    aim: String,
}

#[derive(Default)]
pub struct Ui {
    /// Main-menu name field, seeded from the settings.
    name: Option<String>,
    code: String,
    host_load: bool,
    toasts: Vec<Toast>,
    open: Option<Open>,
    paused: bool,
    /// The settings being edited while the Settings window is open.
    editing: Option<Settings>,
    confirm: Option<Confirm>,
    /// Sac slot picked in the sac window.
    picked: Option<usize>,
    /// This Ui paused the scene (solo pause menu).
    we_paused: bool,
    /// Scale the egui style was last built for.
    scale: f32,
    phase: Option<AppPhase>,
    keys: Option<Keys>,
    fps: hud::Fps,
    last_frame: Option<Instant>,
}

impl Ui {
    pub fn draw(&mut self, scene: &mut GameScene, state: &UiState) -> Vec<UiAction> {
        if self.keys.is_none() {
            self.keys = Some(read_keys(scene));
        }
        let ctx = scene.ui();
        let s = style::scale(&ctx);
        if s != self.scale {
            style::apply(&ctx, s);
            self.scale = s;
        }
        if self.phase != Some(state.phase) {
            self.phase = Some(state.phase);
            self.open = None;
            self.paused = false;
            self.editing = None;
            self.confirm = None;
            self.picked = None;
            self.toasts.clear();
        }
        let mut out = Vec::new();
        let frame = state.frame.filter(|_| state.phase == AppPhase::Playing);
        if let Some(frame) = frame {
            self.input(scene, state);
            let solo = frame.is_host && frame.room_code.is_none() && frame.peers == 0;
            let pause = self.paused && solo;
            if pause != scene.paused() && (pause || self.we_paused) {
                scene.set_paused(pause);
                self.we_paused = pause;
            }
        } else if self.we_paused {
            scene.set_paused(false);
            self.we_paused = false;
        }

        match state.phase {
            AppPhase::MainMenu => self.main_menu(&ctx, state, &mut out),
            AppPhase::Connecting => self.connecting(&ctx, state, &mut out),
            AppPhase::Playing => {
                if let Some(frame) = state.frame {
                    self.hud(&ctx, scene, frame, state.hud);
                    self.windows(&ctx, frame, &mut out);
                    self.pause_menu(&ctx, frame, state.settings, &mut out);
                }
            }
            AppPhase::GameOver => self.game_over(&ctx, state, &mut out),
        }
        self.settings_window(&ctx, state, &mut out);
        // Wall time: the engine's `real_delta` is clamped to one step while it is overloaded.
        let now = Instant::now();
        let real_dt = self.last_frame.map_or(0.0, |t| (now - t).as_secs_f32());
        self.last_frame = Some(now);
        let fps = self.fps.tick(real_dt);
        if state.settings.show_fps {
            hud::fps_counter(&ctx, s, fps);
        }
        scene.set_counter("fps_shown", state.settings.show_fps as i32);
        scene.set_counter("ui_window", self.window_code());
        out
    }

    /// True while a window owns the pointer: the player ignores gameplay input.
    pub fn modal(&self) -> bool {
        self.open.is_some() || self.paused || self.editing.is_some() || self.confirm.is_some()
    }

    /// Esc, Tab, `LocalHud.open` and the `ui_open` test hook.
    fn input(&mut self, scene: &mut GameScene, state: &UiState) {
        let hook = scene.counter_or("ui_open", 0);
        if hook != 0 {
            scene.set_counter("ui_open", 0);
            match hook {
                1..=6 => self.show(Open::Win(WINDOWS[hook as usize - 1])),
                7 => self.show(Open::Sac),
                8 => self.paused = true,
                _ => {}
            }
        }
        if scene.pressed("pause") {
            if self.confirm.is_some() {
                self.confirm = None;
            } else if self.editing.is_some() {
                self.editing = None;
            } else if self.open.is_some() {
                self.open = None;
            } else if state.hud.ghost.is_none() {
                // In placement mode Esc belongs to the player (cancel placement).
                self.paused = !self.paused;
            }
        } else if scene.pressed("sac") && !self.paused && self.editing.is_none() {
            if self.open == Some(Open::Sac) {
                self.open = None;
            } else {
                self.show(Open::Sac);
            }
        }
        if let Some(window) = state.hud.open {
            if !self.paused {
                self.show(Open::Win(window));
            }
        }
    }

    fn show(&mut self, open: Open) {
        self.open = Some(open);
        self.picked = None;
    }

    fn window_code(&self) -> i32 {
        if self.confirm.is_some() {
            10
        } else if self.editing.is_some() {
            9
        } else if self.paused {
            8
        } else {
            match self.open {
                None => 0,
                Some(Open::Sac) => 7,
                Some(Open::Win(w)) => WINDOWS.iter().position(|x| *x == w).map_or(0, |i| i as i32 + 1),
            }
        }
    }
}

/// `ui_open` / `ui_window` numbering.
const WINDOWS: [Window; 6] = [
    Window::Workbench,
    Window::Campfire,
    Window::Logbook,
    Window::Fisherman,
    Window::Trader,
    Window::Radio,
];

fn read_keys(scene: &mut GameScene) -> Keys {
    // Tab and Esc are ui's own keys: bind them when the scene does not.
    for (action, key) in [("sac", "Tab"), ("pause", "Escape")] {
        if scene.binding(action).is_empty() {
            let _ = scene.rebind(action, &[key]);
        }
    }
    let mut key = |action: &str, fallback: &str| {
        scene
            .binding(action)
            .first()
            .map_or_else(|| fallback.to_string(), |k| key_label(k))
    };
    Keys {
        interact: key("interact", "E"),
        sac: key("sac", "Tab"),
        build: key("build", "B"),
        rotate: key("rotate", "R"),
        camera: key("camera", "V"),
        pause: key("pause", "Esc"),
        use_tool: key("use", "LMB"),
        aim: key("aim", "RMB"),
    }
}

/// `KeyE` -> `E`, `MouseLeft` -> `LMB`, `Escape` -> `Esc`.
fn key_label(input: &str) -> String {
    match input {
        "MouseLeft" => "LMB".into(),
        "MouseRight" => "RMB".into(),
        "MouseMiddle" => "MMB".into(),
        "Escape" => "Esc".into(),
        "ShiftLeft" | "ShiftRight" => "Shift".into(),
        "ControlLeft" | "ControlRight" => "Ctrl".into(),
        other => other
            .strip_prefix("Key")
            .or_else(|| other.strip_prefix("Digit"))
            .unwrap_or(other)
            .to_string(),
    }
}

/// Menu text for a session's `Status::Closed` reason.
fn notice_text(reason: &str) -> &str {
    match reason {
        "name taken" => "Someone else already plays that name in this world. Pick another name.",
        "world full" => "This world has no room for new sailors. Rejoin with a name it knows.",
        "bad token" => "The host refused this install's identity token.",
        "Version mismatch" => "The host runs a different version of the game.",
        other if other.starts_with("relay connection failed") => {
            "Could not reach the relay. Check the relay address in Settings and that the relay is running."
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_labels_are_short() {
        assert_eq!(key_label("KeyE"), "E");
        assert_eq!(key_label("Digit1"), "1");
        assert_eq!(key_label("MouseLeft"), "LMB");
        assert_eq!(key_label("Escape"), "Esc");
        assert_eq!(key_label("Tab"), "Tab");
    }

    #[test]
    fn notices_explain_known_reasons() {
        assert!(notice_text("name taken").contains("name"));
        assert_eq!(notice_text("Relay unreachable"), "Relay unreachable");
        assert!(notice_text("relay connection failed: os error 10061").starts_with("Could not reach"));
    }
}
