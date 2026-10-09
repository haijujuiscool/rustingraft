//! egui menus, HUD and windows. Reads `UiState`, returns `UiAction`s; never moves scene
//! objects (that is `view` and `player`).

use crate::shared::{AppPhase, Frame, LocalHud, UiAction, UiState};
use raft_sim::Event;
use rusting_engine::egui::{self, Align2, Color32, RichText};
use rusting_engine::prelude::GameScene;

const TOAST_SECONDS: f32 = 4.0;

#[derive(Default)]
pub struct Ui {
    /// Menu text fields, seeded from the settings on first draw.
    relay: Option<String>,
    name: String,
    code: String,
    host_load: bool,
    toasts: Vec<(String, f32)>,
}

impl Ui {
    pub fn draw(&mut self, scene: &mut GameScene, state: &UiState) -> Vec<UiAction> {
        let ctx = scene.ui();
        let mut actions = Vec::new();
        match state.phase {
            AppPhase::MainMenu => self.main_menu(&ctx, state, &mut actions),
            AppPhase::Connecting => {
                center_window(&ctx, "Connecting", |ui| {
                    ui.label("Connecting to the relay...");
                    if ui.button("Cancel").clicked() {
                        actions.push(UiAction::ToMenu);
                    }
                });
            }
            AppPhase::Playing => {
                if let Some(frame) = state.frame {
                    self.hud(&ctx, frame, state.hud);
                }
            }
            AppPhase::GameOver => {
                let host = state.frame.is_none_or(|f| f.is_host);
                center_window(&ctx, "Game over", |ui| {
                    ui.label("Everyone is down.");
                    if !host {
                        ui.label("Waiting for the host...");
                        return;
                    }
                    if ui
                        .add_enabled(state.has_save, egui::Button::new("Load last save"))
                        .clicked()
                    {
                        actions.push(UiAction::LoadLastSave);
                    }
                    if ui.button("Main menu").clicked() {
                        actions.push(UiAction::ToMenu);
                    }
                });
            }
        }
        // TODO(ui): pause menu (Esc, warns that unsaved progress is lost), settings window
        // (UiAction::SetSettings), sac (Tab), build menu (B -> BeginPlacement), crafting,
        // logbook/save, Barnaby's and the trader's shops, quests, hotbar, compass, fishing
        // reel bar from `hud.fishing`, downed/revive overlay.
        actions
    }

    /// True while a window owns the pointer: the player ignores gameplay input.
    pub fn modal(&self) -> bool {
        false
    }

    fn main_menu(&mut self, ctx: &egui::Context, state: &UiState, actions: &mut Vec<UiAction>) {
        if self.relay.is_none() {
            self.relay = Some(state.settings.relay.clone());
            self.name = state.settings.name.clone();
        }
        let relay = self.relay.as_mut().expect("seeded above");
        center_window(ctx, "Rusting Raft", |ui| {
            if let Some(notice) = state.notice {
                ui.colored_label(Color32::LIGHT_RED, notice_text(notice));
            }
            if ui.button("New world").clicked() {
                actions.push(UiAction::StartSolo);
            }
            if ui
                .add_enabled(state.has_save, egui::Button::new("Load world"))
                .clicked()
            {
                actions.push(UiAction::LoadWorld);
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut self.name);
            });
            ui.horizontal(|ui| {
                ui.label("Relay");
                ui.text_edit_singleline(relay);
            });
            ui.add_enabled(
                state.has_save,
                egui::Checkbox::new(&mut self.host_load, "Host the saved world"),
            );
            if ui.button("Host online").clicked() {
                actions.push(UiAction::Host {
                    relay: relay.trim().to_string(),
                    name: self.name.trim().to_string(),
                    load: self.host_load && state.has_save,
                });
            }
            ui.horizontal(|ui| {
                ui.label("Room code");
                ui.text_edit_singleline(&mut self.code);
            });
            if ui.button("Join online").clicked() {
                actions.push(UiAction::Join {
                    relay: relay.trim().to_string(),
                    code: self.code.trim().to_uppercase(),
                    name: self.name.trim().to_string(),
                });
            }
            ui.separator();
            if ui.button("Quit").clicked() {
                actions.push(UiAction::Quit);
            }
        });
    }

    fn hud(&mut self, ctx: &egui::Context, frame: &Frame, hud: &LocalHud) {
        for event in frame.events {
            match event {
                Event::Toast { player, text } if player.is_none_or(|p| p == frame.me) => {
                    self.toasts.push((text.clone(), TOAST_SECONDS));
                }
                Event::Denied { player, reason } if *player == frame.me => {
                    self.toasts.push((reason.clone(), TOAST_SECONDS));
                }
                _ => {}
            }
        }
        self.toasts.retain_mut(|(_, left)| {
            *left -= frame.dt;
            *left > 0.0
        });

        let world = frame.world;
        let Some(me) = world.player(frame.me) else {
            return;
        };
        egui::Area::new(egui::Id::new("hud.vitals"))
            .anchor(Align2::LEFT_BOTTOM, [12.0, -12.0])
            .show(ctx, |ui| {
                ui.label(big(format!(
                    "HP {:.0}   Hunger {:.0}",
                    me.health, me.hunger
                )));
                ui.label(big(format!("Coins {}", me.coins)));
            });
        let clock = &world.clock;
        let hours = (clock.time / raft_sim::content::CYCLE_SECONDS * 24.0 + 6.0) % 24.0;
        egui::Area::new(egui::Id::new("hud.clock"))
            .anchor(Align2::RIGHT_TOP, [-12.0, 12.0])
            .show(ctx, |ui| {
                ui.label(big(format!(
                    "Day {}  {:02}:{:02}",
                    clock.day,
                    hours as u32,
                    (hours.fract() * 60.0) as u32
                )));
                if let Some(code) = frame.room_code {
                    ui.label(big(format!("Room {code}  ({} online)", frame.peers + 1)));
                }
            });
        if let Some(prompt) = &hud.prompt {
            egui::Area::new(egui::Id::new("hud.prompt"))
                .anchor(Align2::CENTER_CENTER, [0.0, 40.0])
                .show(ctx, |ui| ui.label(big(prompt.clone())));
        }
        egui::Area::new(egui::Id::new("hud.toasts"))
            .anchor(Align2::CENTER_TOP, [0.0, 60.0])
            .show(ctx, |ui| {
                for (text, _) in &self.toasts {
                    ui.label(big(text.clone()));
                }
            });
    }
}

/// Menu text for a session's `Status::Closed` reason.
fn notice_text(reason: &str) -> &str {
    match reason {
        "name taken" => "Someone else already plays that name in this world. Pick another name.",
        "world full" => "This world has no room for new sailors. Rejoin with a name it knows.",
        "bad token" => "The host refused this install's identity token.",
        "Version mismatch" => "The host runs a different version of the game.",
        other => other,
    }
}

fn big(text: String) -> RichText {
    RichText::new(text).size(18.0).color(Color32::WHITE)
}

fn center_window(ctx: &egui::Context, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::Window::new(title)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .collapsible(false)
        .resizable(false)
        .show(ctx, body);
}
