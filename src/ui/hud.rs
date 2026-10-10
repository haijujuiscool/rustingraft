//! The in-game HUD, painted straight on the screen below every window: vitals, hotbar,
//! compass, clock and warnings, stockpile, prompt, crosshair, toasts, fishing bar and
//! build hints.

use super::style::{
    self, hud_bg, AMBER, CORAL, CREAM, GOLD, LEAF, SAND, SAND_DEEP, SEA,
};
use super::{Toast, ToastKind, Ui, MAX_TOASTS, TOAST_SECONDS};
use crate::shared::{Frame, LocalHud};
use raft_sim::content::{
    self, harpoon, melee, rod, PlaceableKind, QuestGoal, QuestId, Spot, CYCLE_SECONDS,
    DAY_SECONDS, MAX_HEALTH, MAX_HUNGER,
};
use raft_sim::islands::QuestStatus;
use raft_sim::survivors::SurvivorState;
use raft_sim::{Event, Phase, Player, Stack, V3};
use rusting_engine::egui::{self, Align2, Color32, FontId, LayerId, Painter, Pos2, Rect, Stroke, Vec2};
use rusting_engine::prelude::GameScene;
use std::f32::consts::{FRAC_PI_2, PI, TAU};

/// The scene's player controller object (its yaw is the camera heading).
const CONTROLLER: &str = "Player";
const PAD: f32 = 16.0;
const SLOT: Vec2 = Vec2::new(96.0, 64.0);
const SLOT_GAP: f32 = 6.0;
const COMPASS_W: f32 = 480.0;
const COMPASS_H: f32 = 34.0;
/// Fuel seconds below which the HUD warns.
const FUEL_LOW: f32 = 60.0;

impl Ui {
    pub(super) fn hud(
        &mut self,
        ctx: &egui::Context,
        scene: &mut GameScene,
        frame: &Frame,
        hud: &LocalHud,
    ) {
        self.collect_toasts(frame);
        let Some(me) = frame.world.player(frame.me) else {
            return;
        };
        let s = self.scale;
        let p = ctx.layer_painter(LayerId::background());
        let screen = ctx.screen_rect();
        let yaw = scene.player(CONTROLLER).map_or(me.yaw, |c| c.yaw);
        let modal = self.modal();

        stockpile(&p, s, screen, frame, me);
        compass(&p, s, screen, frame, me, yaw);
        let right = status(&p, s, screen, frame, me);
        self.paint_toasts(&p, s, Pos2::new(screen.right() - PAD * s, right + 8.0 * s));
        vitals(&p, s, screen, me);
        let hotbar_top = hotbar(&p, s, screen, me, hud.selected_slot);
        let keys = self.keys.as_ref().expect("read on the first draw");
        let mut above = hotbar_top - 10.0 * s;
        if hud.fishing.reeling {
            above = fishing_bar(&p, s, Pos2::new(screen.center().x, above), hud, &keys.use_tool);
        }
        if let Some(ghost) = &hud.ghost {
            let name = ghost.recipe.def().name;
            let keys = [
                (keys.use_tool.as_str(), "Place"),
                (keys.rotate.as_str(), "Rotate"),
                (keys.pause.as_str(), "Cancel"),
            ];
            build_hint(&p, s, Pos2::new(screen.center().x, above), name, ghost.valid, &keys);
        }
        if me.downed {
            downed(&p, s, screen);
        } else if !modal {
            if hud.aiming {
                crosshair(&p, s, screen.center());
            }
            if let Some(prompt) = &hud.prompt {
                // 60% down: below the crosshair, clear of the third-person avatar's head.
                let at = Pos2::new(screen.center().x, screen.top() + screen.height() * 0.6);
                prompt_chip(&p, s, at, prompt);
            }
        }
    }

    /// Turns this frame's events into toasts and ages the old ones.
    fn collect_toasts(&mut self, frame: &Frame) {
        let me = frame.me;
        for event in frame.events {
            let (text, kind) = match event {
                Event::Toast { player, text } if player.is_none_or(|p| p == me) => {
                    (text.clone(), ToastKind::Info)
                }
                Event::Denied { player, reason } if *player == me => {
                    (reason.clone(), ToastKind::Denied)
                }
                Event::Gained { player, goods } if *player == me => {
                    (format!("+ {}", goods.label()), ToastKind::Gained)
                }
                _ => continue,
            };
            push_toast(&mut self.toasts, text, kind);
        }
        for toast in &mut self.toasts {
            toast.left -= frame.dt;
        }
        self.toasts.retain(|t| t.left > 0.0);
    }

    /// Right-aligned toast stack starting at `top_right`, newest at the bottom.
    fn paint_toasts(&self, p: &Painter, s: f32, top_right: Pos2) {
        let mut y = top_right.y;
        for toast in &self.toasts {
            let fade = toast.left.clamp(0.0, 1.0);
            let (accent, text_color) = match toast.kind {
                ToastKind::Info => (SAND, CREAM),
                ToastKind::Denied => (CORAL, Color32::from_rgb(255, 196, 182)),
                ToastKind::Gained => (LEAF, Color32::from_rgb(200, 240, 180)),
            };
            let text = if toast.count > 1 {
                format!("{}  x{}", toast.text, toast.count)
            } else {
                toast.text.clone()
            };
            let galley = p.layout(
                text,
                FontId::proportional(16.0 * s),
                text_color.gamma_multiply(fade),
                300.0 * s,
            );
            let size = galley.size() + Vec2::new(26.0 * s, 12.0 * s);
            let rect = Rect::from_min_size(Pos2::new(top_right.x - size.x, y), size);
            p.rect_filled(rect, style::radius(6.0 * s), hud_bg().gamma_multiply(fade));
            let bar = Rect::from_min_size(rect.min, Vec2::new(4.0 * s, size.y));
            p.rect_filled(bar, style::radius(2.0 * s), accent.gamma_multiply(fade));
            p.galley(rect.min + Vec2::new(16.0 * s, 6.0 * s), galley, CREAM);
            y = rect.bottom() + 6.0 * s;
        }
    }
}

fn push_toast(toasts: &mut Vec<Toast>, text: String, kind: ToastKind) {
    if let Some(last) = toasts.last_mut() {
        if last.text == text && last.kind == kind {
            last.count += 1;
            last.left = TOAST_SECONDS;
            return;
        }
    }
    toasts.push(Toast {
        text,
        kind,
        left: TOAST_SECONDS,
        count: 1,
    });
    if toasts.len() > MAX_TOASTS {
        toasts.remove(0);
    }
}

/// A rounded dark chip.
fn chip(p: &Painter, s: f32, rect: Rect) {
    p.rect_filled(rect, style::radius(8.0 * s), hud_bg());
}

fn font(px: f32, s: f32) -> FontId {
    FontId::proportional(px * s)
}

/// Health and hunger bars, bottom left.
fn vitals(p: &Painter, s: f32, screen: Rect, me: &Player) {
    let w = 250.0 * s;
    let bar_h = 18.0 * s;
    let rect = Rect::from_min_size(
        Pos2::new(screen.left() + PAD * s, screen.bottom() - PAD * s - 66.0 * s),
        Vec2::new(w, 66.0 * s),
    );
    chip(p, s, rect);
    let rows = [
        ("Health", me.health / MAX_HEALTH, me.health, CORAL),
        ("Hunger", me.hunger / MAX_HUNGER, me.hunger, AMBER),
    ];
    for (i, (label, frac, value, color)) in rows.into_iter().enumerate() {
        let y = rect.top() + 10.0 * s + i as f32 * (bar_h + 10.0 * s);
        let track = Rect::from_min_size(
            Pos2::new(rect.left() + 10.0 * s, y),
            Vec2::new(w - 20.0 * s, bar_h),
        );
        p.rect_filled(track, style::radius(5.0 * s), Color32::from_black_alpha(140));
        let mut fill = track;
        fill.set_width(track.width() * frac.clamp(0.0, 1.0));
        let low = frac < 0.25;
        let color = if low { color.gamma_multiply(1.15) } else { color };
        p.rect_filled(fill, style::radius(5.0 * s), color);
        let text = format!("{label}  {}", value.max(0.0).ceil() as i32);
        style::shadow_text(
            p,
            track.left_center() + Vec2::new(8.0 * s, 0.0),
            Align2::LEFT_CENTER,
            &text,
            font(14.0, s),
            CREAM,
        );
    }
}

/// The six hotbar slots, bottom centre. Returns the hotbar's top edge.
fn hotbar(p: &Painter, s: f32, screen: Rect, me: &Player, selected: u8) -> f32 {
    let slot = SLOT * s;
    let gap = SLOT_GAP * s;
    let total = slot.x * 6.0 + gap * 5.0;
    let top = screen.bottom() - PAD * s - slot.y;
    let hand = |stack: Option<Stack>, empty: &str| match stack {
        Some(st) => (st.kind.name().to_string(), format!("x{}", st.count), true),
        None => (empty.to_string(), "empty".to_string(), false),
    };
    let used = me.sac.stacks().count();
    let slots = [
        (harpoon(me.gear.harpoon).name.to_string(), "Harpoon".to_string(), true),
        ("Sac".to_string(), format!("{used}/{}", me.sac.capacity()), true),
        (melee(me.gear.melee).name.to_string(), "Melee".to_string(), true),
        hand(me.food, "Food"),
        hand(me.heal, "Heal"),
        (rod(me.gear.rod).name.to_string(), "Rod".to_string(), true),
    ];
    for (i, (name, sub, filled)) in slots.into_iter().enumerate() {
        let x = screen.center().x - total / 2.0 + i as f32 * (slot.x + gap);
        let on = selected as usize == i + 1;
        let rect = Rect::from_min_size(Pos2::new(x, top - if on { 4.0 * s } else { 0.0 }), slot);
        p.rect_filled(
            rect,
            style::radius(8.0 * s),
            if on {
                Color32::from_rgba_unmultiplied(20, 70, 76, 220)
            } else {
                hud_bg()
            },
        );
        if on {
            p.rect_stroke(
                rect,
                style::radius(8.0 * s),
                Stroke::new(2.5 * s, Color32::from_rgb(120, 214, 210)),
                egui::StrokeKind::Inside,
            );
        }
        p.text(
            rect.left_top() + Vec2::new(7.0 * s, 4.0 * s),
            Align2::LEFT_TOP,
            (i + 1).to_string(),
            font(13.0, s),
            if on { GOLD } else { SAND_DEEP.gamma_multiply(0.8) },
        );
        let color = if filled { CREAM } else { CREAM.gamma_multiply(0.45) };
        // One line: long tier names shrink to fit the slot.
        let fit = slot.x - 10.0 * s;
        let mut galley = p.layout_no_wrap(name.clone(), font(13.5, s), color);
        if galley.size().x > fit {
            let size = (13.5 * fit / galley.size().x).max(9.0);
            galley = p.layout_no_wrap(name, font(size, s), color);
        }
        let name_pos = Pos2::new(
            rect.center().x - galley.size().x / 2.0,
            rect.top() + 20.0 * s,
        );
        p.galley(name_pos, galley, color);
        p.text(
            rect.center_bottom() - Vec2::new(0.0, 5.0 * s),
            Align2::CENTER_BOTTOM,
            sub,
            font(12.5, s),
            if filled { SAND_DEEP } else { SAND_DEEP.gamma_multiply(0.5) },
        );
    }
    top - 4.0 * s
}

/// Bearing from `from` to `to` relative to `yaw`, in radians; positive is to the left.
fn relative_bearing(from: V3, to: V3, yaw: f32) -> f32 {
    let bearing = (-(to.x - from.x)).atan2(-(to.z - from.z));
    wrap(bearing - yaw)
}

fn wrap(a: f32) -> f32 {
    (a + PI).rem_euclid(TAU) - PI
}

fn dist_xz(a: V3, b: V3) -> f32 {
    ((a.x - b.x).powi(2) + (a.z - b.z).powi(2)).sqrt()
}

struct Marker {
    at: V3,
    color: Color32,
    label: Option<String>,
    /// Keep the label while the marker sits clamped at the strip's edge (behind you).
    edge_label: bool,
}

/// Heading strip, top centre: cardinals, discovered islands, quest target, survivor
/// signals (with a radio), downed crewmates.
fn compass(p: &Painter, s: f32, screen: Rect, frame: &Frame, me: &Player, yaw: f32) {
    let w = COMPASS_W * s;
    let rect = Rect::from_center_size(
        Pos2::new(screen.center().x, screen.top() + PAD * s + COMPASS_H * s / 2.0),
        Vec2::new(w, COMPASS_H * s),
    );
    chip(p, s, rect);
    let x_of = |rel: f32| rect.center().x - rel / FRAC_PI_2 * (w / 2.0 - 14.0 * s);
    let visible = |rel: f32| rel.abs() <= FRAC_PI_2;
    // Ticks every 15 degrees, letters on the cardinals (N is -Z, E is +X).
    for i in 0..24 {
        let rel = wrap(i as f32 * TAU / 24.0 - yaw);
        if !visible(rel) {
            continue;
        }
        let x = x_of(rel);
        let letter = match i {
            0 => Some("N"),
            6 => Some("W"),
            12 => Some("S"),
            18 => Some("E"),
            _ => None,
        };
        match letter {
            Some(l) => {
                let color = if l == "N" { CORAL } else { CREAM };
                p.text(Pos2::new(x, rect.center().y), Align2::CENTER_CENTER, l, font(17.0, s), color);
            }
            None => {
                let h = if i % 3 == 0 { 9.0 } else { 5.0 } * s;
                p.line_segment(
                    [Pos2::new(x, rect.bottom() - 4.0 * s - h), Pos2::new(x, rect.bottom() - 4.0 * s)],
                    Stroke::new(1.2 * s, SAND.gamma_multiply(0.6)),
                );
            }
        }
    }
    // Centre notch.
    let c = rect.center_top();
    p.add(egui::Shape::convex_polygon(
        vec![c + Vec2::new(-6.0 * s, -2.0 * s), c + Vec2::new(6.0 * s, -2.0 * s), c + Vec2::new(0.0, 7.0 * s)],
        GOLD,
        Stroke::NONE,
    ));

    let world = frame.world;
    let mut markers: Vec<Marker> = Vec::new();
    for (id, st) in world.islands.list.iter().enumerate() {
        if st.discovered {
            let def = content::island(id as u8);
            markers.push(Marker {
                at: def.center(),
                color: SAND_DEEP,
                label: None,
                edge_label: false,
            });
        }
    }
    if let Some((at, label)) = quest_target(frame) {
        markers.push(Marker {
            at,
            color: GOLD,
            label: Some(label),
            edge_label: true,
        });
    }
    if world.raft.count(PlaceableKind::RadioStation) > 0 {
        for (id, st) in world.islands.list.iter().enumerate() {
            if st.survivor == SurvivorState::Waiting {
                let def = content::island(id as u8);
                markers.push(Marker {
                    at: def.spot(Spot::Survivor),
                    color: LEAF,
                    label: Some("SOS".to_string()),
                    edge_label: false,
                });
            }
        }
    }
    for (id, other) in world.connected() {
        if id != frame.me && other.downed {
            markers.push(Marker {
                at: other.pos,
                color: CORAL,
                label: Some(format!("{} down", other.name)),
                edge_label: true,
            });
        }
    }
    let mut labels = Vec::new();
    for m in markers {
        let rel = relative_bearing(me.pos, m.at, yaw);
        let x = x_of(rel.clamp(-FRAC_PI_2, FRAC_PI_2));
        let edge = !visible(rel);
        let y = rect.top() + 7.0 * s;
        let r = if m.label.is_some() { 5.5 } else { 4.0 } * s;
        let color = if edge { m.color.gamma_multiply(0.6) } else { m.color };
        p.add(egui::Shape::convex_polygon(
            vec![
                Pos2::new(x, y - r),
                Pos2::new(x + r, y),
                Pos2::new(x, y + r),
                Pos2::new(x - r, y),
            ],
            color,
            Stroke::new(1.0 * s, Color32::from_black_alpha(160)),
        ));
        if let Some(label) = m.label.filter(|_| !edge || m.edge_label) {
            labels.push((x, format!("{label}  {:.0} m", dist_xz(me.pos, m.at)), m.color));
        }
    }
    // Left to right, overlapping labels pushed apart; one pushed off the strip is dropped.
    labels.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut label_right = f32::MIN;
    for (x, text, color) in labels {
        let galley = p.layout_no_wrap(text, font(13.0, s), color);
        let width = galley.size().x;
        let lx = (x - width / 2.0)
            .clamp(rect.left(), rect.right() - width)
            .max(label_right + 6.0 * s);
        if lx + width > rect.right() {
            continue;
        }
        let pos = Pos2::new(lx, rect.bottom() + 4.0 * s);
        let bg = Rect::from_min_size(pos - Vec2::new(5.0 * s, 1.0 * s), galley.size() + Vec2::new(10.0 * s, 2.0 * s));
        p.rect_filled(bg, style::radius(4.0 * s), hud_bg());
        label_right = bg.right();
        p.galley(pos, galley, color);
    }
}

/// Where the first open quest sends the player, and its compass label.
fn quest_target(frame: &Frame) -> Option<(V3, String)> {
    let world = frame.world;
    QuestId::ALL.iter().find_map(|&q| {
        let def = q.def();
        let status = world.quests.status(q);
        if !matches!(status, QuestStatus::Active | QuestStatus::Ready) {
            return None;
        }
        let island = content::island(def.giver);
        let barnaby = def.reward.len() > 0 && def.goal != QuestGoal::Ending;
        match def.goal {
            // Done on the raft: nothing to steer toward.
            QuestGoal::CampfireLevel(_) if status == QuestStatus::Active => None,
            QuestGoal::SellFish(_) | QuestGoal::BringItem(_) | QuestGoal::CampfireLevel(_)
                if barnaby =>
            {
                Some((island.spot(Spot::Barnaby), "Barnaby".to_string()))
            }
            _ => Some((island.center(), island.name.to_string())),
        }
    })
}

/// Top-right column: clock, night warning, campfire fuel, room. Returns its bottom.
fn status(p: &Painter, s: f32, screen: Rect, frame: &Frame, me: &Player) -> f32 {
    let world = frame.world;
    let clock = &world.clock;
    let right = screen.right() - PAD * s;
    let mut y = screen.top() + PAD * s;
    let mut line = |text: String, color: Color32, accent: Option<Color32>, size: f32| {
        let galley = p.layout_no_wrap(text, font(size, s), color);
        let size = galley.size() + Vec2::new(24.0 * s, 10.0 * s);
        let rect = Rect::from_min_size(Pos2::new(right - size.x, y), size);
        chip(p, s, rect);
        if let Some(accent) = accent {
            p.rect_stroke(rect, style::radius(8.0 * s), Stroke::new(1.5 * s, accent), egui::StrokeKind::Inside);
        }
        p.galley(rect.min + Vec2::new(12.0 * s, 5.0 * s), galley, color);
        y = rect.bottom() + 6.0 * s;
    };
    let phase = clock.phase();
    let icon = match phase {
        Phase::Night => "Night",
        Phase::Dusk => "Dusk",
        Phase::Dawn => "Dawn",
        Phase::Day => "Day",
    };
    line(
        format!("Day {}   {}   {}", clock.day, clock_text(clock.time), icon),
        CREAM,
        None,
        18.0,
    );
    let campfire = &world.economy.campfire;
    match phase {
        Phase::Night | Phase::Dusk => {
            let dark = !world.in_light(me.pos);
            let text = if phase == Phase::Dusk {
                "Night is coming: stay near the fire"
            } else {
                "Night: stay near the fire"
            };
            let color = if dark && phase == Phase::Night { CORAL } else { AMBER };
            line(text.to_string(), color, Some(color), 15.0);
        }
        _ => {}
    }
    if !campfire.lit() {
        line("Campfire is out: feed it wood".to_string(), CORAL, Some(CORAL), 15.0);
    } else if campfire.fuel < FUEL_LOW {
        line(
            format!("Campfire fuel low  {:.0} s", campfire.fuel.ceil()),
            AMBER,
            Some(AMBER),
            15.0,
        );
    }
    if let Some(code) = frame.room_code {
        let n = frame.peers + 1;
        let sailors = if n == 1 { "sailor" } else { "sailors" };
        line(format!("Room {code}   {n} {sailors}"), CREAM, Some(SEA), 15.0);
    }
    y
}

/// Cycle seconds to a 24 h clock: the day runs 06:00-20:00, the night 20:00-06:00.
fn clock_text(time: f32) -> String {
    let night = CYCLE_SECONDS - DAY_SECONDS;
    let hours = if time < DAY_SECONDS {
        6.0 + time / DAY_SECONDS * 14.0
    } else {
        20.0 + (time - DAY_SECONDS) / night * 10.0
    };
    let minutes = (hours * 60.0) as u32 % (24 * 60);
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// Wood, metal, biomass, oil and coins, top left.
fn stockpile(p: &Painter, s: f32, screen: Rect, frame: &Frame, me: &Player) {
    let stock = &frame.world.economy.stock;
    let items = [
        ("Wood", stock.wood, Color32::from_rgb(196, 140, 86)),
        ("Metal", stock.metal, Color32::from_rgb(170, 182, 190)),
        ("Biomass", stock.biomass, LEAF),
        ("Oil", stock.oil, Color32::from_rgb(120, 110, 160)),
        ("Coins", me.coins, GOLD),
    ];
    let col = 66.0 * s;
    let rect = Rect::from_min_size(
        screen.left_top() + Vec2::splat(PAD * s),
        Vec2::new(col * items.len() as f32 + 12.0 * s, 46.0 * s),
    );
    chip(p, s, rect);
    for (i, (name, n, color)) in items.into_iter().enumerate() {
        let x = rect.left() + 12.0 * s + i as f32 * col;
        let dot = Pos2::new(x + 5.0 * s, rect.top() + 14.0 * s);
        if name == "Coins" {
            p.circle_filled(dot, 5.0 * s, color);
        } else {
            p.rect_filled(Rect::from_center_size(dot, Vec2::splat(9.0 * s)), style::radius(2.0 * s), color);
        }
        p.text(dot + Vec2::new(10.0 * s, 0.0), Align2::LEFT_CENTER, n.to_string(), font(17.0, s), CREAM);
        p.text(
            Pos2::new(x, rect.bottom() - 6.0 * s),
            Align2::LEFT_BOTTOM,
            name,
            font(11.5, s),
            SAND_DEEP.gamma_multiply(0.85),
        );
    }
}

/// Frames per second, counted over about half a second.
#[derive(Default)]
pub(super) struct Fps {
    frames: u32,
    seconds: f32,
    shown: u32,
}

impl Fps {
    /// Counts one frame of `real_dt` seconds; returns the rate to show (0 until the
    /// first half second is in).
    pub(super) fn tick(&mut self, real_dt: f32) -> u32 {
        self.frames += 1;
        self.seconds += real_dt;
        if self.seconds >= 0.5 {
            self.shown = (self.frames as f32 / self.seconds).round() as u32;
            self.frames = 0;
            self.seconds = 0.0;
        }
        self.shown
    }
}

/// "60 FPS", small in the bottom-right corner, where no HUD part sits (F3's engine
/// overlay is top right).
pub(super) fn fps_counter(ctx: &egui::Context, s: f32, fps: u32) {
    if fps == 0 {
        return;
    }
    let p = ctx.layer_painter(LayerId::background());
    let corner = ctx.screen_rect().right_bottom() - Vec2::splat(PAD * s * 0.5);
    let text = format!("{fps} FPS");
    style::shadow_text(&p, corner, Align2::RIGHT_BOTTOM, &text, font(13.0, s), SAND);
}

/// "E  Feed campfire": the key in a cap, then the text.
fn prompt_chip(p: &Painter, s: f32, center: Pos2, prompt: &str) {
    let (key, text) = match prompt.split_once("  ") {
        Some((k, t)) if k.len() <= 5 => (Some(k), t),
        _ => (None, prompt),
    };
    let galley = p.layout_no_wrap(text.to_string(), font(17.0, s), CREAM);
    let key_w = key.map_or(0.0, |k| {
        p.layout_no_wrap(k.to_string(), font(15.0, s), CREAM).size().x.max(12.0 * s) + 20.0 * s
    });
    let size = Vec2::new(galley.size().x + key_w + 28.0 * s, 36.0 * s);
    let rect = Rect::from_center_size(center, size);
    chip(p, s, rect);
    let mut x = rect.left() + 14.0 * s;
    if let Some(k) = key {
        x += style::paint_key(p, s, Pos2::new(x, rect.center().y), k) + 8.0 * s;
    }
    p.galley(Pos2::new(x, rect.center().y - galley.size().y / 2.0), galley, CREAM);
}

fn crosshair(p: &Painter, s: f32, c: Pos2) {
    let stroke = Stroke::new(2.0 * s, CREAM);
    let shadow = Stroke::new(4.0 * s, Color32::from_black_alpha(110));
    for (a, b) in [
        (Vec2::new(-12.0, 0.0), Vec2::new(-5.0, 0.0)),
        (Vec2::new(5.0, 0.0), Vec2::new(12.0, 0.0)),
        (Vec2::new(0.0, -12.0), Vec2::new(0.0, -5.0)),
        (Vec2::new(0.0, 5.0), Vec2::new(0.0, 12.0)),
    ] {
        p.line_segment([c + a * s, c + b * s], shadow);
        p.line_segment([c + a * s, c + b * s], stroke);
    }
    p.circle_filled(c, 1.5 * s, CREAM);
}

/// The reel-in timing bar. Returns the top edge it leaves free above itself.
fn fishing_bar(p: &Painter, s: f32, bottom_center: Pos2, hud: &LocalHud, key: &str) -> f32 {
    let f = &hud.fishing;
    let size = Vec2::new(380.0 * s, 58.0 * s);
    let rect = Rect::from_min_size(bottom_center - Vec2::new(size.x / 2.0, size.y), size);
    chip(p, s, rect);
    p.text(
        rect.center_top() + Vec2::new(0.0, 6.0 * s),
        Align2::CENTER_TOP,
        format!("Fish on!  {key} when the marker is in the green"),
        font(14.0, s),
        CREAM,
    );
    let track = Rect::from_min_size(
        Pos2::new(rect.left() + 14.0 * s, rect.bottom() - 24.0 * s),
        Vec2::new(size.x - 28.0 * s, 14.0 * s),
    );
    p.rect_filled(track, style::radius(4.0 * s), Color32::from_rgb(28, 52, 70));
    let zx = |v: f32| track.left() + track.width() * v.clamp(0.0, 1.0);
    let zone = Rect::from_x_y_ranges(zx(f.zone[0])..=zx(f.zone[1]), track.y_range());
    p.rect_filled(zone, style::radius(3.0 * s), LEAF);
    let mx = zx(f.marker);
    let inside = f.marker >= f.zone[0] && f.marker <= f.zone[1];
    p.rect_filled(
        Rect::from_center_size(Pos2::new(mx, track.center().y), Vec2::new(5.0 * s, 22.0 * s)),
        style::radius(2.0 * s),
        if inside { GOLD } else { CREAM },
    );
    rect.top() - 8.0 * s
}

fn build_hint(p: &Painter, s: f32, bottom_center: Pos2, name: &str, valid: bool, keys: &[(&str, &str)]) {
    let title = if valid {
        format!("Placing {name}")
    } else {
        format!("Placing {name}: not here")
    };
    let title_color = if valid { CREAM } else { Color32::from_rgb(255, 170, 150) };
    let title_g = p.layout_no_wrap(title, font(16.0, s), title_color);
    let labels: Vec<_> = keys
        .iter()
        .map(|(k, what)| {
            let kw = p.layout_no_wrap(k.to_string(), font(15.0, s), CREAM).size().x.max(12.0 * s) + 12.0 * s;
            let g = p.layout_no_wrap(what.to_string(), font(14.0, s), SAND);
            (*k, kw, g)
        })
        .collect();
    let keys_w: f32 = labels.iter().map(|(_, kw, g)| kw + 6.0 * s + g.size().x + 16.0 * s).sum();
    let w = title_g.size().x.max(keys_w) + 28.0 * s;
    let rect = Rect::from_min_size(bottom_center - Vec2::new(w / 2.0, 64.0 * s), Vec2::new(w, 64.0 * s));
    chip(p, s, rect);
    if !valid {
        p.rect_stroke(rect, style::radius(8.0 * s), Stroke::new(1.5 * s, CORAL), egui::StrokeKind::Inside);
    }
    p.galley(
        Pos2::new(rect.center().x - title_g.size().x / 2.0, rect.top() + 7.0 * s),
        title_g,
        title_color,
    );
    let mut x = rect.center().x - keys_w / 2.0;
    let y = rect.bottom() - 18.0 * s;
    for (k, _, g) in labels {
        x += style::paint_key(p, s, Pos2::new(x, y), k) + 6.0 * s;
        let gw = g.size().x;
        p.galley(Pos2::new(x, y - g.size().y / 2.0), g, SAND);
        x += gw + 16.0 * s;
    }
}

fn downed(p: &Painter, s: f32, screen: Rect) {
    p.rect_filled(screen, 0.0, Color32::from_rgba_unmultiplied(90, 10, 4, 90));
    let c = screen.center() - Vec2::new(0.0, 40.0 * s);
    style::shadow_text(p, c, Align2::CENTER_CENTER, "You are down", font(36.0, s), CREAM);
    style::shadow_text(
        p,
        c + Vec2::new(0.0, 38.0 * s),
        Align2::CENTER_CENTER,
        "A crewmate can revive you: hold E next to you.",
        font(17.0, s),
        SAND,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_runs_from_dawn_to_night() {
        assert_eq!(clock_text(0.0), "06:00");
        assert_eq!(clock_text(150.0), "13:00");
        assert_eq!(clock_text(300.0), "20:00");
        assert_eq!(clock_text(330.0), "01:00");
        assert_eq!(clock_text(359.0), "05:50");
    }

    #[test]
    fn bearings_turn_left_positive() {
        let o = V3::xz(0.0, 0.0);
        // Facing north (-Z): something to the west (-X) is on the left.
        assert!(relative_bearing(o, V3::xz(-10.0, 0.0), 0.0) > 1.5);
        assert!(relative_bearing(o, V3::xz(10.0, 0.0), 0.0) < -1.5);
        assert!(relative_bearing(o, V3::xz(0.0, -10.0), 0.0).abs() < 1e-4);
        // Turned to face east, east is straight ahead.
        assert!(relative_bearing(o, V3::xz(10.0, 0.0), -FRAC_PI_2).abs() < 1e-4);
    }

    #[test]
    fn fps_counts_frames_over_half_a_second() {
        let mut fps = Fps::default();
        // Nothing until half a second is in, then frames over that time.
        for _ in 0..31 {
            assert_eq!(fps.tick(1.0 / 64.0), 0);
        }
        assert_eq!(fps.tick(1.0 / 64.0), 64);
        // Held until the next half second; a hitch drags the next reading down.
        assert_eq!(fps.tick(0.25), 64);
        assert_eq!(fps.tick(0.25), 4);
    }

    #[test]
    fn identical_toasts_merge() {
        let mut t = Vec::new();
        push_toast(&mut t, "Hi".into(), ToastKind::Info);
        push_toast(&mut t, "Hi".into(), ToastKind::Info);
        push_toast(&mut t, "Hi".into(), ToastKind::Denied);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].count, 2);
        for i in 0..10 {
            push_toast(&mut t, format!("n{i}"), ToastKind::Gained);
        }
        assert_eq!(t.len(), MAX_TOASTS);
    }
}
