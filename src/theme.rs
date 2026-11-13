//! Look and motion: colours, widget styling, easing curves and small paint helpers.

use eframe::egui::{self, Color32, CornerRadius, FontId, Margin, Rect, Shape, Stroke, TextStyle, Vec2};

pub const BG: Color32 = Color32::from_rgb(12, 14, 19);
pub const PANEL: Color32 = Color32::from_rgb(18, 21, 28);
pub const CARD: Color32 = Color32::from_rgb(26, 30, 40);
pub const RAISED: Color32 = Color32::from_rgb(36, 41, 55);
pub const BORDER: Color32 = Color32::from_rgb(44, 50, 66);
pub const TEXT: Color32 = Color32::from_rgb(224, 228, 238);
pub const MUTED: Color32 = Color32::from_rgb(134, 143, 164);
pub const ACCENT: Color32 = Color32::from_rgb(122, 139, 255);
pub const WARN: Color32 = Color32::from_rgb(240, 178, 76);
pub const DANGER: Color32 = Color32::from_rgb(242, 100, 110);
pub const OK: Color32 = Color32::from_rgb(92, 208, 150);

/// `color` at `a` opacity (0..1).
pub fn alpha(color: Color32, a: f32) -> Color32 {
    color.gamma_multiply(a.clamp(0.0, 1.0))
}

pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    let v = &mut style.visuals;
    v.panel_fill = PANEL;
    v.window_fill = CARD;
    v.extreme_bg_color = Color32::from_rgb(30, 34, 46);
    v.faint_bg_color = Color32::from_rgb(22, 25, 33);
    v.code_bg_color = BG;
    v.hyperlink_color = ACCENT;
    v.warn_fg_color = WARN;
    v.error_fg_color = DANGER;
    v.window_stroke = Stroke::new(1.0_f32, BORDER);
    v.window_corner_radius = CornerRadius::same(14);
    v.menu_corner_radius = CornerRadius::same(10);
    v.window_shadow = egui::Shadow { offset: [0, 14], blur: 40, spread: 0, color: Color32::from_black_alpha(140) };
    v.popup_shadow = egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(120) };
    v.selection.bg_fill = alpha(ACCENT, 0.26);
    v.selection.stroke = Stroke::new(1.0_f32, TEXT);

    let radius = CornerRadius::same(7);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = PANEL;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    w.inactive.bg_fill = RAISED;
    w.inactive.weak_bg_fill = RAISED;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    w.hovered.bg_fill = Color32::from_rgb(50, 57, 78);
    w.hovered.weak_bg_fill = Color32::from_rgb(50, 57, 78);
    w.hovered.bg_stroke = Stroke::new(1.0_f32, alpha(ACCENT, 0.7));
    w.hovered.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
    w.active.bg_fill = alpha(ACCENT, 0.55);
    w.active.weak_bg_fill = alpha(ACCENT, 0.55);
    w.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    w.active.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
    w.open.bg_fill = Color32::from_rgb(44, 50, 70);
    w.open.weak_bg_fill = Color32::from_rgb(44, 50, 70);
    w.open.bg_stroke = Stroke::new(1.0_f32, alpha(ACCENT, 0.7));
    w.open.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
    for visuals in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        visuals.corner_radius = radius;
        visuals.expansion = 0.0;
    }

    let s = &mut style.spacing;
    s.item_spacing = Vec2::new(8.0, 6.0);
    s.button_padding = Vec2::new(10.0, 5.0);
    s.interact_size.y = 26.0;
    s.menu_margin = Margin::same(6);
    s.window_margin = Margin::same(14);
    s.scroll.bar_width = 8.0;
    s.scroll.floating = true;

    style.text_styles.insert(TextStyle::Body, FontId::proportional(14.0));
    style.text_styles.insert(TextStyle::Button, FontId::proportional(14.0));
    style.text_styles.insert(TextStyle::Small, FontId::proportional(11.5));
    style.text_styles.insert(TextStyle::Heading, FontId::proportional(20.0));
    style.text_styles.insert(TextStyle::Monospace, FontId::monospace(13.0));

    // Row clicks select entries, so labels must not swallow them for text selection.
    style.interaction.selectable_labels = false;
    style.animation_time = 0.16;
    ctx.set_style(style);
}

/// Colour that identifies a content type in the table.
pub fn kind_color(kind: &str) -> Color32 {
    match kind {
        kind if crate::format::is_image(kind) => Color32::from_rgb(86, 204, 170),
        kind if crate::format::is_audio(kind) => Color32::from_rgb(244, 174, 92),
        "mesh" => Color32::from_rgb(104, 170, 255),
        "rbxm" | "rbxmx" => Color32::from_rgb(178, 142, 255),
        "webm" | "mp4" => Color32::from_rgb(246, 128, 190),
        "ttf" | "otf" => Color32::from_rgb(110, 214, 232),
        "missing" | "empty" => DANGER,
        _ => MUTED,
    }
}

/// A small rounded label tinted with `color`.
pub fn pill(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::new().fill(alpha(color, 0.16)).corner_radius(CornerRadius::same(6)).inner_margin(Margin::symmetric(7, 1)).show(ui, |ui| {
        ui.label(egui::RichText::new(text).color(color).size(12.5));
    });
}

pub fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// Ease-out that overshoots slightly before settling.
pub fn ease_out_back(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0) - 1.0;
    1.0 + 2.2 * t * t * t + 1.2 * t * t
}

/// Moves `current` toward `target`; larger `rate` is snappier. Frame-rate independent.
pub fn approach(current: f32, target: f32, rate: f32, dt: f32) -> f32 {
    current + (target - current) * (1.0 - (-rate * dt).exp())
}

/// Fading copies of `rect` smeared along `streak` (which points away from the direction of travel).
pub fn motion_trail(rect: Rect, streak: Vec2, color: Color32, radius: CornerRadius, strength: f32) -> Shape {
    const COPIES: usize = 5;
    let ghosts = (1..=COPIES)
        .map(|k| {
            let f = k as f32 / COPIES as f32;
            Shape::rect_filled(rect.translate(streak * f), radius, alpha(color, strength * (1.0 - f) * 0.5))
        })
        .collect();
    Shape::Vec(ghosts)
}
