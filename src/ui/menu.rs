//! Main menu, connecting screen, pause menu, settings, leave/quit confirmation and the
//! game-over card.

use super::style::{self, CREAM, INK, MUTED, RED_INK, SAND, WOOD};
use super::{notice_text, Confirm, Ui};
use crate::shared::{Frame, Settings, UiAction, UiState};
use rusting_engine::egui::{self, Align2, Color32, Id, RichText, Vec2};

const MENU_W: f32 = 360.0;
const HELP_W: f32 = 380.0;
const GAP: f32 = 24.0;
/// Window margin and border around a card's content width.
const FRAME: f32 = 34.0;
/// Top of the main-menu cards, below the title.
const MENU_TOP: f32 = 146.0;

impl Ui {
    pub(super) fn main_menu(
        &mut self,
        ctx: &egui::Context,
        state: &UiState,
        out: &mut Vec<UiAction>,
    ) {
        let s = self.scale;
        style::backdrop(ctx);
        title(ctx, s);
        if self.editing.is_some() {
            return;
        }
        let settings = state.settings;
        let name = self.name.get_or_insert_with(|| settings.name.clone());
        let mut open_settings = false;
        let half = (MENU_W + HELP_W + 2.0 * FRAME + GAP) / 2.0;
        let left = -(half - (MENU_W + FRAME) / 2.0);
        card(ctx, "menu.main", s, MENU_W, Align2::CENTER_TOP, [left, MENU_TOP], |ui| {
            if let Some(notice) = state.notice {
                egui::Frame::new()
                    .fill(Color32::from_rgb(250, 222, 208))
                    .stroke(egui::Stroke::new(1.0 * s, style::CORAL))
                    .corner_radius(style::radius(6.0 * s))
                    .inner_margin(style::margin(8.0 * s))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let text = notice_text(notice);
                        ui.label(RichText::new(text).color(RED_INK));
                        if text != notice {
                            ui.label(style::note(notice, s));
                        }
                    });
                ui.add_space(4.0 * s);
            }
            ui.label(RichText::new("Your name").color(MUTED));
            ui.add(
                egui::TextEdit::singleline(name)
                    .char_limit(20)
                    .text_color(INK)
                    .margin(egui::Margin::symmetric((8.0 * s) as i8, (6.0 * s) as i8))
                    .desired_width(f32::INFINITY),
            );
            let name = name.trim();
            let name = if name.is_empty() { settings.name.as_str() } else { name };
            let rename = |out: &mut Vec<UiAction>| {
                if name != settings.name {
                    let mut next = settings.clone();
                    next.name = name.to_string();
                    out.push(UiAction::SetSettings(next));
                }
            };
            ui.add_space(4.0 * s);
            if style::big_button(ui, s, "New world", true, true) {
                rename(out);
                out.push(UiAction::StartSolo);
            }
            if style::big_button(ui, s, "Load world", false, state.has_save) {
                rename(out);
                out.push(UiAction::LoadWorld);
            }
            if !state.has_save {
                ui.label(style::note("No saved world yet: save at the Captain's Logbook.", s));
            }

            style::section(ui, s, "Play online");
            ui.label(style::note(format!("Relay {}  (change it in Settings)", settings.relay), s));
            if state.has_save {
                ui.checkbox(&mut self.host_load, "Host my saved world");
            }
            if style::big_button(ui, s, "Host online", false, true) {
                out.push(UiAction::Host {
                    relay: settings.relay.clone(),
                    name: name.to_string(),
                    load: self.host_load && state.has_save,
                });
            }
            ui.horizontal(|ui| {
                let code_w = ui.available_width() - 150.0 * s;
                ui.add(
                    egui::TextEdit::singleline(&mut self.code)
                        .hint_text("Room code")
                        .char_limit(12)
                        .text_color(INK)
                        .margin(egui::Margin::symmetric((8.0 * s) as i8, (6.0 * s) as i8))
                        .desired_width(code_w),
                );
                let code = self.code.trim().to_uppercase();
                let join = egui::Button::new(RichText::new("Join online").size(18.0 * s))
                    .min_size(Vec2::new(ui.available_width(), 34.0 * s));
                if ui.add_enabled(!code.is_empty(), join).clicked() {
                    out.push(UiAction::Join {
                        relay: settings.relay.clone(),
                        code,
                        name: name.to_string(),
                    });
                }
            });
            ui.add_space(6.0 * s);
            ui.separator();
            ui.columns(2, |cols| {
                if style::big_button(&mut cols[0], s, "Settings", false, true) {
                    open_settings = true;
                }
                if style::big_button(&mut cols[1], s, "Quit", false, true) {
                    out.push(UiAction::Quit);
                }
            });
        });
        let keys = self.keys.as_ref().expect("read on the first draw");
        card(ctx, "menu.help", s, HELP_W, Align2::CENTER_TOP, [half - (HELP_W + FRAME) / 2.0, MENU_TOP], |ui| {
            ui.label(RichText::new("How to play").size(22.0 * s).color(WOOD).strong());
            ui.label(
                "Harpoon floating debris to grow your raft. Keep the campfire fed: it \
                 lights the night and unlocks crafting. Sell fish to Barnaby on Hook Isle, \
                 sail out to the islands, rescue the survivors and light the beacon on \
                 Lighthouse Atoll.",
            );
            ui.add_space(4.0 * s);
            let rows: [(&str, &str); 12] = [
                ("W A S D", "Move, Shift to sprint"),
                ("Space", "Jump, swim up"),
                (&keys.use_tool, "Use the tool in your hand"),
                (&keys.aim, "Aim harpoon or rod"),
                ("1 - 6", "Hotbar slot"),
                (&keys.interact, "Interact, pick up"),
                (&keys.sac, "Open your sac"),
                (&keys.build, "Build menu at the workbench"),
                (&keys.rotate, "Rotate while placing"),
                (&keys.camera, "First / third person"),
                (&keys.pause, "Pause menu"),
                ("Night", "Stay inside the campfire light"),
            ];
            egui::Grid::new("menu.help.keys")
                .num_columns(2)
                .spacing([10.0 * s, 5.0 * s])
                .show(ui, |ui| {
                    for (key, what) in rows {
                        style::key_cap(ui, s, key);
                        ui.label(what);
                        ui.end_row();
                    }
                });
        });
        if open_settings {
            self.editing = Some(settings.clone());
        }
    }

    pub(super) fn connecting(
        &mut self,
        ctx: &egui::Context,
        state: &UiState,
        out: &mut Vec<UiAction>,
    ) {
        let s = self.scale;
        style::backdrop(ctx);
        title(ctx, s);
        card(ctx, "menu.connecting", s, MENU_W, Align2::CENTER_CENTER, [0.0, 40.0], |ui| {
            ui.vertical_centered(|ui| {
                ui.add(egui::Spinner::new().size(36.0 * s).color(style::SEA));
                ui.label(RichText::new("Connecting...").size(22.0 * s).color(WOOD).strong());
                ui.label(style::note(format!("Relay {}", state.settings.relay), s));
                ui.add_space(6.0 * s);
            });
            if style::big_button(ui, s, "Cancel", false, true) {
                out.push(UiAction::ToMenu);
            }
        });
    }

    pub(super) fn pause_menu(
        &mut self,
        ctx: &egui::Context,
        frame: &Frame,
        settings: &Settings,
        out: &mut Vec<UiAction>,
    ) {
        let s = self.scale;
        let online = !(frame.is_host && frame.room_code.is_none() && frame.peers == 0);
        if let Some(confirm) = self.confirm {
            style::dim(ctx, Color32::from_black_alpha(150));
            let (heading, yes) = match confirm {
                Confirm::Leave => ("Leave this world?", "Leave world"),
                Confirm::Quit => ("Quit the game?", "Quit game"),
            };
            card(ctx, "menu.confirm", s, 400.0, Align2::CENTER_CENTER, [0.0, 0.0], |ui| {
                ui.label(RichText::new(heading).size(24.0 * s).color(WOOD).strong());
                ui.label("Unsaved progress is lost. Save the world at the Captain's Logbook first.");
                if online && frame.is_host {
                    ui.label(
                        RichText::new("You are the host: everyone in your room is disconnected.")
                            .color(RED_INK),
                    );
                }
                ui.add_space(8.0 * s);
                ui.horizontal(|ui| {
                    if style::coral_button(ui, s, yes) {
                        out.push(match confirm {
                            Confirm::Leave => UiAction::ToMenu,
                            Confirm::Quit => UiAction::Quit,
                        });
                    }
                    let stay = egui::Button::new(RichText::new("Stay").size(18.0 * s))
                        .min_size(Vec2::new(120.0 * s, 38.0 * s));
                    if ui.add(stay).clicked() {
                        self.confirm = None;
                    }
                });
            });
            return;
        }
        if !self.paused || self.editing.is_some() {
            return;
        }
        style::dim(ctx, Color32::from_black_alpha(120));
        card(ctx, "menu.pause", s, 320.0, Align2::CENTER_CENTER, [0.0, 0.0], |ui| {
            ui.vertical_centered(|ui| {
                let heading = if online { "Menu" } else { "Paused" };
                ui.label(RichText::new(heading).size(28.0 * s).color(WOOD).strong());
                if online {
                    ui.label(style::note("Online: the world keeps running.", s));
                }
            });
            ui.add_space(6.0 * s);
            if style::big_button(ui, s, "Resume", true, true) {
                self.paused = false;
            }
            if style::big_button(ui, s, "Settings", false, true) {
                self.editing = Some(settings.clone());
            }
            if style::big_button(ui, s, "Main menu", false, true) {
                self.confirm = Some(Confirm::Leave);
            }
            if style::big_button(ui, s, "Quit", false, true) {
                self.confirm = Some(Confirm::Quit);
            }
        });
    }

    pub(super) fn settings_window(
        &mut self,
        ctx: &egui::Context,
        state: &UiState,
        out: &mut Vec<UiAction>,
    ) {
        let s = self.scale;
        let Some(draft) = self.editing.as_mut() else {
            return;
        };
        let in_game = state.frame.is_some();
        if in_game {
            style::dim(ctx, Color32::from_black_alpha(120));
        }
        let mut close = false;
        card(ctx, "menu.settings", s, 460.0, Align2::CENTER_CENTER, [0.0, 30.0], |ui| {
            ui.label(RichText::new("Settings").size(26.0 * s).color(WOOD).strong());
            ui.add_space(4.0 * s);
            egui::Grid::new("menu.settings.grid")
                .num_columns(2)
                .spacing([14.0 * s, 10.0 * s])
                .show(ui, |ui| {
                    ui.label("Name");
                    ui.add(
                        egui::TextEdit::singleline(&mut draft.name)
                            .char_limit(20)
                            .text_color(INK)
                            .margin(egui::Margin::symmetric((8.0 * s) as i8, (6.0 * s) as i8))
                            .desired_width(240.0 * s),
                    );
                    ui.end_row();
                    ui.label("Relay address");
                    ui.add(
                        egui::TextEdit::singleline(&mut draft.relay)
                            .text_color(INK)
                            .margin(egui::Margin::symmetric((8.0 * s) as i8, (6.0 * s) as i8))
                            .desired_width(240.0 * s),
                    );
                    ui.end_row();
                    ui.label("Volume");
                    ui.add(
                        egui::Slider::new(&mut draft.volume, 0.0..=1.0)
                            .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                    );
                    ui.end_row();
                    ui.label("Mouse sensitivity");
                    ui.add(
                        egui::Slider::new(&mut draft.mouse_sensitivity, 0.2..=3.0)
                            .custom_formatter(|v, _| format!("{v:.1}x")),
                    );
                    ui.end_row();
                    ui.label("Render scale");
                    ui.add(
                        egui::Slider::new(&mut draft.render_scale, 0.5..=1.5)
                            .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                    );
                    ui.end_row();
                    ui.label("Max FPS");
                    egui::ComboBox::from_id_salt("menu.settings.max_fps")
                        .selected_text(fps_label(draft.max_fps))
                        // The default 200 px cuts off "Unlimited" at a large UI scale.
                        .height(f32::INFINITY)
                        .show_ui(ui, |ui| {
                            for fps in [30, 60, 120, 144, 240, 0] {
                                ui.selectable_value(&mut draft.max_fps, fps, fps_label(fps));
                            }
                        });
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(&mut draft.show_fps, "Show FPS");
                    ui.end_row();
                });
            if in_game {
                ui.label(style::note(
                    "Name and relay apply the next time you start or join a world.",
                    s,
                ));
            }
            ui.add_space(6.0 * s);
            ui.horizontal(|ui| {
                let save = egui::Button::new(RichText::new("Save").size(18.0 * s).color(CREAM))
                    .fill(WOOD)
                    .min_size(Vec2::new(130.0 * s, 36.0 * s));
                if ui.add(save).clicked() {
                    let mut next = draft.clone();
                    next.name = next.name.trim().to_string();
                    next.relay = next.relay.trim().to_string();
                    if next.name.is_empty() {
                        next.name = state.settings.name.clone();
                    }
                    if next.relay.is_empty() {
                        next.relay = state.settings.relay.clone();
                    }
                    out.push(UiAction::SetSettings(next));
                    close = true;
                }
                let cancel = egui::Button::new(RichText::new("Cancel").size(18.0 * s))
                    .min_size(Vec2::new(110.0 * s, 36.0 * s));
                if ui.add(cancel).clicked() {
                    close = true;
                }
            });
        });
        if close {
            if let Some(UiAction::SetSettings(next)) = out.last() {
                self.name = Some(next.name.clone());
            }
            self.editing = None;
        }
    }

    pub(super) fn game_over(
        &mut self,
        ctx: &egui::Context,
        state: &UiState,
        out: &mut Vec<UiAction>,
    ) {
        let s = self.scale;
        style::dim(ctx, Color32::from_rgba_unmultiplied(60, 8, 4, 150));
        let host = state.frame.is_none_or(|f| f.is_host);
        card(ctx, "menu.game_over", s, 380.0, Align2::CENTER_CENTER, [0.0, 0.0], |ui| {
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("Game over").size(30.0 * s).color(RED_INK).strong());
                ui.label("Everyone is down. The raft drifts on without a crew.");
            });
            ui.add_space(8.0 * s);
            if host {
                if style::big_button(ui, s, "Load last save", true, state.has_save) {
                    out.push(UiAction::LoadLastSave);
                }
                if !state.has_save {
                    ui.label(style::note("No save yet.", s));
                }
            } else {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(20.0 * s).color(style::SEA));
                    ui.label("Waiting for the host to load the last save...");
                });
            }
            if style::big_button(ui, s, "Main menu", false, true) {
                out.push(UiAction::ToMenu);
            }
        });
    }
}

/// Game title over the menu backdrop.
fn title(ctx: &egui::Context, s: f32) {
    egui::Area::new(Id::new("menu.title"))
        .anchor(Align2::CENTER_TOP, [0.0, 34.0 * s])
        .interactable(false)
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                let painter = ui.painter();
                let font = egui::FontId::proportional(56.0 * s);
                let size = painter
                    .layout_no_wrap("RUSTING RAFT".into(), font.clone(), CREAM)
                    .size();
                let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                style::shadow_text(
                    ui.painter(),
                    rect.center(),
                    Align2::CENTER_CENTER,
                    "RUSTING RAFT",
                    font,
                    CREAM,
                );
                ui.label(
                    RichText::new("Drift. Build. Survive the night.")
                        .size(19.0 * s)
                        .color(SAND),
                );
            });
        });
}

/// A fixed card window anchored at the screen centre plus `offset` (unscaled px).
pub(super) fn card(
    ctx: &egui::Context,
    id: &str,
    s: f32,
    width: f32,
    align: Align2,
    offset: [f32; 2],
    body: impl FnOnce(&mut egui::Ui),
) {
    egui::Window::new(id)
        .id(Id::new(id))
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .movable(false)
        .frame(style::window_frame(ctx))
        .anchor(align, [offset[0] * s, offset[1] * s])
        .min_width(width * s)
        .max_width(width * s)
        .show(ctx, |ui| {
            ui.set_width(width * s);
            body(ui);
        });
}

/// A `Settings::max_fps` choice: 0 is no cap.
fn fps_label(fps: u32) -> String {
    match fps {
        0 => "Unlimited".into(),
        n => n.to_string(),
    }
}
