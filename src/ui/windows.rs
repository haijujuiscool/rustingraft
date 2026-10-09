//! Game windows over the HUD: sac, workbench, campfire, logbook, Barnaby's shop, the
//! wandering trader and the radio. Every button sends a sim `Request`; results come
//! back as toasts.

use super::style::{self, CREAM, INK, LEAF_INK, MUTED, RED_INK, SEA, WOOD};
use super::{Open, Ui};
use crate::shared::{Frame, UiAction, Window};
use raft_sim::content::{
    self, ItemClass, ItemKind, Offer, PlaceableKind, QuestGoal, QuestId, RecipeOutput, Shop,
    Spot, Stockpile, BARNABY_STOCK, CAMPFIRE_MAX_LEVEL, CAMPFIRE_THRESHOLDS, FEED_OIL,
    FEED_WOOD, FUEL_CAP, FUEL_PER_OIL, FUEL_PER_WOOD, POINTS_PER_OIL, POINTS_PER_WOOD,
    RECIPES, TRADER_EVERY_DAYS,
};
use raft_sim::islands::QuestStatus;
use raft_sim::survivors::SurvivorState;
use raft_sim::trader::can_buy;
use raft_sim::{Hand, Player, Request, Stack, World};
use rusting_engine::egui::{self, Align, Align2, Color32, Id, Layout, RichText, Sense, Stroke, Vec2};

impl Ui {
    pub(super) fn windows(&mut self, ctx: &egui::Context, frame: &Frame, out: &mut Vec<UiAction>) {
        let Some(open) = self.open else {
            return;
        };
        if self.paused {
            return;
        }
        let Some(me) = frame.world.player(frame.me) else {
            return;
        };
        let s = self.scale;
        let (title, width) = match open {
            Open::Sac => ("Sac", 560.0),
            Open::Win(Window::Workbench) => ("Workbench", 620.0),
            Open::Win(Window::Campfire) => ("Campfire", 460.0),
            Open::Win(Window::Logbook) => ("Captain's Logbook", 580.0),
            Open::Win(Window::Fisherman) => ("Barnaby's Bait & Tackle", 900.0),
            Open::Win(Window::Trader) => ("Wandering Trader", 540.0),
            Open::Win(Window::Radio) => ("Radio Station", 480.0),
        };
        let max_h = ctx.screen_rect().height() - 220.0 * s;
        let mut close = false;
        let mut act = Vec::new();
        let keys = self.keys.as_ref().expect("read on the first draw");
        let close_key = if open == Open::Sac {
            format!("{} / {}", keys.sac, keys.pause)
        } else {
            keys.pause.clone()
        };
        let mut picked = self.picked;
        egui::Window::new(title)
            .id(Id::new(("ui.window", title)))
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .movable(false)
            .frame(style::window_frame(ctx))
            .anchor(Align2::CENTER_CENTER, [0.0, -10.0 * s])
            .min_width(width * s)
            .max_width(width * s)
            .show(ctx, |ui| {
                ui.set_width(width * s);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(title).size(25.0 * s).color(WOOD).strong());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let b = egui::Button::new(RichText::new("Close").size(16.0 * s))
                            .min_size(Vec2::new(80.0 * s, 30.0 * s));
                        if ui.add(b).clicked() {
                            close = true;
                        }
                        ui.label(style::note(close_key, s));
                    });
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height(max_h)
                    .auto_shrink([false, true])
                    .show(ui, |ui| match open {
                        Open::Sac => sac(ui, s, me, &mut picked, &mut act),
                        Open::Win(Window::Workbench) => workbench(ui, s, frame.world, me, &mut act, &mut close),
                        Open::Win(Window::Campfire) => campfire(ui, s, frame.world, me, &mut act),
                        Open::Win(Window::Logbook) => logbook(ui, s, frame, me, &mut act),
                        Open::Win(Window::Fisherman) => fisherman(ui, s, frame.world, me, &mut act),
                        Open::Win(Window::Trader) => trader(ui, s, frame.world, me, &mut act),
                        Open::Win(Window::Radio) => radio(ui, s, frame.world, me),
                    });
            });
        self.picked = picked;
        out.extend(act);
        if close {
            self.open = None;
            self.picked = None;
        }
    }
}

fn req(out: &mut Vec<UiAction>, r: Request) {
    out.push(UiAction::Request(r));
}

/// A right-aligned small button; returns clicked.
fn side_button(ui: &mut egui::Ui, s: f32, label: &str, enabled: bool) -> bool {
    let b = egui::Button::new(RichText::new(label).size(16.0 * s)).min_size(Vec2::new(86.0 * s, 30.0 * s));
    ui.add_enabled(enabled, b).clicked()
}

/// One sunken row: `left` fills the row, `right` is laid out from the right edge.
fn row(ui: &mut egui::Ui, s: f32, left: impl FnOnce(&mut egui::Ui), right: impl FnOnce(&mut egui::Ui)) {
    style::row_frame(s).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                right(ui);
                ui.with_layout(Layout::left_to_right(Align::Center), left);
            });
        });
    });
}

fn coins_line(ui: &mut egui::Ui, s: f32, coins: u32) {
    ui.label(RichText::new(format!("Your coins: {coins}")).size(16.0 * s).color(INK).strong());
}

/// "4 wood  2 metal" with the parts the stockpile lacks in red.
fn cost_text(ui: &mut egui::Ui, s: f32, cost: &Stockpile, have: &Stockpile) {
    for (need, got, name) in [
        (cost.wood, have.wood, "wood"),
        (cost.metal, have.metal, "metal"),
        (cost.biomass, have.biomass, "biomass"),
        (cost.oil, have.oil, "oil"),
    ] {
        if need > 0 {
            let color = if got >= need { LEAF_INK } else { RED_INK };
            ui.label(RichText::new(format!("{need} {name}")).size(15.0 * s).color(color));
        }
    }
}

fn class_tint(kind: ItemKind) -> Color32 {
    match kind.class() {
        ItemClass::Debris => Color32::from_rgb(214, 186, 150),
        ItemClass::Fish => Color32::from_rgb(170, 214, 214),
        ItemClass::Food => Color32::from_rgb(244, 204, 150),
        ItemClass::Heal => Color32::from_rgb(240, 178, 166),
        ItemClass::Bait => Color32::from_rgb(190, 216, 160),
        ItemClass::Misc => Color32::from_rgb(214, 208, 196),
        ItemClass::Quest => Color32::from_rgb(244, 220, 120),
    }
}

// ---------------------------------------------------------------- sac

fn sac(ui: &mut egui::Ui, s: f32, me: &Player, picked: &mut Option<usize>, out: &mut Vec<UiAction>) {
    let cap = me.sac.capacity();
    let used = me.sac.stacks().count();
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{used} / {cap} slots")).color(INK).strong());
        ui.label(style::note("Click an item, then another slot to move it.", s));
    });
    let cols = match cap {
        0..=8 => 4,
        9..=12 => 6,
        _ => 8,
    };
    let gap = 6.0 * s;
    let cell = Vec2::new((ui.available_width() - gap * (cols - 1) as f32) / cols as f32, 62.0 * s);
    if picked.is_some_and(|i| me.sac.get(i).is_none()) {
        *picked = None;
    }
    egui::Grid::new("ui.sac.grid")
        .spacing([gap, gap])
        .show(ui, |ui| {
            for i in 0..cap {
                let stack = me.sac.get(i);
                let (rect, resp) = ui.allocate_exact_size(cell, Sense::click());
                let on = *picked == Some(i);
                let fill = stack.map_or(Color32::from_rgb(231, 214, 180), |st| class_tint(st.kind));
                let painter = ui.painter();
                painter.rect_filled(rect, style::radius(7.0 * s), fill);
                let stroke = if on {
                    Stroke::new(3.0 * s, SEA)
                } else if resp.hovered() {
                    Stroke::new(1.5 * s, INK)
                } else {
                    Stroke::new(1.0 * s, WOOD.gamma_multiply(0.5))
                };
                painter.rect_stroke(rect, style::radius(7.0 * s), stroke, egui::StrokeKind::Inside);
                if let Some(st) = stack {
                    let name = st.kind.name();
                    let wrap = rect.width() - 10.0 * s;
                    // Shrink until the longest word fits: names wrap between words only.
                    let longest = name
                        .split(' ')
                        .map(|w| painter.layout_no_wrap(w.into(), egui::FontId::proportional(13.0 * s), INK).size().x)
                        .fold(0.0, f32::max);
                    let size = 13.0 * s * (wrap / longest).min(1.0);
                    let g = painter.layout(name.to_string(), egui::FontId::proportional(size), INK, wrap);
                    painter.galley(rect.left_top() + Vec2::new(6.0 * s, 5.0 * s), g, INK);
                    if st.count > 1 {
                        painter.text(
                            rect.right_bottom() - Vec2::new(6.0 * s, 4.0 * s),
                            Align2::RIGHT_BOTTOM,
                            format!("x{}", st.count),
                            egui::FontId::proportional(14.0 * s),
                            WOOD,
                        );
                    }
                }
                if resp.clicked() {
                    match (*picked, stack) {
                        (Some(from), _) if from == i => *picked = None,
                        (Some(from), _) => {
                            req(out, Request::MoveItem { from, to: i });
                            *picked = None;
                        }
                        (None, Some(_)) => *picked = Some(i),
                        (None, None) => {}
                    }
                }
                if (i + 1) % cols == 0 {
                    ui.end_row();
                }
            }
        });

    if let Some(i) = *picked {
        if let Some(st) = me.sac.get(i) {
            item_actions(ui, s, i, st, out, picked);
        }
    }

    style::section(ui, s, "Hands");
    hand_row(ui, s, "Food", me.food, Hand::Food, out);
    hand_row(ui, s, "Heal", me.heal, Hand::Heal, out);
}

fn item_actions(
    ui: &mut egui::Ui,
    s: f32,
    slot: usize,
    st: Stack,
    out: &mut Vec<UiAction>,
    picked: &mut Option<usize>,
) {
    ui.add_space(4.0 * s);
    let kind = st.kind;
    let mut facts = Vec::new();
    if let Some(h) = kind.hunger() {
        facts.push(format!("+{h:.0} hunger"));
    }
    if let Some(h) = kind.heal() {
        facts.push(format!("+{h:.0} health"));
    }
    if let Some(p) = kind.sell_price() {
        facts.push(format!("sells for {p}"));
    }
    row(
        ui,
        s,
        |ui| {
            ui.label(RichText::new(format!("{} x{}", kind.name(), st.count)).color(INK).strong());
            ui.label(style::note(facts.join(", "), s));
        },
        |_| {},
    );
    ui.horizontal_wrapped(|ui| {
        let mut done = None;
        if kind.hunger().is_some() && side_button(ui, s, "Equip as food", true) {
            done = Some(Request::Equip { slot, hand: Hand::Food });
        }
        if kind.heal().is_some() && side_button(ui, s, "Equip as heal", true) {
            done = Some(Request::Equip { slot, hand: Hand::Heal });
        }
        if matches!(kind, ItemKind::Fish(_)) && side_button(ui, s, "Cook", true) {
            done = Some(Request::Cook { slot: Some(slot) });
        }
        if kind == ItemKind::Flare && side_button(ui, s, "Light flare", true) {
            done = Some(Request::UseFlare { slot });
        }
        if kind.class() != ItemClass::Quest && side_button(ui, s, "Drop", true) {
            done = Some(Request::DropItem { slot });
        }
        if let Some(r) = done {
            req(out, r);
            *picked = None;
        }
    });
}

fn hand_row(ui: &mut egui::Ui, s: f32, name: &str, stack: Option<Stack>, hand: Hand, out: &mut Vec<UiAction>) {
    row(
        ui,
        s,
        |ui| {
            ui.label(RichText::new(format!("{name} hand")).color(MUTED));
            match stack {
                Some(st) => ui.label(RichText::new(format!("{} x{}", st.kind.name(), st.count)).color(INK).strong()),
                None => ui.label(style::note("empty: equip an item from the sac", s)),
            };
        },
        |ui| {
            if stack.is_some() {
                if side_button(ui, s, "Unequip", true) {
                    req(out, Request::Unequip(hand));
                }
                let (label, r) = match hand {
                    Hand::Food => ("Eat", Request::Eat),
                    Hand::Heal => ("Use", Request::Heal),
                };
                if side_button(ui, s, label, true) {
                    req(out, r);
                }
            }
        },
    );
}

// ---------------------------------------------------------------- workbench

fn workbench(
    ui: &mut egui::Ui,
    s: f32,
    world: &World,
    me: &Player,
    out: &mut Vec<UiAction>,
    close: &mut bool,
) {
    let stock = &world.economy.stock;
    let level = world.economy.campfire.level;
    ui.horizontal(|ui| {
        ui.label(RichText::new("Stockpile").color(MUTED));
        ui.label(
            RichText::new(format!(
                "{} wood   {} metal   {} biomass   {} oil",
                stock.wood, stock.metal, stock.biomass, stock.oil
            ))
            .color(INK)
            .strong(),
        );
    });
    for l in 1..=CAMPFIRE_MAX_LEVEL {
        let unlocked = l <= level;
        let heading = if unlocked {
            format!("Campfire level {l}")
        } else {
            format!("Campfire level {l}   locked")
        };
        style::section(ui, s, &heading);
        for r in RECIPES.iter().filter(|r| r.level == l) {
            let affordable = stock.covers(&r.cost);
            let owned = match r.output {
                RecipeOutput::Give(g) => can_buy(me, g).is_err(),
                _ => false,
            };
            let what = match r.output {
                RecipeOutput::Floor | RecipeOutput::Railing => "raft",
                RecipeOutput::Structure(_) => "placed on the raft",
                RecipeOutput::Boat(_) => "boat, at the dock",
                RecipeOutput::Give(content::Goods::Item(..)) => "into your sac",
                RecipeOutput::Give(_) => "gear",
            };
            row(
                ui,
                s,
                |ui| {
                    ui.label(RichText::new(r.name).color(if unlocked { INK } else { MUTED }).strong());
                    ui.label(style::note(what, s));
                    cost_text(ui, s, &r.cost, stock);
                },
                |ui| {
                    if !unlocked {
                        ui.label(style::note(format!("needs campfire L{l}"), s));
                    } else if owned {
                        ui.label(RichText::new("Owned").color(LEAF_INK));
                    } else if r.is_placed() {
                        if side_button(ui, s, "Place", affordable) {
                            out.push(UiAction::BeginPlacement(r.id));
                            *close = true;
                        }
                    } else if side_button(ui, s, "Craft", affordable) {
                        req(out, Request::Craft(r.id));
                    }
                },
            );
        }
    }
}

// ---------------------------------------------------------------- campfire

fn campfire(ui: &mut egui::Ui, s: f32, world: &World, me: &Player, out: &mut Vec<UiAction>) {
    let fire = &world.economy.campfire;
    let stock = &world.economy.stock;
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("Level {}", fire.level)).size(22.0 * s).color(INK).strong());
        ui.label(style::note(format!("light reaches {:.0} m", fire.light_radius()), s));
    });
    if fire.level < CAMPFIRE_MAX_LEVEL {
        let next = CAMPFIRE_THRESHOLDS[fire.level as usize - 1];
        let prev = if fire.level >= 2 { CAMPFIRE_THRESHOLDS[fire.level as usize - 2] } else { 0 };
        let frac = (fire.points.saturating_sub(prev)) as f32 / (next - prev).max(1) as f32;
        ui.label(RichText::new(format!("Upgrade points {} / {next} for level {}", fire.points, fire.level + 1)).color(MUTED));
        ui.add(
            egui::ProgressBar::new(frac.clamp(0.0, 1.0))
                .desired_height(14.0 * s)
                .fill(style::GOLD)
                .corner_radius(style::radius(5.0 * s)),
        );
        ui.label(style::note("Higher levels unlock workbench recipes and light more of the night.", s));
    } else {
        ui.label(RichText::new("Max level: the fire roars.").color(LEAF_INK));
    }
    ui.add_space(4.0 * s);
    let fuel = fire.fuel.max(0.0);
    let fuel_text = if fire.lit() {
        format!("Fuel {}:{:02}", fuel as u32 / 60, fuel as u32 % 60)
    } else {
        "Out: feed it before night".to_string()
    };
    ui.label(RichText::new(fuel_text).color(if fire.lit() { INK } else { RED_INK }).strong());
    ui.add(
        egui::ProgressBar::new((fuel / FUEL_CAP).clamp(0.0, 1.0))
            .desired_height(14.0 * s)
            .fill(style::AMBER)
            .corner_radius(style::radius(5.0 * s)),
    );

    style::section(ui, s, "Feed");
    ui.label(style::note(format!("Stockpile: {} wood, {} oil", stock.wood, stock.oil), s));
    let feeds = [
        (false, format!("{FEED_WOOD} wood"), stock.wood >= FEED_WOOD, FEED_WOOD * POINTS_PER_WOOD, FEED_WOOD as f32 * FUEL_PER_WOOD),
        (true, format!("{FEED_OIL} oil"), stock.oil >= FEED_OIL, FEED_OIL * POINTS_PER_OIL, FEED_OIL as f32 * FUEL_PER_OIL),
    ];
    for (oil, what, can, points, secs) in feeds {
        row(
            ui,
            s,
            |ui| {
                ui.label(RichText::new(what).color(INK).strong());
                ui.label(style::note(format!("+{points} points, +{secs:.0} s fuel"), s));
            },
            |ui| {
                if side_button(ui, s, "Feed", can) {
                    req(out, Request::FeedCampfire { oil });
                }
            },
        );
    }

    style::section(ui, s, "Cook fish");
    let mut raw: Vec<(Option<usize>, Stack)> = me
        .sac
        .stacks()
        .filter(|(_, st)| matches!(st.kind, ItemKind::Fish(_)))
        .map(|(i, st)| (Some(i), st))
        .collect();
    if let Some(st) = me.food.filter(|st| matches!(st.kind, ItemKind::Fish(_))) {
        raw.insert(0, (None, st));
    }
    if raw.is_empty() {
        ui.label(style::note("No raw fish on you. Catch some with the rod.", s));
    }
    for (slot, st) in raw {
        row(
            ui,
            s,
            |ui| {
                ui.label(RichText::new(format!("{} x{}", st.kind.name(), st.count)).color(INK).strong());
                if slot.is_none() {
                    ui.label(style::note("food hand", s));
                }
            },
            |ui| {
                if side_button(ui, s, "Cook", fire.lit()) {
                    req(out, Request::Cook { slot });
                }
            },
        );
    }
}

// ---------------------------------------------------------------- quests

fn quest_progress(world: &World, q: QuestId) -> Option<String> {
    let def = q.def();
    let progress = world.quests.entries.get(q as usize).map_or(0, |e| e.progress);
    match def.goal {
        QuestGoal::SellFish(n) => Some(format!("{} / {n} sold", progress.min(n))),
        QuestGoal::GeneratorMetal(n) => Some(format!("{} / {n} metal", progress.min(n))),
        QuestGoal::Shrines(n) => {
            let lit = world.islands.get(def.giver).map_or(0, |i| i.shrines.iter().filter(|b| **b).count());
            Some(format!("{lit} / {n} shrines"))
        }
        QuestGoal::CampfireLevel(l) => Some(format!("campfire {} / {l}", world.economy.campfire.level.min(l))),
        _ => None,
    }
}

/// What the claim button would do for this quest: `Ok(true)` claim here, `Ok(false)`
/// nothing to claim, `Err(hint)` where to claim instead.
fn claim_here(world: &World, me: &Player, q: QuestId, at_barnaby: bool) -> Result<bool, String> {
    let def = q.def();
    let status = world.quests.status(q);
    let ready = match def.goal {
        QuestGoal::BringItem(item) => status == QuestStatus::Active && me.sac.count(item) > 0,
        _ => status == QuestStatus::Ready,
    };
    if !ready {
        return Ok(false);
    }
    let ending = def.goal == QuestGoal::Ending;
    if def.reward.is_empty() {
        let island = content::island(def.giver).name;
        return Err(format!("Open the chest on {island}"));
    }
    if ending || at_barnaby {
        Ok(true)
    } else {
        Err("Claim it at Barnaby".to_string())
    }
}

fn quest_rows(ui: &mut egui::Ui, s: f32, world: &World, me: &Player, quests: &[QuestId], at_barnaby: bool, out: &mut Vec<UiAction>) {
    let mut hidden = 0;
    for &q in quests {
        let def = q.def();
        let status = world.quests.status(q);
        if status == QuestStatus::Locked {
            hidden += 1;
            continue;
        }
        let island = content::island(def.giver).name;
        row(
            ui,
            s,
            |ui| {
                ui.vertical(|ui| {
                    let color = if status == QuestStatus::Done { MUTED } else { INK };
                    ui.label(RichText::new(format!("{}. {}", def.number, def.title)).color(color).strong());
                    let mut info = vec![island.to_string()];
                    if status != QuestStatus::Done {
                        info.extend(quest_progress(world, q));
                    }
                    ui.label(style::note(info.join("  ·  "), s));
                });
            },
            |ui| match claim_here(world, me, q, at_barnaby) {
                Ok(true) => {
                    let b = egui::Button::new(RichText::new("Claim").size(16.0 * s).color(CREAM))
                        .fill(WOOD)
                        .min_size(Vec2::new(86.0 * s, 30.0 * s));
                    if ui.add(b).clicked() {
                        req(out, Request::ClaimQuest(q));
                    }
                }
                Err(hint) => {
                    ui.label(RichText::new(hint).size(14.0 * s).color(SEA));
                }
                Ok(false) => {
                    let (text, color) = match status {
                        QuestStatus::Locked => ("Locked", MUTED),
                        QuestStatus::Active => ("Active", WOOD),
                        QuestStatus::Ready => ("Ready", LEAF_INK),
                        QuestStatus::Done => ("Done", LEAF_INK),
                    };
                    ui.label(RichText::new(text).color(color).strong());
                }
            },
        );
    }
    if hidden > 0 {
        let what = if hidden == 1 { "quest" } else { "quests" };
        ui.label(style::note(format!("{hidden} more {what} not found yet: explore the islands."), s));
    }
}

fn logbook(ui: &mut egui::Ui, s: f32, frame: &Frame, me: &Player, out: &mut Vec<UiAction>) {
    let world = frame.world;
    style::section(ui, s, "Save the world");
    let cost = content::save_cost(world.economy.campfire.level);
    ui.label(style::note(
        "Saving costs coins and grows with the campfire level. Without a save, game over \
         sends you back to the main menu.",
        s,
    ));
    row(
        ui,
        s,
        |ui| {
            ui.label(RichText::new(format!("{cost} coins")).color(INK).strong());
            let color = if me.coins >= cost { LEAF_INK } else { RED_INK };
            ui.label(RichText::new(format!("you have {}", me.coins)).size(15.0 * s).color(color));
        },
        |ui| {
            let b = egui::Button::new(RichText::new("Save world").size(16.0 * s).color(CREAM))
                .fill(WOOD)
                .min_size(Vec2::new(120.0 * s, 32.0 * s));
            if ui.add_enabled(me.coins >= cost, b).clicked() {
                req(out, Request::SaveWorld);
            }
        },
    );
    if !frame.is_host {
        ui.label(style::note("The host keeps the save file.", s));
    }
    style::section(ui, s, "Quests");
    quest_rows(ui, s, world, me, &QuestId::ALL, false, out);
}

// ---------------------------------------------------------------- shops

fn offer_rows(ui: &mut egui::Ui, s: f32, me: &Player, shop: Shop, offers: &[Offer], out: &mut Vec<UiAction>) {
    for (i, offer) in offers.iter().enumerate() {
        let owned = can_buy(me, offer.goods).is_err();
        let afford = me.coins >= offer.price;
        row(
            ui,
            s,
            |ui| {
                ui.label(RichText::new(offer.goods.label()).color(INK).strong());
                let color = match (owned, afford) {
                    (true, _) => MUTED,
                    (false, true) => WOOD,
                    (false, false) => RED_INK,
                };
                ui.label(RichText::new(format!("{} coins", offer.price)).size(15.0 * s).color(color));
            },
            |ui| {
                if owned {
                    ui.label(RichText::new("Owned").color(LEAF_INK));
                } else if side_button(ui, s, "Buy", afford) {
                    req(out, Request::Buy { shop, offer: i });
                }
            },
        );
    }
}

fn fisherman(ui: &mut egui::Ui, s: f32, world: &World, me: &Player, out: &mut Vec<UiAction>) {
    coins_line(ui, s, me.coins);
    // Two columns: a full sac of fish must not push the shop and the jobs out of sight.
    ui.columns(2, |cols| {
        let [left, right] = cols else { return };
        sell_fish(left, s, me, out);
        style::section(right, s, "Buy");
        offer_rows(right, s, me, Shop::Barnaby, &BARNABY_STOCK, out);
        style::section(right, s, "Barnaby's jobs");
        quest_rows(right, s, world, me, &QuestId::ALL[..3], true, out);
    });
}

fn sell_fish(ui: &mut egui::Ui, s: f32, me: &Player, out: &mut Vec<UiAction>) {
    style::section(ui, s, "Sell fish");
    let fish: Vec<(usize, Stack, u32)> = me
        .sac
        .stacks()
        .filter(|(_, st)| st.kind.is_fish())
        .filter_map(|(i, st)| st.kind.sell_price().map(|p| (i, st, p)))
        .collect();
    if fish.is_empty() {
        ui.label(style::note("No fish in your sac. Barnaby buys raw and cooked fish.", s));
    }
    let total: u32 = fish.iter().map(|(_, st, p)| st.count * p).sum();
    for (slot, st, price) in &fish {
        row(
            ui,
            s,
            |ui| {
                ui.label(RichText::new(format!("{} x{}", st.kind.name(), st.count)).color(INK).strong());
                ui.label(style::note(format!("{price} each"), s));
            },
            |ui| {
                if side_button(ui, s, &format!("Sell {}", st.count * price), true) {
                    req(out, Request::Sell { slot: *slot });
                }
            },
        );
    }
    if fish.len() > 1 {
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if side_button(ui, s, &format!("Sell all fish  ({total})"), true) {
                    req(out, Request::SellAllFish);
                }
            });
        });
    }
}

fn trader(ui: &mut egui::Ui, s: f32, world: &World, me: &Player, out: &mut Vec<UiAction>) {
    let t = &world.trader;
    if !t.present {
        ui.label(RichText::new("The trader's boat is away.").color(INK).strong());
        ui.label(style::note(format!("It docks at the raft every {TRADER_EVERY_DAYS} days."), s));
        return;
    }
    coins_line(ui, s, me.coins);
    ui.label(style::note("Rare goods, this visit only.", s));
    offer_rows(ui, s, me, Shop::Trader, &t.stock, out);
}

// ---------------------------------------------------------------- radio

/// "NE" for a heading from `from` to `to` (N is -Z, E is +X).
fn heading(dx: f32, dz: f32) -> &'static str {
    const NAMES: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    let a = dx.atan2(-dz).to_degrees().rem_euclid(360.0);
    NAMES[((a + 22.5) / 45.0) as usize % 8]
}

fn radio(ui: &mut egui::Ui, s: f32, world: &World, me: &Player) {
    if world.raft.count(PlaceableKind::RadioStation) == 0 {
        ui.label(style::note("Build a Radio Station to pick up survivor signals.", s));
        return;
    }
    let mut any = false;
    for (id, st) in world.islands.list.iter().enumerate() {
        let def = content::island(id as u8);
        match st.survivor {
            SurvivorState::Waiting => {
                any = true;
                let at = def.spot(Spot::Survivor);
                let (dx, dz) = (at.x - me.pos.x, at.z - me.pos.z);
                let dist = (dx * dx + dz * dz).sqrt();
                row(
                    ui,
                    s,
                    |ui| {
                        ui.label(RichText::new(format!("Signal from {}", def.name)).color(INK).strong());
                        ui.label(style::note(format!("{dist:.0} m {}", heading(dx, dz)), s));
                    },
                    |ui| {
                        ui.label(RichText::new("Waiting").color(SEA));
                    },
                );
            }
            SurvivorState::Rescued => {
                any = true;
                row(
                    ui,
                    s,
                    |ui| {
                        ui.label(RichText::new(def.name).color(MUTED));
                    },
                    |ui| {
                        ui.label(RichText::new("Rescued").color(LEAF_INK));
                    },
                );
            }
            SurvivorState::Hidden => {}
        }
    }
    if !any {
        ui.label(style::note("Only static. Keep the radio on and sail further out.", s));
    } else {
        ui.label(style::note("Rescued survivors live on the raft and speed up your farms.", s));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_follow_the_compass() {
        assert_eq!(heading(0.0, -1.0), "N");
        assert_eq!(heading(1.0, 0.0), "E");
        assert_eq!(heading(0.0, 1.0), "S");
        assert_eq!(heading(-1.0, -1.0), "NW");
    }

    /// Writes tests/fixtures/ui_world.json, a save that fills every window (tests/ui_filled.json):
    /// `cargo test --release --bin rustingraft ui::windows::tests::write_ui_world -- --ignored`
    #[test]
    #[ignore]
    fn write_ui_world() {
        use raft_sim::content::{Fish, TRADER_POOL};
        use raft_sim::raft::Structure;
        let mut w = World::new(7);
        w.clock.elapsed = 360.0 + 60.0; // day 2, morning: the trader is in
        w.clock.day = 2;
        w.clock.time = 60.0;
        let id = w.join("Sailor").unwrap();
        let p = w.player_mut(id).unwrap();
        p.coins = 95;
        p.sac.upgrade(16);
        for (kind, n) in [
            (ItemKind::Fish(Fish::Sardine), 4),
            (ItemKind::Fish(Fish::Mackerel), 2),
            (ItemKind::Fish(Fish::Goldfin), 1),
            (ItemKind::Fish(Fish::AbyssalKoi), 1),
            (ItemKind::CookedFish(Fish::Sardine), 3),
            (ItemKind::CookedFish(Fish::Moonray), 1),
            (ItemKind::Bandage, 2),
            (ItemKind::BiomassSalve, 1),
            (ItemKind::Bait, 5),
            (ItemKind::Flare, 1),
            (ItemKind::Plank, 6),
            (ItemKind::ScrapChunk, 3),
            (ItemKind::PalmBundle, 2),
            (ItemKind::OilBarrel, 1),
            (ItemKind::GloomfinCarcass, 1),
        ] {
            assert_eq!(p.sac.add(kind, n), 0, "{kind:?} fits");
        }
        p.food = Some(Stack::new(ItemKind::CookedFish(Fish::Mackerel), 2));
        p.heal = Some(Stack::new(ItemKind::Bandage, 1));
        let e = &mut w.economy;
        e.stock = Stockpile { wood: 64, metal: 5, biomass: 6, oil: 1 };
        e.campfire.level = 3;
        e.campfire.points = 60;
        e.campfire.fuel = 200.0;
        let cell = (-3..4)
            .flat_map(|x| (-3..4).map(move |z| (x, z)))
            .find(|&c| w.raft.can_place(PlaceableKind::RadioStation, c, 0).is_ok())
            .expect("room for a radio");
        let rid = w.alloc_id();
        w.raft.structures.insert(rid, Structure::new(PlaceableKind::RadioStation, cell, 0));
        let mut rescued = false;
        for (i, st) in w.islands.list.iter_mut().enumerate() {
            if content::island(i as u8).survivor {
                st.survivor = if rescued { SurvivorState::Waiting } else { SurvivorState::Rescued };
                if !rescued {
                    w.economy.survivors.push(i as u8);
                    rescued = true;
                }
            }
        }
        for (q, status, progress) in [
            (0, QuestStatus::Done, 3),
            (1, QuestStatus::Ready, 0),
            (2, QuestStatus::Active, 0),
            (3, QuestStatus::Active, 0),
            (4, QuestStatus::Active, 12),
        ] {
            w.quests.entries[q].status = status;
            w.quests.entries[q].progress = progress;
        }
        w.trader.present = true;
        w.trader.stock = [0, 3, 5, 6, 8].map(|i| TRADER_POOL[i]).to_vec();
        w.trader.dock = w.raft.trader_dock_pos();
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/ui_world.json");
        std::fs::write(path, w.to_json()).unwrap();
        assert!(World::load(&w.to_json()).is_ok());
    }
}
