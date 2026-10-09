//! Look of every menu, window and HUD element: warm sand panels with driftwood
//! borders over the ocean, teal for "selected", coral for danger.
//!
//! egui styling in code: `assets/ui/theme.json` belongs to the scene/assets side and
//! cannot size fonts. Everything scales with the screen height (see `scale`).

use rusting_engine::egui::{
    self, Align2, Color32, CornerRadius, FontId, Margin, Pos2, Rect, RichText, Shadow, Stroke,
    TextStyle, Vec2, Visuals,
};

pub const SAND: Color32 = Color32::from_rgb(241, 227, 198);
pub const SAND_DEEP: Color32 = Color32::from_rgb(225, 204, 163);
pub const CREAM: Color32 = Color32::from_rgb(253, 247, 234);
pub const WOOD: Color32 = Color32::from_rgb(122, 80, 45);
pub const INK: Color32 = Color32::from_rgb(58, 38, 22);
pub const MUTED: Color32 = Color32::from_rgb(128, 104, 80);
pub const TAN: Color32 = Color32::from_rgb(216, 184, 136);
pub const TAN_HOVER: Color32 = Color32::from_rgb(229, 202, 158);
pub const TAN_ACTIVE: Color32 = Color32::from_rgb(197, 163, 114);
pub const SEA: Color32 = Color32::from_rgb(34, 136, 142);
pub const CORAL: Color32 = Color32::from_rgb(222, 92, 70);
pub const RED_INK: Color32 = Color32::from_rgb(160, 44, 30);
pub const AMBER: Color32 = Color32::from_rgb(242, 170, 70);
pub const GOLD: Color32 = Color32::from_rgb(244, 204, 92);
pub const LEAF: Color32 = Color32::from_rgb(108, 176, 82);
pub const LEAF_INK: Color32 = Color32::from_rgb(52, 112, 40);

/// HUD chips: dark driftwood, see-through.
pub fn hud_bg() -> Color32 {
    Color32::from_rgba_unmultiplied(30, 21, 14, 178)
}

/// UI scale for this screen: 1.0 at 1280 x 720.
pub fn scale(ctx: &egui::Context) -> f32 {
    let size = ctx.screen_rect().size();
    (size.y / 720.0).min(size.x / 1280.0).clamp(0.6, 3.0)
}

pub fn apply(ctx: &egui::Context, s: f32) {
    let mut style = (*ctx.style()).clone();
    style.text_styles = [
        (TextStyle::Small, FontId::proportional(13.0 * s)),
        (TextStyle::Body, FontId::proportional(16.0 * s)),
        (TextStyle::Button, FontId::proportional(17.0 * s)),
        (TextStyle::Heading, FontId::proportional(25.0 * s)),
        (TextStyle::Monospace, FontId::monospace(14.0 * s)),
    ]
    .into();
    let sp = &mut style.spacing;
    sp.item_spacing = Vec2::new(8.0, 7.0) * s;
    sp.button_padding = Vec2::new(12.0, 5.0) * s;
    sp.interact_size = Vec2::new(40.0, 26.0) * s;
    sp.slider_width = 190.0 * s;
    sp.slider_rail_height = 6.0 * s;
    sp.text_edit_width = 200.0 * s;
    sp.icon_width = 18.0 * s;
    sp.icon_width_inner = 10.0 * s;
    sp.icon_spacing = 6.0 * s;
    sp.window_margin = margin(16.0 * s);
    // Solid bars: long windows (workbench, logbook) show that they scroll.
    sp.scroll = egui::style::ScrollStyle::solid();
    sp.scroll.bar_width = 8.0 * s;

    let mut v = Visuals::light();
    v.window_fill = SAND;
    v.panel_fill = SAND;
    v.window_stroke = Stroke::new(2.0 * s, WOOD);
    v.window_corner_radius = radius(12.0 * s);
    v.window_shadow = Shadow {
        offset: [0, (6.0 * s) as i8],
        blur: (18.0 * s) as u8,
        spread: 0,
        color: Color32::from_black_alpha(90),
    };
    v.popup_shadow = v.window_shadow;
    v.window_highlight_topmost = false;
    v.extreme_bg_color = CREAM;
    v.faint_bg_color = SAND_DEEP;
    v.selection.bg_fill = SEA;
    v.selection.stroke = Stroke::new(1.5 * s, CREAM);
    v.hyperlink_color = SEA;
    v.warn_fg_color = Color32::from_rgb(170, 100, 20);
    v.error_fg_color = RED_INK;
    v.slider_trailing_fill = true;
    let r = radius(7.0 * s);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = SAND;
    w.noninteractive.weak_bg_fill = SAND;
    w.noninteractive.bg_stroke = Stroke::new(1.0 * s, WOOD.gamma_multiply(0.45));
    w.noninteractive.fg_stroke = Stroke::new(1.0 * s, INK);
    w.noninteractive.corner_radius = r;
    for (wv, fill) in [
        (&mut w.inactive, TAN),
        (&mut w.hovered, TAN_HOVER),
        (&mut w.active, TAN_ACTIVE),
        (&mut w.open, TAN),
    ] {
        wv.bg_fill = fill;
        wv.weak_bg_fill = fill;
        wv.bg_stroke = Stroke::new(1.0 * s, WOOD);
        wv.fg_stroke = Stroke::new(1.5 * s, INK);
        wv.corner_radius = r;
        wv.expansion = 0.0;
    }
    w.hovered.bg_stroke = Stroke::new(1.5 * s, INK);
    w.hovered.expansion = 1.0 * s;
    style.visuals = v;
    ctx.set_style(style);
}

pub fn margin(px: f32) -> Margin {
    Margin::same(px.clamp(0.0, 127.0) as i8)
}

pub fn radius(px: f32) -> CornerRadius {
    CornerRadius::same(px.clamp(0.0, 255.0) as u8)
}

/// Window frame: sand with a driftwood border.
pub fn window_frame(ctx: &egui::Context) -> egui::Frame {
    egui::Frame::window(&ctx.style())
}

/// A sunken row inside a window.
pub fn row_frame(s: f32) -> egui::Frame {
    egui::Frame::new()
        .fill(SAND_DEEP.gamma_multiply(0.55))
        .corner_radius(radius(7.0 * s))
        .inner_margin(Margin::symmetric((10.0 * s) as i8, (6.0 * s) as i8))
}

/// The main-menu ocean behind the cards: deep blue at the top to teal at the bottom.
pub fn backdrop(ctx: &egui::Context) {
    let rect = ctx.screen_rect();
    let painter = ctx.layer_painter(egui::LayerId::background());
    let top = Color32::from_rgba_unmultiplied(10, 34, 54, 235);
    let bottom = Color32::from_rgba_unmultiplied(22, 104, 116, 215);
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 2, 3);
    painter.add(mesh);
}

/// Darkens everything below the windows (pause, game over). Paints on the background
/// layer after the HUD, so call it after the HUD is drawn.
pub fn dim(ctx: &egui::Context, color: Color32) {
    ctx.layer_painter(egui::LayerId::background())
        .rect_filled(ctx.screen_rect(), 0.0, color);
}

/// A full-width button; `primary` fills it with driftwood and cream text.
pub fn big_button(ui: &mut egui::Ui, s: f32, label: &str, primary: bool, enabled: bool) -> bool {
    let text = RichText::new(label).size(19.0 * s);
    let button = if primary {
        egui::Button::new(text.color(CREAM)).fill(WOOD)
    } else {
        egui::Button::new(text)
    };
    let size = Vec2::new(ui.available_width(), 40.0 * s);
    ui.add_enabled(enabled, button.min_size(size)).clicked()
}

/// A danger button (leave, quit).
pub fn coral_button(ui: &mut egui::Ui, s: f32, label: &str) -> bool {
    let text = RichText::new(label).size(18.0 * s).color(CREAM);
    ui.add(
        egui::Button::new(text)
            .fill(CORAL)
            .min_size(Vec2::new(150.0 * s, 38.0 * s)),
    )
    .clicked()
}

/// A small keyboard key cap such as `E` or `Tab`.
pub fn key_cap(ui: &mut egui::Ui, s: f32, key: &str) {
    egui::Frame::new()
        .fill(CREAM)
        .stroke(Stroke::new(1.0 * s, WOOD))
        .corner_radius(radius(4.0 * s))
        .inner_margin(Margin::symmetric((6.0 * s) as i8, (1.0 * s) as i8))
        .show(ui, |ui| {
            ui.label(RichText::new(key).size(14.0 * s).color(INK).strong());
        });
}

/// Section title inside a window.
pub fn section(ui: &mut egui::Ui, s: f32, text: &str) {
    ui.add_space(4.0 * s);
    ui.label(RichText::new(text).size(18.0 * s).color(WOOD).strong());
}

/// Muted small text.
pub fn note(text: impl Into<String>, s: f32) -> RichText {
    RichText::new(text).size(14.0 * s).color(MUTED)
}

/// Paints a key cap on the HUD and returns its width.
pub fn paint_key(painter: &egui::Painter, s: f32, left_center: Pos2, key: &str) -> f32 {
    let font = FontId::proportional(15.0 * s);
    let galley = painter.layout_no_wrap(key.to_string(), font, INK);
    let size = Vec2::new(
        (galley.size().x + 12.0 * s).max(24.0 * s),
        galley.size().y + 4.0 * s,
    );
    let rect = Rect::from_min_size(left_center - Vec2::new(0.0, size.y / 2.0), size);
    painter.rect_filled(rect, radius(4.0 * s), CREAM);
    painter.rect_stroke(
        rect,
        radius(4.0 * s),
        Stroke::new(1.0 * s, WOOD),
        egui::StrokeKind::Inside,
    );
    painter.galley(
        rect.center() - galley.size() / 2.0,
        galley,
        INK,
    );
    size.x
}

/// Text with a soft shadow, for HUD text drawn straight on the world.
pub fn shadow_text(
    painter: &egui::Painter,
    pos: Pos2,
    anchor: Align2,
    text: &str,
    font: FontId,
    color: Color32,
) -> Rect {
    let shadow = Color32::from_black_alpha((color.a() as f32 * 0.7) as u8);
    painter.text(pos + Vec2::splat(1.5), anchor, text, font.clone(), shadow);
    painter.text(pos, anchor, text, font, color)
}
