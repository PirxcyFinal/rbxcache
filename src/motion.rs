//! Timing for the loading screen's transitions and the line burst on hover. Each picture
//! depends only on the time, so these are plain functions of seconds.
//!
//! Zoom through follows Motion Prompt #020 (MIT License (c) 2026 Shohei Ohtaki,
//! https://showheyohtaki.github.io/motion-prompt/): its shapes, timing and easing are kept,
//! only the scene is rbxcache's own.

use eframe::egui;
use std::sync::OnceLock;

pub fn clamp01(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

pub fn lerp(a: f32, b: f32, x: f32) -> f32 {
    a + (b - a) * x
}

/// How far `t` is between `a` and `b`, 0 to 1.
pub fn progress(t: f32, a: f32, b: f32) -> f32 {
    clamp01((t - a) / (b - a))
}

pub fn ease_in(x: f32) -> f32 {
    x * x * x
}

pub fn ease_out(x: f32) -> f32 {
    1.0 - (1.0 - x).powi(3)
}

/// The same 0..1 number for the same integer.
pub fn hash01(n: i32) -> f32 {
    let mut n = ((n as u32) ^ 0x9e37_79b9).wrapping_mul(0x85eb_ca6b);
    n ^= n >> 13;
    n = n.wrapping_mul(0xc2b2_ae35);
    n ^= n >> 16;
    n as f32 / 4_294_967_296.0
}

/// How the loading screen comes in. One is picked at random each time it shows; it always
/// leaves the same way, diving through the hole of the "p" into the app.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Entrance {
    /// Shrinks in out of a blur.
    Puff,
    /// Rises from below, past its place, and settles back.
    Bounce,
}

impl Entrance {
    pub const ALL: [Entrance; 2] = [Entrance::Puff, Entrance::Bounce];

    /// One of the two, picked from the clock: nothing else needs to be random.
    pub fn random() -> Self {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
        Self::ALL[(hash01(nanos as i32) * Self::ALL.len() as f32) as usize % Self::ALL.len()]
    }
}

/// `RBXCACHE_SPLASH=puff|bounce[,seconds]` makes every loading screen come in that way and
/// stay at least that long, for checking one of them without waiting for the dice.
pub fn splash_override() -> (Option<Entrance>, f32) {
    static OVERRIDE: OnceLock<(Option<Entrance>, f32)> = OnceLock::new();
    *OVERRIDE.get_or_init(|| parse_override(&std::env::var("RBXCACHE_SPLASH").unwrap_or_default()))
}

fn parse_override(text: &str) -> (Option<Entrance>, f32) {
    let mut parts = text.split(',').map(str::trim);
    let entrance = match parts.next().unwrap_or("").to_ascii_lowercase().as_str() {
        "puff" => Some(Entrance::Puff),
        // "zoom" was the name of the bounce in, back when it alone left by the zoom.
        "bounce" | "zoom" | "zoom-through" => Some(Entrance::Bounce),
        _ => None,
    };
    let stay = parts.next().and_then(|s| s.parse::<f32>().ok()).filter(|s| s.is_finite()).unwrap_or(0.0).clamp(0.0, 60.0);
    (entrance, stay)
}

// Puff in.

/// Seconds to puff in.
pub const PUFF_IN: f32 = 0.42;

/// The puff at `t` seconds after it appeared: scale, blur radius in points, and opacity.
pub fn puff(t: f32) -> (f32, f32, f32) {
    let x = ease_out(progress(t, 0.0, PUFF_IN));
    (lerp(1.6, 1.0, x), lerp(18.0, 0.0, x), x)
}

// Zoom through (Motion Prompt #020).

/// The zoom is over by this many seconds after it starts.
pub const ZOOM_END: f32 = 1.65;

/// How far into the zoom, 0 to 1: a cubic ease-in from 0.25 to 1.2 s carries 80% of it and
/// a cubic ease-out from 1.2 to 1.65 s the rest.
pub fn zoom_progress(u: f32) -> f32 {
    const TA: f32 = 0.5;
    const TB: f32 = 1.5;
    ease_in(progress(u, TA - 0.25, TB - 0.3)) * 0.8 + ease_out(progress(u, TB - 0.3, TB + 0.15)) * 0.2
}

/// The scale at which a hole of `radius` round `center` covers all of `frame`, times 1.08.
pub fn zoom_max_scale(center: egui::Pos2, radius: f32, frame: egui::Rect) -> f32 {
    let dx = (center.x - frame.left()).max(frame.right() - center.x);
    let dy = (center.y - frame.top()).max(frame.bottom() - center.y);
    (dx.hypot(dy) / radius.max(0.5)) * 1.08
}

/// The scale at `u`: exponential, so it reads as a constant speed.
pub fn zoom_scale(u: f32, max_scale: f32) -> f32 {
    max_scale.powf(zoom_progress(u))
}

// Shape divider (Motion Prompt #165).

/// One loop of the waves between two things on a page, in seconds.
pub const DIVIDER_LOOP: f32 = 4.7;

/// How far the back and the front wave of a divider are from their middle lines `x` points
/// across, `t` seconds in. The back wave adds a 260 point swell 8 high to a 130 point one 4
/// high; the front a 300 point swell 9 high to a 110 point one 3.5 high. The two in each run
/// opposite ways, so the shape keeps changing, and it is back where it began every loop.
pub fn divider_waves(x: f32, t: f32) -> (f32, f32) {
    use std::f32::consts::TAU;
    let a = TAU * t / DIVIDER_LOOP;
    let back = 8.0 * (TAU * x / 260.0 + a + 1.0).sin() + 4.0 * (TAU * x / 130.0 - 2.0 * a).sin();
    let front = 9.0 * (TAU * x / 300.0 - a).sin() + 3.5 * (TAU * x / 110.0 + 2.0 * a + 0.5).sin();
    (back, front)
}

// Line burst.

/// Seconds a line burst lasts.
pub const BURST: f32 = 0.34;

/// Where a burst's stroke is at `t` seconds, as fractions of its length from its inner end:
/// it grows outward from the inner end, then its inner end runs out after the outer one
/// until it is gone.
pub fn burst_stroke(t: f32) -> (f32, f32) {
    let x = t / BURST;
    (ease_out(progress(x, 0.38, 1.0)), ease_out(progress(x, 0.0, 0.55)))
}

/// One stretch of a rounded rectangle's outline: its length, and the point and outward
/// direction a distance along it.
type Segment<'a> = (f32, Box<dyn Fn(f32) -> (egui::Pos2, egui::Vec2) + 'a>);

/// Points evenly spaced round a rounded rectangle, with the way out of it at each, for the
/// strokes of a burst. Straight sides and corners share the spacing.
pub fn ring_points(rect: egui::Rect, radius: f32, count: usize) -> Vec<(egui::Pos2, egui::Vec2)> {
    use std::f32::consts::{FRAC_PI_2, PI};
    let r = radius.min(rect.width() / 2.0).min(rect.height() / 2.0).max(0.0);
    let (w, h) = (rect.width() - 2.0 * r, rect.height() - 2.0 * r);
    let arc = FRAC_PI_2 * r;
    let total = 2.0 * (w + h) + 4.0 * arc;
    if total <= 0.0 || count == 0 {
        return Vec::new();
    }
    let corner = |center: egui::Pos2, from: f32, along: f32| {
        let angle = from + if r > 0.0 { along / r } else { FRAC_PI_2 * (along / arc.max(1e-3)) };
        let dir = egui::vec2(angle.cos(), angle.sin());
        (center + dir * r, dir)
    };
    // Clockwise from the top-left end of the top side.
    let segments: [Segment<'_>; 8] = [
        (w, Box::new(move |s| (egui::pos2(rect.left() + r + s, rect.top()), egui::vec2(0.0, -1.0)))),
        (arc, Box::new(move |s| corner(egui::pos2(rect.right() - r, rect.top() + r), -FRAC_PI_2, s))),
        (h, Box::new(move |s| (egui::pos2(rect.right(), rect.top() + r + s), egui::vec2(1.0, 0.0)))),
        (arc, Box::new(move |s| corner(egui::pos2(rect.right() - r, rect.bottom() - r), 0.0, s))),
        (w, Box::new(move |s| (egui::pos2(rect.right() - r - s, rect.bottom()), egui::vec2(0.0, 1.0)))),
        (arc, Box::new(move |s| corner(egui::pos2(rect.left() + r, rect.bottom() - r), FRAC_PI_2, s))),
        (h, Box::new(move |s| (egui::pos2(rect.left(), rect.bottom() - r - s), egui::vec2(-1.0, 0.0)))),
        (arc, Box::new(move |s| corner(egui::pos2(rect.left() + r, rect.top() + r), PI, s))),
    ];
    let step = total / count as f32;
    (0..count)
        .map(|i| {
            let mut at = step * (i as f32 + 0.5);
            for (length, point) in &segments {
                if at <= *length || *length >= total {
                    return point(at.min(*length));
                }
                at -= length;
            }
            segments[7].1(arc)
        })
        .map(|(p, d)| (p, if d.length() > 0.0 { d.normalized() } else { egui::vec2(0.0, -1.0) }))
        .collect()
}

/// How far a stroke can go from `from` along `dir` before it would touch one of `around`,
/// up to `most`. This is how a burst fits the gap it has: a neighbour close by shortens the
/// strokes on that side, and one that is touching leaves no room at all.
pub fn ray_room(from: egui::Pos2, dir: egui::Vec2, most: f32, around: &[egui::Rect]) -> f32 {
    let mut room = most;
    for rect in around {
        if rect.contains(from) {
            return 0.0;
        }
        // Where the ray enters the box, if it does: the slab method, one axis at a time.
        let (mut near, mut far) = (0.0_f32, room);
        for (o, d, lo, hi) in [(from.x, dir.x, rect.left(), rect.right()), (from.y, dir.y, rect.top(), rect.bottom())] {
            if d.abs() < 1e-6 {
                if o < lo || o > hi {
                    far = -1.0;
                }
                continue;
            }
            let (a, b) = ((lo - o) / d, (hi - o) / d);
            near = near.max(a.min(b));
            far = far.min(a.max(b));
        }
        if near <= far {
            room = room.min(near);
        }
    }
    room.max(0.0)
}

// The hole in the wordmark's "p".

/// Where the inside of the "p" is, as fractions of the glyph's inked box: its centre across
/// and down, and the radius of the largest circle that fits, over the box's height.
pub fn p_hole(font: &[u8]) -> (f32, f32, f32) {
    static HOLE: OnceLock<(f32, f32, f32)> = OnceLock::new();
    *HOLE.get_or_init(|| measure_hole(font, 'p').unwrap_or((0.62, 0.36, 0.14)))
}

fn measure_hole(font: &[u8], letter: char) -> Option<(f32, f32, f32)> {
    use ab_glyph::{Font, FontRef, PxScale};
    let font = FontRef::try_from_slice(font).ok()?;
    let outline = font.outline_glyph(font.glyph_id(letter).with_scale(PxScale::from(400.0)))?;
    let bounds = outline.px_bounds();
    let (w, h) = (bounds.width().ceil() as usize, bounds.height().ceil() as usize);
    if w < 8 || h < 8 {
        return None;
    }
    let mut ink = vec![false; w * h];
    outline.draw(|x, y, coverage| {
        if (x as usize) < w && (y as usize) < h {
            ink[y as usize * w + x as usize] = coverage > 0.5;
        }
    });
    hole_in(&ink, w, h).map(|(cx, cy, r)| (cx / w as f32, cy / h as f32, r / h as f32))
}

/// The widest run of empty pixels with ink on both sides of it, then how far it reaches up
/// and down from its middle: the centre and radius of the counter of a letter.
fn hole_in(ink: &[bool], w: usize, h: usize) -> Option<(f32, f32, f32)> {
    let mut best: Option<(usize, usize, usize)> = None;
    for y in 0..h {
        let row = &ink[y * w..(y + 1) * w];
        let Some(first) = row.iter().position(|&i| i) else { continue };
        let Some(last) = row.iter().rposition(|&i| i) else { continue };
        let mut x = first;
        while x < last {
            if row[x] {
                x += 1;
                continue;
            }
            let start = x;
            while x < last && !row[x] {
                x += 1;
            }
            if best.is_none_or(|(_, a, b)| x - start > b - a) {
                best = Some((y, start, x));
            }
        }
    }
    let (y, start, end) = best?;
    let cx = (start + end) / 2;
    let up = (0..y).rev().find(|&yy| ink[yy * w + cx]).map_or(0, |yy| yy + 1);
    let down = (y..h).find(|&yy| ink[yy * w + cx]).unwrap_or(h);
    let r = ((end - start) as f32).min((down - up) as f32) / 2.0;
    (r > 1.0).then_some((cx as f32 + 0.5, (up + down) as f32 / 2.0, r))
}

// Letters as shapes.

/// A letter of the wordmark as filled triangles and its outline, in fractions of its inked box
/// (0 to 1 across, 0 to 1 down). Text in egui is a picture of each letter at one size, which
/// goes soft when it is scaled up; the zoom scales the "p" about fifty times, so the letters
/// are drawn from their outlines instead.
pub struct Letter {
    /// Three corners per triangle.
    pub triangles: Vec<egui::Pos2>,
    /// The closed outlines, for the soft edge the triangles do not have.
    pub contours: Vec<Vec<egui::Pos2>>,
}

/// The letter `ch` of `font`, worked out once.
pub fn letter(font: &'static [u8], ch: char) -> Option<std::sync::Arc<Letter>> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    static LETTERS: OnceLock<Mutex<HashMap<char, Option<Arc<Letter>>>>> = OnceLock::new();
    let mut letters = LETTERS.get_or_init(Default::default).lock().ok()?;
    letters.entry(ch).or_insert_with(|| outline_letter(font, ch).map(Arc::new)).clone()
}

fn outline_letter(font: &[u8], ch: char) -> Option<Letter> {
    use ab_glyph::{Font, FontRef, OutlineCurve, Point};
    let font = FontRef::try_from_slice(font).ok()?;
    let outline = font.outline(font.glyph_id(ch))?;
    let size = (outline.bounds.width().abs()).max(outline.bounds.height().abs()).max(1.0);
    // Enough steps that a curve stays round when the letter fills the window.
    let steps = |length: f32| ((length / size * 96.0).ceil() as usize).clamp(2, 40);
    let mut contours: Vec<Vec<egui::Pos2>> = Vec::new();
    let mut last: Option<Point> = None;
    let at = |p: Point| egui::pos2(p.x, p.y);
    for curve in &outline.curves {
        let (start, end) = match *curve {
            OutlineCurve::Line(a, b) => (a, b),
            OutlineCurve::Quad(a, _, c) => (a, c),
            OutlineCurve::Cubic(a, _, _, d) => (a, d),
        };
        if last.is_none_or(|p| (p.x - start.x).abs() > 1e-3 || (p.y - start.y).abs() > 1e-3) {
            contours.push(vec![at(start)]);
        }
        let Some(contour) = contours.last_mut() else { continue };
        match *curve {
            OutlineCurve::Line(_, b) => contour.push(at(b)),
            OutlineCurve::Quad(a, b, c) => {
                let n = steps(at(a).distance(at(b)) + at(b).distance(at(c)));
                for i in 1..=n {
                    let t = i as f32 / n as f32;
                    let u = 1.0 - t;
                    contour.push(egui::pos2(u * u * a.x + 2.0 * u * t * b.x + t * t * c.x, u * u * a.y + 2.0 * u * t * b.y + t * t * c.y));
                }
            }
            OutlineCurve::Cubic(a, b, c, d) => {
                let n = steps(at(a).distance(at(b)) + at(b).distance(at(c)) + at(c).distance(at(d)));
                for i in 1..=n {
                    let t = i as f32 / n as f32;
                    let u = 1.0 - t;
                    let (wa, wb, wc, wd) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                    contour.push(egui::pos2(wa * a.x + wb * b.x + wc * c.x + wd * d.x, wa * a.y + wb * b.y + wc * c.y + wd * d.y));
                }
            }
        }
        last = Some(end);
    }
    for contour in &mut contours {
        if contour.len() > 1 && contour[0].distance(contour[contour.len() - 1]) < 1e-3 {
            contour.pop();
        }
    }
    contours.retain(|c| c.len() >= 3);
    // Font units point up; the box's fractions point down.
    let points = contours.iter().flatten();
    let (min, max) = points.fold((egui::pos2(f32::MAX, f32::MAX), egui::pos2(f32::MIN, f32::MIN)), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    let (w, h) = ((max.x - min.x).max(1e-3), (max.y - min.y).max(1e-3));
    for contour in &mut contours {
        for p in contour.iter_mut() {
            *p = egui::pos2((p.x - min.x) / w, (max.y - p.y) / h);
        }
    }
    let triangles = fill_triangles(&contours);
    (!triangles.is_empty()).then_some(Letter { triangles, contours })
}

/// The inside of `contours` (non-zero winding) as triangles: cut into bands at every corner's
/// height, where each edge is straight across the band, and fill between the edges that go in
/// and out of the shape.
fn fill_triangles(contours: &[Vec<egui::Pos2>]) -> Vec<egui::Pos2> {
    let mut edges = Vec::new();
    for contour in contours {
        for i in 0..contour.len() {
            let (a, b) = (contour[i], contour[(i + 1) % contour.len()]);
            if (a.y - b.y).abs() > 1e-7 {
                edges.push(if a.y < b.y { (a, b, 1) } else { (b, a, -1) });
            }
        }
    }
    let mut heights: Vec<f32> = contours.iter().flatten().map(|p| p.y).collect();
    heights.sort_by(f32::total_cmp);
    heights.dedup_by(|a, b| (*a - *b).abs() < 1e-7);
    let mut triangles = Vec::new();
    let mut crossings: Vec<(f32, f32, f32, i32)> = Vec::new();
    for band in heights.windows(2) {
        let (top, bottom) = (band[0], band[1]);
        let middle = (top + bottom) / 2.0;
        crossings.clear();
        for &(a, b, winding) in &edges {
            if a.y < middle && middle < b.y {
                let x = |y: f32| a.x + (b.x - a.x) * (y - a.y) / (b.y - a.y);
                crossings.push((x(middle), x(top), x(bottom), winding));
            }
        }
        crossings.sort_by(|p, q| p.0.total_cmp(&q.0));
        let mut winding = 0;
        let mut left = (0.0, 0.0);
        for &(_, x_top, x_bottom, turn) in &crossings {
            let inside = winding != 0;
            winding += turn;
            if !inside && winding != 0 {
                left = (x_top, x_bottom);
            } else if inside && winding == 0 {
                let (lt, lb) = left;
                triangles.extend([
                    egui::pos2(lt, top),
                    egui::pos2(x_top, top),
                    egui::pos2(x_bottom, bottom),
                    egui::pos2(lt, top),
                    egui::pos2(x_bottom, bottom),
                    egui::pos2(lb, bottom),
                ]);
            }
        }
    }
    triangles
}

// Reduced motion.

/// True when Windows is set not to show animations ("Animation effects" off), which is
/// what prefers-reduced-motion follows there.
pub fn reduced_motion() -> bool {
    #[cfg(windows)]
    {
        #[link(name = "user32")]
        extern "system" {
            fn SystemParametersInfoW(action: u32, param: u32, value: *mut std::ffi::c_void, ini: u32) -> i32;
        }
        const SPI_GETCLIENTAREAANIMATION: u32 = 0x1042;
        let mut on: i32 = 1;
        let ok = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&mut on as *mut i32).cast(), 0) };
        ok != 0 && on == 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_zoom_accelerates_in_and_settles() {
        assert_eq!(zoom_progress(0.0), 0.0);
        assert_eq!(zoom_progress(0.25), 0.0);
        assert!((zoom_progress(1.2) - 0.8).abs() < 1e-5);
        assert!((zoom_progress(ZOOM_END) - 1.0).abs() < 1e-5);
        let frame = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(539.0, 208.0));
        let max = zoom_max_scale(egui::pos2(228.0, 106.0), 24.5, frame);
        assert!((13.0..15.5).contains(&max), "{max}");
        assert!((zoom_scale(ZOOM_END, max) - max).abs() < 1e-3);
        assert_eq!(zoom_scale(0.0, max), 1.0);
    }

    #[test]
    fn the_divider_waves_sway_and_come_back() {
        for x in [0.0, 37.0, 260.0, 511.0] {
            let (back, front) = divider_waves(x, 0.0);
            let (later_back, later_front) = divider_waves(x, DIVIDER_LOOP);
            assert!((back - later_back).abs() < 1e-3 && (front - later_front).abs() < 1e-3);
            assert!(back.abs() <= 12.0 && front.abs() <= 12.5);
        }
        assert_ne!(divider_waves(100.0, 0.0), divider_waves(100.0, 1.0));
    }

    #[test]
    fn a_burst_stroke_grows_out_then_runs_off() {
        assert_eq!(burst_stroke(0.0), (0.0, 0.0));
        let (inner, outer) = burst_stroke(BURST * 0.3);
        assert!(inner == 0.0 && outer > 0.5);
        let (inner, outer) = burst_stroke(BURST);
        assert!((inner - 1.0).abs() < 1e-5 && (outer - 1.0).abs() < 1e-5);
    }

    #[test]
    fn burst_points_go_round_evenly_and_point_out() {
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(120.0, 30.0));
        let points = ring_points(rect, 15.0, 12);
        assert_eq!(points.len(), 12);
        for (p, d) in &points {
            assert!(rect.expand(0.5).contains(*p));
            let away = *p - rect.center();
            assert!(away.x * d.x + away.y * d.y > 0.0);
        }
    }

    #[test]
    fn the_hole_of_an_o_is_found() {
        let (w, h) = (40, 40);
        let mut ink = vec![false; w * h];
        for y in 0..h {
            for x in 0..w {
                let d = ((x as f32 - 19.5).powi(2) + (y as f32 - 19.5).powi(2)).sqrt();
                ink[y * w + x] = (10.0..18.0).contains(&d);
            }
        }
        let (cx, cy, r) = hole_in(&ink, w, h).unwrap();
        assert!((cx - 20.0).abs() < 1.5 && (cy - 20.0).abs() < 1.5 && (8.0..11.0).contains(&r), "{cx} {cy} {r}");
    }

    fn area(triangles: &[egui::Pos2]) -> f32 {
        triangles.chunks(3).map(|t| ((t[1] - t[0]).x * (t[2] - t[0]).y - (t[1] - t[0]).y * (t[2] - t[0]).x).abs() / 2.0).sum()
    }

    #[test]
    fn a_ring_is_filled_round_its_hole() {
        let circle = |r: f32, reverse: bool| {
            let mut points: Vec<egui::Pos2> =
                (0..64).map(|i| i as f32 * std::f32::consts::TAU / 64.0).map(|a| egui::pos2(0.5 + r * a.cos(), 0.5 + r * a.sin())).collect();
            if reverse {
                points.reverse();
            }
            points
        };
        let triangles = fill_triangles(&[circle(0.5, false), circle(0.25, true)]);
        let ring = std::f32::consts::PI * (0.25 - 0.0625);
        assert!((area(&triangles) - ring).abs() < 0.01, "{} vs {ring}", area(&triangles));
    }

    #[test]
    fn the_wordmark_letters_have_outlines() {
        for ch in "pirxcy".chars() {
            let letter = letter(crate::theme::wordmark_font_data(), ch).expect("an outline");
            let filled = area(&letter.triangles);
            // Ink covers part of its box, never more than all of it.
            assert!(filled > 0.15 && filled < 1.0, "{ch}: {filled}");
            for p in letter.contours.iter().flatten() {
                assert!((-1e-3..=1.001).contains(&p.x) && (-1e-3..=1.001).contains(&p.y));
            }
        }
        // The "p" is a ring round its counter, so it has an inner outline.
        assert!(letter(crate::theme::wordmark_font_data(), 'p').unwrap().contours.len() >= 2);
    }

    #[test]
    fn a_burst_stroke_stops_short_of_a_neighbour() {
        let next = [egui::Rect::from_min_max(egui::pos2(20.0, -5.0), egui::pos2(40.0, 5.0))];
        assert_eq!(ray_room(egui::pos2(10.0, 0.0), egui::vec2(1.0, 0.0), 30.0, &next), 10.0);
        assert_eq!(ray_room(egui::pos2(10.0, 0.0), egui::vec2(-1.0, 0.0), 30.0, &next), 30.0);
        assert_eq!(ray_room(egui::pos2(10.0, 0.0), egui::vec2(0.0, 1.0), 30.0, &next), 30.0);
        assert_eq!(ray_room(egui::pos2(25.0, 0.0), egui::vec2(1.0, 0.0), 30.0, &next), 0.0);
    }

    #[test]
    fn the_splash_can_be_pinned_for_a_check() {
        assert_eq!(parse_override("bounce"), (Some(Entrance::Bounce), 0.0));
        assert_eq!(parse_override("Zoom, 4.5"), (Some(Entrance::Bounce), 4.5));
        assert_eq!(parse_override("split"), (None, 0.0));
        assert_eq!(parse_override(""), (None, 0.0));
        assert_eq!(parse_override("puff,nan"), (Some(Entrance::Puff), 0.0));
    }

    #[test]
    fn puff_settles_into_focus() {
        assert_eq!(puff(0.0), (1.6, 18.0, 0.0));
        assert_eq!(puff(PUFF_IN), (1.0, 0.0, 1.0));
        assert_eq!(puff(5.0), (1.0, 0.0, 1.0));
    }
}
