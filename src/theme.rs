//! Look and motion: colours, widget styling, easing curves and small paint helpers.

use eframe::egui::{self, Color32, CornerRadius, FontId, Margin, Rect, Shape, Stroke, TextStyle, Vec2};

// Black and purple. The window's backdrop is black at the top with purple coming up from
// the bottom (see `backdrop`), and the bars, side panels and cards are see-through, so that
// purple shows through them.

/// Solid black, for boxes that must hide what is behind them.
pub const BG: Color32 = Color32::from_rgb(8, 7, 12);
/// The bars and side panels: a veil of black over the backdrop.
pub const PANEL: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 96);
/// A card on a page: dark purple glass.
pub const CARD: Color32 = Color32::from_rgba_premultiplied(15, 11, 25, 188);
/// Things that float over the page (menus, dialogs, toasts) and so must be solid.
pub const SURFACE: Color32 = Color32::from_rgb(21, 16, 33);
pub const RAISED: Color32 = Color32::from_rgb(41, 32, 62);
pub const BORDER: Color32 = Color32::from_rgb(62, 48, 94);
pub const TEXT: Color32 = Color32::from_rgb(233, 229, 243);
pub const MUTED: Color32 = Color32::from_rgb(152, 142, 177);
pub const ACCENT: Color32 = Color32::from_rgb(157, 108, 255);
/// A lighter purple, for the few things that should stand out from the accent.
pub const HIGHLIGHT: Color32 = Color32::from_rgb(208, 176, 255);
pub const WARN: Color32 = Color32::from_rgb(240, 178, 76);
pub const DANGER: Color32 = Color32::from_rgb(242, 100, 110);
pub const OK: Color32 = Color32::from_rgb(92, 208, 150);

/// The top and the bottom of the backdrop, and the light its front gives off on the way up.
const BACKDROP_BLACK: Color32 = Color32::from_rgb(7, 6, 10);
const BACKDROP_PURPLE: Color32 = Color32::from_rgb(62, 25, 110);
const BACKDROP_GLOW: Color32 = Color32::from_rgb(150, 96, 255);
/// The paler rim of that light where it wraps round something on the page. Lighter than the
/// band itself, so the outline still shows where the band is already bright.
const GLOW_RIM: Color32 = Color32::from_rgb(244, 236, 255);
/// How far the rise's front travels, in window heights: it starts at the bottom edge and
/// ends well past the top, so the whole window is lit once the rise is over.
const FRONT_TRAVEL: f32 = 1.6;
/// How far behind the front's leading edge its glow rides, in window heights.
const GLOW_BEHIND: f32 = 0.14;
/// The wordmark stays white on every splash, whatever the backdrop does underneath.
pub const SPLASH_INK: Color32 = Color32::WHITE;
pub const SPLASH_MUTED: Color32 = Color32::from_rgb(168, 156, 196);
/// The thin progress line on the splash.
pub const SPLASH_BAR: Color32 = Color32::from_rgb(148, 112, 255);
/// The icon keeps the pirxcy site's colours rather than the app's: a lime letter on a near-black tile.
const ICON_TILE: Color32 = Color32::from_rgb(8, 8, 10);
const ICON_INK: Color32 = Color32::from_rgb(200, 255, 46);

/// How the backdrop stands this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BackdropLook {
    /// How far the purple has climbed from the bottom, 0 to 1: a soft front carries it up the
    /// window when the app gets the focus.
    pub rise: f32,
    /// How brightly that front shines, 0 to 1.
    pub glow: f32,
    /// 0 to 1: how far into its live look it is, while the app has the focus and Roblox is
    /// running. Purple gathers at the top as well as the bottom, and both drift.
    pub live: f32,
    /// Seconds, for the drift. It stands still when motion is off.
    pub time: f32,
}

/// The second purple, the one that gathers at the top in the live look: bluer than the one
/// at the bottom.
const BACKDROP_TOP: Color32 = Color32::from_rgb(36, 24, 104);

/// Fills `rect` with the app's backdrop: black at the top, purple coming up from the bottom,
/// and in the live look purple at the top too. `opacity` fades the whole backdrop, for the
/// splash that dissolves over the app.
pub fn backdrop(painter: &egui::Painter, rect: Rect, look: BackdropLook, opacity: f32) {
    painter.add(backdrop_shape(rect, look, opacity));
}

/// [`backdrop`] as a shape, to put in a place kept for it earlier in the frame.
pub fn backdrop_shape(rect: Rect, look: BackdropLook, opacity: f32) -> Shape {
    // Rows of a mesh, coloured at the corners, so the gradient has no steps in it. They are
    // close enough together to carry the narrow middle of the glow. The live look changes
    // across the window as well, so it gets columns too.
    const ROWS: u32 = 128;
    let columns: u32 = if look.live > 0.001 { 28 } else { 1 };
    let mut mesh = egui::Mesh::default();
    for row in 0..=ROWS {
        let down = row as f32 / ROWS as f32;
        let y = rect.top() + rect.height() * down;
        for column in 0..=columns {
            let across = column as f32 / columns as f32;
            let color = alpha(backdrop_color(across, 1.0 - down, &look), opacity);
            mesh.colored_vertex(egui::pos2(rect.left() + rect.width() * across, y), color);
        }
        if row > 0 {
            let (above, here) = ((row - 1) * (columns + 1), row * (columns + 1));
            for column in 0..columns {
                mesh.add_triangle(above + column, above + column + 1, here + column);
                mesh.add_triangle(above + column + 1, here + column + 1, here + column);
            }
        }
    }
    Shape::mesh(mesh)
}

/// The backdrop's colour at `at` in `rect`, for something painted to match it.
pub fn backdrop_at(rect: Rect, at: egui::Pos2, look: BackdropLook) -> Color32 {
    backdrop_color((at.x - rect.left()) / rect.width(), 1.0 - (at.y - rect.top()) / rect.height(), &look)
}

/// [`backdrop`] with a hole round `center`, for the loading screen's zoom: the app shows
/// through it. `edge` is how far the hole reaches from `center` along evenly spaced
/// directions, clockwise from the right, so it can follow the inside of a letter. Painted as
/// rings out past the corners of `rect`, so it still covers the window once the whole picture
/// is scaled up round the hole.
pub fn backdrop_with_hole(painter: &egui::Painter, rect: Rect, look: BackdropLook, center: egui::Pos2, edge: &[f32]) {
    const RINGS: u32 = 40;
    let spokes = edge.len() as u32;
    if spokes < 3 {
        return;
    }
    let far = [rect.left_top(), rect.right_top(), rect.left_bottom(), rect.right_bottom()]
        .iter()
        .map(|corner| corner.distance(center))
        .fold(0.0_f32, f32::max)
        + 4.0;
    let mut mesh = egui::Mesh::default();
    for ring in 0..=RINGS {
        for (spoke, &inner) in edge.iter().enumerate() {
            // Closer together near the hole, where the zoom makes them biggest.
            let inner = inner.clamp(0.5, far);
            let r = inner * (far / inner).powf(ring as f32 / RINGS as f32);
            let angle = spoke as f32 * std::f32::consts::TAU / spokes as f32;
            let at = center + Vec2::new(angle.cos(), angle.sin()) * r;
            mesh.colored_vertex(at, backdrop_at(rect, at, look));
        }
        if ring > 0 {
            let (inner, outer) = ((ring - 1) * spokes, ring * spokes);
            for spoke in 0..spokes {
                let next = (spoke + 1) % spokes;
                mesh.add_triangle(inner + spoke, inner + next, outer + spoke);
                mesh.add_triangle(inner + next, outer + next, outer + spoke);
            }
        }
    }
    painter.add(Shape::mesh(mesh));
}

/// The hole of [`backdrop_with_hole`] filled in `color`, for closing it over.
pub fn hole_fill(center: egui::Pos2, edge: &[f32], color: Color32) -> Shape {
    let spokes = edge.len() as u32;
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(center, color);
    for (spoke, &r) in edge.iter().enumerate() {
        let angle = spoke as f32 * std::f32::consts::TAU / spokes as f32;
        mesh.colored_vertex(center + Vec2::new(angle.cos(), angle.sin()) * (r + 0.5), color);
    }
    for spoke in 0..spokes {
        mesh.add_triangle(0, 1 + spoke, 1 + (spoke + 1) % spokes);
    }
    Shape::mesh(mesh)
}

/// The backdrop's colour `across` the window (0 at the left, 1 at the right) and `height` of
/// the way up it (0 at the bottom, 1 at the top).
fn backdrop_color(across: f32, height: f32, look: &BackdropLook) -> Color32 {
    use std::f32::consts::TAU;
    let height = height.clamp(0.0, 1.0);
    let live = look.live.clamp(0.0, 1.0);
    let t = look.time;
    // In the live look the edge of each purple rolls slowly along the window: two swells of
    // different lengths moving opposite ways, so the shape never quite repeats.
    let swell = |phase: f32, slow: f32, quick: f32| {
        0.6 * (TAU * (0.55 * across + t / slow) + phase).sin() + 0.4 * (TAU * (1.3 * across - t / quick) + 1.7 * phase).sin()
    };
    let breathe = 1.0 + live * 0.07 * (TAU * t / 6.5).sin();
    // The purple gathers low down: by half way up most of it has gone. Live, it reaches a
    // little higher, by an amount that moves.
    let reach = 1.0 + live * (0.16 + 0.14 * swell(0.0, 13.0, 8.0));
    let depth = ((1.0 - height) * reach).clamp(0.0, 1.0).powf(1.7);
    let front = look.rise.clamp(0.0, 1.0) * FRONT_TRAVEL;
    let lit = ease_in_out((front - height) / 0.6);
    let bottom = depth * lit * breathe;
    // Live, a bluer purple comes down from the top the same way.
    let top_reach = 0.96 + 0.16 * swell(2.1, 11.0, 9.0);
    let top = live * (height * top_reach).clamp(0.0, 1.0).powf(2.1) * breathe;
    // The glow rides just behind the front's leading edge: a wide soft band with a brighter
    // narrow one inside it. It leaves by the top of the window while the purple settles.
    let off = height - (front - GLOW_BEHIND);
    let band = (0.55 * (-(off / 0.11).powi(2)).exp()) + (0.45 * (-(off / 0.035).powi(2)).exp());
    // Live, a faint slanted sheen drifts across the purple from left to right.
    let sweep = (t / 15.0).fract() * 1.8 - 0.4;
    let sheen = live * 0.09 * (-((across - sweep + (height - 0.5) * 0.4) / 0.16).powi(2)).exp() * (bottom + top).min(1.0);
    let shine = look.glow.clamp(0.0, 1.0) * band * 0.62 + sheen;
    let mix = |black: u8, purple: u8, upper: u8, light: u8| {
        let black = black as f32;
        (black + (purple as f32 - black) * bottom + (upper as f32 - black) * top + light as f32 * shine).round().clamp(0.0, 255.0) as u8
    };
    Color32::from_rgb(
        mix(BACKDROP_BLACK.r(), BACKDROP_PURPLE.r(), BACKDROP_TOP.r(), BACKDROP_GLOW.r()),
        mix(BACKDROP_BLACK.g(), BACKDROP_PURPLE.g(), BACKDROP_TOP.g(), BACKDROP_GLOW.g()),
        mix(BACKDROP_BLACK.b(), BACKDROP_PURPLE.b(), BACKDROP_TOP.b(), BACKDROP_GLOW.b()),
    )
}

/// Where the backdrop's glowing front is this frame, kept so that things on the page can
/// light up as it passes them.
#[derive(Clone, Copy)]
struct GlowFront {
    /// The middle of the glow, in screen points down the window.
    y: f32,
    /// How far above and below that it still lights things.
    reach: f32,
    strength: f32,
}

fn glow_front_id() -> egui::Id {
    egui::Id::new("backdrop glow front")
}

/// Notes where the backdrop's glowing front is, for [`glow_wrap`]. `rect` is the window;
/// `rise` and `glow` are what [`backdrop`] is painted with this frame.
pub fn set_glow_front(ctx: &egui::Context, rect: Rect, rise: f32, glow: f32) {
    let height = (rise.clamp(0.0, 1.0) * FRONT_TRAVEL) - GLOW_BEHIND;
    let front = GlowFront { y: rect.bottom() - (rect.height() * height), reach: rect.height() * 0.075, strength: glow.clamp(0.0, 1.0) };
    ctx.data_mut(|data| data.insert_temp(glow_front_id(), front));
}

/// How hard `rect`'s middle flashes as the front crosses it: 1 on the front, gone a short
/// way either side. `reach` is the front's own band, and the flash is a fraction of that,
/// so it lasts a split second and the next element has not started yet.
fn wrap_flash(center_y: f32, front_y: f32, reach: f32, strength: f32) -> f32 {
    if strength < 0.05 || reach < 1.0 {
        return 0.0;
    }
    let presence = (strength * 1.6).clamp(0.0, 1.0);
    let band = reach * 0.34;
    presence * (-((center_y - front_y) / band).powi(2)).exp()
}

/// The glowing front wrapped round `rect`. The light sits on the outline and is brightest
/// where that outline is level with the front, so as the front climbs it runs along the
/// bottom, up the sides and across the top of each record, tab or card in turn. As that
/// outline closes round an element it flashes white for a split second. Nothing to draw
/// when the front is elsewhere, which is nearly always.
pub fn glow_wrap(ctx: &egui::Context, rect: Rect, radius: CornerRadius) -> Shape {
    glow_outline(ctx, rect, radius, false)
}

/// [`glow_wrap`] for an element that sits on its own. When it flashes, a few short lines
/// leave each corner. A record in the list does not get these: the next row is too close.
pub fn glow_pop(ctx: &egui::Context, rect: Rect, radius: CornerRadius) -> Shape {
    glow_outline(ctx, rect, radius, true)
}

fn glow_outline(ctx: &egui::Context, rect: Rect, radius: CornerRadius, pop: bool) -> Shape {
    let Some(front) = ctx.data(|data| data.get_temp::<GlowFront>(glow_front_id())) else { return Shape::Noop };
    if front.strength < 0.02 || front.reach < 1.0 || rect.width() < 2.0 || rect.height() < 2.0 {
        return Shape::Noop;
    }
    let pad = front.reach + 6.0;
    if rect.bottom() < front.y - pad || rect.top() > front.y + pad {
        return Shape::Noop;
    }
    let points = outline_points(rect, radius);
    let n = points.len();
    if n < 3 {
        return Shape::Noop;
    }
    let light_at = |y: f32| front.strength * (-((y - front.y) / front.reach).powi(2)).exp();
    let flash = wrap_flash(rect.center().y, front.y, front.reach, front.strength);
    if points.iter().all(|point| light_at(point.y) < 0.02) && flash < 0.02 {
        return Shape::Noop;
    }

    // Outward normals from the two edges that meet at each point, so the stroke stays even
    // round the corners rather than being pulled along the long sides.
    let mut edge = Vec::with_capacity(n);
    for i in 0..n {
        let step = points[(i + 1) % n] - points[i];
        let len = step.length();
        edge.push(if len < 0.01 { Vec2::ZERO } else { Vec2::new(step.y, -step.x) / len });
    }
    let mut normal = Vec::with_capacity(n);
    for i in 0..n {
        let sum = edge[(i + n - 1) % n] + edge[i];
        let len = sum.length();
        normal.push(if len < 0.01 { edge[i] } else { sum / len });
    }

    // Just inside the edge, on it, then two steps out that fade. The flash lifts the whole
    // outline to white, not only the part level with the front.
    let rim = GLOW_RIM.lerp_to_gamma(Color32::WHITE, flash);
    let bloom = BACKDROP_GLOW.lerp_to_gamma(Color32::WHITE, flash * 0.85);
    let rings = [(-1.1_f32, 0.0_f32), (0.0, 1.0), (1.6, 0.55), (4.0, 0.0)];
    let mut mesh = egui::Mesh::default();
    for i in 0..n {
        let lit = (light_at(points[i].y) + flash).min(1.0);
        for &(offset, falloff) in &rings {
            let amount = if falloff >= 0.99 { lit } else { (lit * falloff + if offset > 2.0 { flash * 0.5 } else { 0.0 }).min(1.0) };
            let color = if falloff >= 0.99 { alpha(rim, amount) } else { alpha(bloom, amount) };
            mesh.colored_vertex(points[i] + normal[i] * offset, color);
        }
    }
    let rings_n = rings.len() as u32;
    for i in 0..n {
        let next = (i + 1) % n;
        for k in 0..rings_n - 1 {
            let here = i as u32 * rings_n + k;
            let there = next as u32 * rings_n + k;
            mesh.add_triangle(here, there, here + 1);
            mesh.add_triangle(here + 1, there, there + 1);
        }
    }
    let mesh = Shape::mesh(mesh);
    // A record or a tab is wrapped all at once, so the flash fills it. A tall card only
    // flares its outline, or the wash would blank the whole card.
    let wash = flash * if rect.height() <= front.reach * 1.15 { 0.55 } else { 0.08 };
    let mut shapes = Vec::new();
    if wash >= 0.03 {
        shapes.push(Shape::rect_filled(rect, radius, alpha(Color32::WHITE, wash)));
    }
    shapes.push(mesh);
    if pop && flash > 0.12 {
        shapes.push(corner_pop(rect, radius, flash));
    }
    if shapes.len() == 1 {
        shapes.pop().unwrap_or(Shape::Noop)
    } else {
        Shape::Vec(shapes)
    }
}

/// Short tapered lines leaving each corner, along the diagonal and a little to either side.
/// They start just clear of the corner, so the element looks as if it pops.
fn corner_pop(rect: Rect, radius: CornerRadius, flash: f32) -> Shape {
    let spark = (rect.width().min(rect.height()) * 0.42).clamp(8.0, 20.0);
    let mut mesh = egui::Mesh::default();
    for (origin, dir) in corner_rays(rect, radius) {
        for (angle, len, width) in [(0.0_f32, 1.0_f32, 1.35_f32), (-0.46, 0.58, 0.95), (0.46, 0.58, 0.95)] {
            let out = turn(dir, angle);
            add_ray(&mut mesh, origin + out * 3.0, out, spark * len, width, flash);
        }
    }
    Shape::mesh(mesh)
}

/// Where each corner's lines begin, and the direction that leaves the element there.
fn corner_rays(rect: Rect, radius: CornerRadius) -> [(egui::Pos2, Vec2); 4] {
    let limit = (rect.width().min(rect.height()) * 0.5).max(0.0);
    let clamp = |corner: u8| (corner as f32).min(limit);
    let ray = |center: egui::Pos2, radius: f32, dir: Vec2| {
        let dir = dir.normalized();
        (center + dir * radius, dir)
    };
    let (nw, ne, se, sw) = (clamp(radius.nw), clamp(radius.ne), clamp(radius.se), clamp(radius.sw));
    [
        ray(egui::pos2(rect.right() - ne, rect.top() + ne), ne, Vec2::new(1.0, -1.0)),
        ray(egui::pos2(rect.right() - se, rect.bottom() - se), se, Vec2::new(1.0, 1.0)),
        ray(egui::pos2(rect.left() + sw, rect.bottom() - sw), sw, Vec2::new(-1.0, 1.0)),
        ray(egui::pos2(rect.left() + nw, rect.top() + nw), nw, Vec2::new(-1.0, -1.0)),
    ]
}

fn turn(dir: Vec2, angle: f32) -> Vec2 {
    let (sin, cos) = angle.sin_cos();
    Vec2::new(dir.x * cos - dir.y * sin, dir.x * sin + dir.y * cos)
}

/// A line from `from` along `dir`, wide at the start and gone at the tip.
fn add_ray(mesh: &mut egui::Mesh, from: egui::Pos2, dir: Vec2, length: f32, width: f32, flash: f32) {
    let side = Vec2::new(-dir.y, dir.x);
    let tip = from + dir * length;
    let tri = |mesh: &mut egui::Mesh, half: f32, base: Color32, end: Color32| {
        let at = mesh.vertices.len() as u32;
        mesh.colored_vertex(from + side * half, base);
        mesh.colored_vertex(from - side * half, base);
        mesh.colored_vertex(tip, end);
        mesh.add_triangle(at, at + 1, at + 2);
    };
    tri(mesh, width * 2.1, alpha(BACKDROP_GLOW, flash * 0.55), alpha(BACKDROP_GLOW, 0.0));
    tri(mesh, width, alpha(Color32::WHITE, flash), alpha(Color32::WHITE, 0.0));
}

/// Points round `rect`'s rounded outline, clockwise from the top edge. Straight runs are
/// split so a tall side can be bright in the middle and dark at the ends.
fn outline_points(rect: Rect, radius: CornerRadius) -> Vec<egui::Pos2> {
    let limit = (rect.width().min(rect.height()) * 0.5).max(0.0);
    let clamp = |corner: u8| (corner as f32).min(limit);
    let (nw, ne, se, sw) = (clamp(radius.nw), clamp(radius.ne), clamp(radius.se), clamp(radius.sw));
    let turns = [
        (egui::pos2(rect.right() - ne, rect.top() + ne), ne, -std::f32::consts::FRAC_PI_2, 0.0),
        (egui::pos2(rect.right() - se, rect.bottom() - se), se, 0.0, std::f32::consts::FRAC_PI_2),
        (egui::pos2(rect.left() + sw, rect.bottom() - sw), sw, std::f32::consts::FRAC_PI_2, std::f32::consts::PI),
        (egui::pos2(rect.left() + nw, rect.top() + nw), nw, std::f32::consts::PI, std::f32::consts::PI * 1.5),
    ];
    let mut arcs = Vec::with_capacity(4);
    for (center, radius, start, end) in turns {
        let mut arc = Vec::new();
        if radius < 0.5 {
            arc.push(center);
        } else {
            const STEPS: usize = 5;
            for step in 0..=STEPS {
                let angle = start + (end - start) * (step as f32 / STEPS as f32);
                arc.push(center + Vec2::new(angle.cos(), angle.sin()) * radius);
            }
        }
        arcs.push(arc);
    }
    let mut points = Vec::new();
    for i in 0..arcs.len() {
        let start = arcs[i][0];
        let previous = *arcs[(i + arcs.len() - 1) % arcs.len()].last().unwrap();
        let span = start - previous;
        let steps = (span.length() / 12.0).ceil() as usize;
        if steps > 1 {
            for step in 1..steps {
                points.push(previous + span * (step as f32 / steps as f32));
            }
        }
        let from = usize::from(points.last().is_some_and(|point| point.distance(start) < 0.4));
        points.extend_from_slice(&arcs[i][from..]);
    }
    if points.len() >= 2 && points[0].distance(*points.last().unwrap()) < 0.4 {
        points.pop();
    }
    points
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
    let tile = [ICON_TILE.r(), ICON_TILE.g(), ICON_TILE.b()];
    let ink = [ICON_INK.r() as f32, ICON_INK.g() as f32, ICON_INK.b() as f32];
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

/// The wordmark's font file, for measuring its letters.
pub fn wordmark_font_data() -> &'static [u8] {
    MANROPE
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
    v.window_fill = SURFACE;
    v.extreme_bg_color = Color32::from_rgb(27, 21, 42);
    v.faint_bg_color = Color32::from_rgb(18, 14, 28);
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
    w.hovered.bg_fill = Color32::from_rgb(56, 43, 86);
    w.hovered.weak_bg_fill = Color32::from_rgb(56, 43, 86);
    w.hovered.bg_stroke = Stroke::new(1.0_f32, alpha(ACCENT, 0.7));
    w.hovered.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
    w.active.bg_fill = alpha(ACCENT, 0.55);
    w.active.weak_bg_fill = alpha(ACCENT, 0.55);
    w.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    w.active.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
    w.open.bg_fill = Color32::from_rgb(50, 39, 77);
    w.open.weak_bg_fill = Color32::from_rgb(50, 39, 77);
    w.open.bg_stroke = Stroke::new(1.0_f32, alpha(ACCENT, 0.7));
    w.open.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
    for visuals in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        visuals.corner_radius = radius;
        visuals.expansion = 0.0;
    }

    let s = &mut style.spacing;
    // Room between things, so the lines that leave a hovered button land in the gap rather
    // than on its neighbour.
    s.item_spacing = Vec2::new(10.0, 8.0);
    s.button_padding = Vec2::new(12.0, 6.0);
    s.interact_size.y = 28.0;
    s.menu_margin = Margin::same(8);
    s.window_margin = Margin::same(18);
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

/// Slow at both ends, for something that should neither start nor stop abruptly.
pub fn ease_in_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The backdrop's colour half way across, without the live look.
    fn still(height: f32, rise: f32, glow: f32) -> Color32 {
        backdrop_color(0.5, height, &BackdropLook { rise, glow, live: 0.0, time: 0.0 })
    }

    #[test]
    fn the_backdrop_rises_from_the_bottom() {
        // Before the rise the whole window is black.
        for height in [0.0, 0.5, 1.0] {
            assert_eq!(still(height, 0.0, 0.0), BACKDROP_BLACK);
        }
        // Early on the bottom edge has colour and half way up has none yet.
        assert_ne!(still(0.0, 0.2, 0.0), BACKDROP_BLACK);
        assert_eq!(still(0.5, 0.2, 0.0), BACKDROP_BLACK);
        // Once risen: purple at the bottom, some of it half way up, black at the top.
        assert_eq!(still(0.0, 1.0, 0.0), BACKDROP_PURPLE);
        assert_ne!(still(0.5, 1.0, 0.0), BACKDROP_BLACK);
        assert_eq!(still(1.0, 1.0, 0.0), BACKDROP_BLACK);
    }

    #[test]
    fn the_glow_rides_the_front_and_is_gone_once_risen() {
        let brightness = |c: Color32| c.r() as u32 + c.g() as u32 + c.b() as u32;
        // Part way up, the glowing front is brighter than the same spot without the glow,
        // and brighter than the settled purple at the bottom edge.
        let (rise, at) = (0.4, 0.4 * 1.6 - 0.14);
        assert!(brightness(still(at, rise, 1.0)) > brightness(still(at, rise, 0.0)) + 100);
        assert!(brightness(still(at, rise, 1.0)) > brightness(BACKDROP_PURPLE));
        // Far from the front it changes nothing.
        assert_eq!(still(0.0, 0.9, 1.0), still(0.0, 0.9, 0.0));
        // Once the rise is over the front has left by the top, so even a full glow shows nothing.
        for height in [0.0, 0.5, 1.0] {
            assert_eq!(still(height, 1.0, 1.0), still(height, 1.0, 0.0));
        }
    }

    #[test]
    fn the_live_look_adds_purple_at_the_top_and_moves() {
        let risen =
            |live: f32, across: f32, time: f32, height: f32| backdrop_color(across, height, &BackdropLook { rise: 1.0, glow: 0.0, live, time });
        let blue = |c: Color32| c.b() as u32;
        // The top is black without it and purple with it; the bottom stays purple.
        assert_eq!(risen(0.0, 0.5, 0.0, 1.0), BACKDROP_BLACK);
        assert!(blue(risen(1.0, 0.5, 0.0, 1.0)) > blue(BACKDROP_BLACK) + 60);
        assert!(blue(risen(1.0, 0.5, 0.0, 0.0)) >= blue(BACKDROP_PURPLE) - 10);
        // It changes across the window and over time, a little.
        let middle = |across: f32, time: f32| risen(1.0, across, time, 0.62);
        assert_ne!(middle(0.1, 0.0), middle(0.7, 0.0));
        assert_ne!(middle(0.3, 0.0), middle(0.3, 3.0));
        // Half in, it is between the two.
        assert!(blue(risen(0.5, 0.5, 0.0, 1.0)) < blue(risen(1.0, 0.5, 0.0, 1.0)));
    }

    #[test]
    fn the_glow_outline_goes_round_the_element() {
        let rect = Rect::from_min_max(egui::pos2(10.0, 20.0), egui::pos2(210.0, 80.0));
        let points = outline_points(rect, CornerRadius::same(8));
        assert!(points.len() > 16);
        let (mut left, mut top, mut right, mut bottom) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for point in &points {
            left = left.min(point.x);
            top = top.min(point.y);
            right = right.max(point.x);
            bottom = bottom.max(point.y);
            assert!(rect.expand(0.6).contains(*point));
        }
        // It reaches every side, and it stays off the square corners: those are rounded away.
        assert!((left - rect.left()).abs() < 0.6);
        assert!((right - rect.right()).abs() < 0.6);
        assert!((top - rect.top()).abs() < 0.6);
        assert!((bottom - rect.bottom()).abs() < 0.6);
        // The square corner is rounded off, so nothing sits on it.
        assert!(points.iter().all(|point| point.distance(rect.left_top()) > 3.0));
        let center = points.iter().fold(Vec2::ZERO, |sum, point| sum + point.to_vec2()) / points.len() as f32;
        assert!((center - rect.center().to_vec2()).length() < 1.0);
    }

    #[test]
    fn the_wrap_flashes_only_as_the_front_crosses() {
        let reach = 80.0;
        assert!((wrap_flash(100.0, 100.0, reach, 1.0) - 1.0).abs() < 0.001);
        // A row away it has already fallen off, so each element flashes on its own.
        assert!(wrap_flash(100.0, 170.0, reach, 1.0) < 0.05);
        assert_eq!(wrap_flash(100.0, 100.0, reach, 0.0), 0.0);
        // While the glow is up at all, crossing the middle still flashes hard.
        assert!(wrap_flash(100.0, 100.0, reach, 0.4) > 0.6);
    }

    #[test]
    fn corner_lines_leave_the_element() {
        let rect = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(120.0, 48.0));
        let center = rect.center().to_vec2();
        for (point, dir) in corner_rays(rect, CornerRadius::same(12)) {
            let away = point.to_vec2() - center;
            assert!(dir.x * away.x + dir.y * away.y > 0.0);
            assert!((dir.length() - 1.0).abs() < 0.01);
            assert!(rect.expand(0.6).contains(point));
        }
    }
}
