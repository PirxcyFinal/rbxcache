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

/// The splash screen and the icon use the palette of the pirxcy site rather than the app's.
pub const SPLASH_BG: Color32 = Color32::from_rgb(8, 8, 10);
/// The deep purple at the bottom of the splash gradient.
pub const SPLASH_PURPLE: Color32 = Color32::from_rgb(42, 18, 72);
/// The wordmark stays white on every splash, whatever the gradient does underneath.
pub const SPLASH_INK: Color32 = Color32::WHITE;
pub const SPLASH_MUTED: Color32 = Color32::from_rgb(168, 156, 196);
/// The thin progress line on the splash: soft purple, not the lime of the rest of the app.
pub const SPLASH_BAR: Color32 = Color32::from_rgb(148, 112, 255);
pub const LIME: Color32 = Color32::from_rgb(200, 255, 46);

/// Fills `rect` with a quiet black-to-purple gradient, top to bottom.
pub fn splash_gradient(painter: &egui::Painter, rect: Rect, opacity: f32) {
    const STEPS: i32 = 40;
    let height = rect.height().max(1.0);
    for i in 0..STEPS {
        let t0 = i as f32 / STEPS as f32;
        let t1 = (i + 1) as f32 / STEPS as f32;
        let band = Rect::from_min_max(egui::pos2(rect.left(), rect.top() + height * t0), egui::pos2(rect.right(), rect.top() + height * t1));
        // Ease toward purple so the top stays near black and the colour only shows in the lower half.
        let weight = t0 * t0;
        let color = Color32::from_rgb(
            (SPLASH_BG.r() as f32 + (SPLASH_PURPLE.r() as f32 - SPLASH_BG.r() as f32) * weight) as u8,
            (SPLASH_BG.g() as f32 + (SPLASH_PURPLE.g() as f32 - SPLASH_BG.g() as f32) * weight) as u8,
            (SPLASH_BG.b() as f32 + (SPLASH_PURPLE.b() as f32 - SPLASH_BG.b() as f32) * weight) as u8,
        );
        painter.rect_filled(band, CornerRadius::ZERO, alpha(color, opacity));
    }
}

/// `color` at `a` opacity (0..1).
pub fn alpha(color: Color32, a: f32) -> Color32 {
    color.gamma_multiply(a.clamp(0.0, 1.0))
}

/// Manrope ExtraBold; see assets/Manrope-OFL.txt.
const MANROPE: &[u8] = include_bytes!("../assets/Manrope-ExtraBold.ttf");

/// The app's icon as square RGBA pixels: a lime "p" on a dark rounded tile.
pub fn app_icon(size: u32) -> Vec<u8> {
    use ab_glyph::{Font, FontRef, PxScale};
    let tile = [SPLASH_BG.r(), SPLASH_BG.g(), SPLASH_BG.b()];
    let ink = [LIME.r() as f32, LIME.g() as f32, LIME.b() as f32];
    let (side, radius) = (size as f32, size as f32 * 0.22);
    let mut pixels = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            // Distance from the nearest corner's centre decides whether the pixel is on the tile.
            let (cx, cy) = ((x as f32 + 0.5).clamp(radius, side - radius), (y as f32 + 0.5).clamp(radius, side - radius));
            let distance = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            let coverage = (radius - distance + 0.5).clamp(0.0, 1.0);
            let at = ((y * size + x) * 4) as usize;
            pixels[at..at + 4].copy_from_slice(&[tile[0], tile[1], tile[2], (coverage * 255.0) as u8]);
        }
    }
    let glyph = FontRef::try_from_slice(MANROPE).ok().and_then(|font| {
        let glyph = font.glyph_id('p').with_scale(PxScale::from(side * 0.92));
        font.outline_glyph(glyph)
    });
    if let Some(outline) = glyph {
        let bounds = outline.px_bounds();
        let (left, top) = ((side - bounds.width()) / 2.0, (side - bounds.height()) / 2.0);
        outline.draw(|x, y, coverage| {
            let (px, py) = ((x as f32 + left) as u32, (y as f32 + top) as u32);
            if px < size && py < size {
                let at = ((py * size + px) * 4) as usize;
                for k in 0..3 {
                    pixels[at + k] = (pixels[at + k] as f32 * (1.0 - coverage) + ink[k] * coverage) as u8;
                }
            }
        });
    }
    pixels
}

/// The font family of the "pirxcy" wordmark on the splash screen.
pub fn wordmark_font() -> egui::FontFamily {
    egui::FontFamily::Name("wordmark".into())
}

pub fn apply(ctx: &egui::Context) {
    ctx.add_font(egui::epaint::text::FontInsert::new(
        "Manrope-ExtraBold",
        egui::FontData::from_static(MANROPE),
        vec![egui::epaint::text::InsertFontFamily { family: wordmark_font(), priority: egui::epaint::text::FontPriority::Highest }],
    ));
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
