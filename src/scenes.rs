//! What the loading screen draws for the zoom through the "p", and the burst round a
//! hovered control.

use crate::motion;
use crate::theme::{self, alpha};
use eframe::egui::{self, pos2, vec2, Color32, Pos2, Rect, Shape, Stroke, Vec2};
use std::f32::consts::{FRAC_PI_2, TAU};

// The wordmark from its letters' outlines.

/// Where each letter of `galley` puts its ink when the galley is drawn at `at`.
fn letter_boxes(galley: &egui::Galley, at: Pos2) -> impl Iterator<Item = (char, Rect)> + '_ {
    galley.rows.iter().flat_map(|row| row.glyphs.iter()).filter(|g| !g.uv_rect.is_nothing()).map(move |g| {
        let min = at + g.pos.to_vec2() + g.uv_rect.offset;
        (g.chr, Rect::from_min_size(pos2(min.x.round(), min.y.round()), g.uv_rect.size))
    })
}

/// The wordmark laid out as `galley` at `at`, drawn from its letters' outlines so it stays
/// sharp however far it is scaled. `edge` is the width of the soft line round each letter, in
/// points before any scaling. None when a letter has no outline, to draw the galley instead.
pub fn wordmark(galley: &egui::Galley, at: Pos2, color: Color32, edge: f32) -> Option<Vec<Shape>> {
    let mut mesh = egui::Mesh::default();
    let mut outlines = Vec::new();
    for (ch, ink) in letter_boxes(galley, at) {
        let letter = motion::letter(theme::wordmark_font_data(), ch)?;
        let place = |p: &Pos2| ink.min + vec2(p.x * ink.width(), p.y * ink.height());
        let start = mesh.vertices.len() as u32;
        for p in &letter.triangles {
            mesh.colored_vertex(place(p), color);
        }
        mesh.indices.extend(start..mesh.vertices.len() as u32);
        for contour in &letter.contours {
            outlines.push(Shape::closed_line(contour.iter().map(place).collect(), Stroke::new(edge, color)));
        }
    }
    let mut shapes = vec![Shape::mesh(mesh)];
    shapes.extend(outlines);
    Some(shapes)
}

/// The hole in the wordmark's "p" when `galley` is drawn at `at`: its middle, the radius of
/// the round hole that fits inside it, and how far the inside of the letter reaches from that
/// middle along `spokes` evenly spaced directions, clockwise from the right.
pub fn wordmark_hole(galley: &egui::Galley, at: Pos2, spokes: usize) -> Option<(Pos2, f32, Vec<f32>)> {
    let (_, ink) = letter_boxes(galley, at).find(|(ch, _)| *ch == 'p')?;
    let (across, down, radius) = motion::p_hole(theme::wordmark_font_data());
    let middle = ink.min + vec2(across * ink.width(), down * ink.height());
    let radius = radius * ink.height();
    let place = |p: &Pos2| ink.min + vec2(p.x * ink.width(), p.y * ink.height());
    let edges: Vec<(Pos2, Pos2)> = motion::letter(theme::wordmark_font_data(), 'p')
        .map(|letter| letter.contours.iter().flat_map(|c| (0..c.len()).map(move |i| (place(&c[i]), place(&c[(i + 1) % c.len()])))).collect())
        .unwrap_or_default();
    // Along each direction the nearest outline is the inside of the letter's bowl.
    let reach = (0..spokes)
        .map(|spoke| {
            let angle = spoke as f32 * TAU / spokes as f32;
            let dir = vec2(angle.cos(), angle.sin());
            edges
                .iter()
                .filter_map(|&(a, b)| {
                    let (side, along) = (b - a, a - middle);
                    let cross = dir.x * side.y - dir.y * side.x;
                    if cross.abs() < 1e-6 {
                        return None;
                    }
                    let t = (along.x * side.y - along.y * side.x) / cross;
                    let u = (along.x * dir.y - along.y * dir.x) / cross;
                    (t > 0.0 && (0.0..=1.0).contains(&u)).then_some(t)
                })
                .fold(f32::MAX, f32::min)
        })
        .map(|t| if t == f32::MAX || t < radius * 0.5 { radius } else { t })
        .collect();
    Some((middle, radius, reach))
}

// The divider between things on a page (Motion Prompt #165, MIT License (c) 2026 Shohei
// Ohtaki, https://showheyohtaki.github.io/motion-prompt/): two layered waves that keep
// swaying slowly, the back one fainter.

/// The height a divider takes on the page.
pub const DIVIDER_HEIGHT: f32 = 38.0;

/// The two waves of a divider across `rect`, `t` seconds in. Each is filled from its crest
/// down, fading out before the next thing on the page, with a fine line along the crest. The
/// ends fade too, so the divider does not stop short at the sides.
pub fn wave_divider(rect: Rect, t: f32) -> Shape {
    // The middle lines are 10 apart, as the reference has them.
    let back_middle = rect.top() + 13.0;
    let front_middle = back_middle + 10.0;
    let ends = |x: f32| {
        let inside = ((x - rect.left()).min(rect.right() - x) / 36.0).clamp(0.0, 1.0);
        inside * inside * (3.0 - 2.0 * inside)
    };
    let mut xs = Vec::new();
    let mut x = rect.left();
    while x < rect.right() {
        xs.push(x);
        x += 4.0;
    }
    xs.push(rect.right());
    let mut shapes = Vec::new();
    // The back wave first, faint; the front one over it.
    for (back, fill, line) in [
        (true, alpha(theme::HIGHLIGHT, 0.45 * 0.32), alpha(theme::HIGHLIGHT, 0.45 * 0.5)),
        (false, alpha(theme::ACCENT, 0.34), alpha(theme::HIGHLIGHT, 0.6)),
    ] {
        let crest = |x: f32| {
            let (b, f) = motion::divider_waves(x - rect.left(), t);
            if back {
                back_middle + b
            } else {
                front_middle + f
            }
        };
        let mut mesh = egui::Mesh::default();
        let mut points = Vec::with_capacity(xs.len());
        for (i, &x) in xs.iter().enumerate() {
            let y = crest(x);
            let fade = ends(x);
            mesh.colored_vertex(pos2(x, y), fill.gamma_multiply(fade));
            mesh.colored_vertex(pos2(x, rect.bottom()), Color32::TRANSPARENT);
            if i > 0 {
                let at = i as u32 * 2;
                mesh.add_triangle(at - 2, at - 1, at);
                mesh.add_triangle(at - 1, at + 1, at);
            }
            points.push(pos2(x, y));
        }
        shapes.push(Shape::mesh(mesh));
        // The crest line, in short pieces so its ends can fade with the fill.
        for pair in points.windows(2) {
            let fade = ends((pair[0].x + pair[1].x) / 2.0);
            if fade > 0.01 {
                shapes.push(Shape::line_segment([pair[0], pair[1]], Stroke::new(if back { 1.0_f32 } else { 1.3 }, line.gamma_multiply(fade))));
            }
        }
    }
    Shape::Vec(shapes)
}

// The burst round a hovered control.

/// Strokes shooting out from all round a control `t` seconds after the pointer came onto it.
/// Each of `rays` starts at a point, goes the given way, and may be as long as given; it grows
/// out from its inner end and then is drawn in toward its outer end until it is gone.
pub fn line_burst(rays: &[(Pos2, Vec2, f32)], t: f32) -> Shape {
    let (inner, outer) = motion::burst_stroke(t);
    if outer <= inner {
        return Shape::Noop;
    }
    // A little dimmer as the strokes leave, so the last of them does not hang about.
    let fade = 1.0 - 0.5 * motion::progress(t / motion::BURST, 0.6, 1.0);
    let mut shapes = Vec::with_capacity(rays.len() * 2);
    for &(from, dir, length) in rays {
        let (a, b) = (from + dir * (length * inner), from + dir * (length * outer));
        shapes.push(Shape::line_segment([a, b], Stroke::new(4.2_f32, alpha(theme::ACCENT, 0.32 * fade))));
        shapes.push(Shape::line_segment([a, b], Stroke::new(1.8_f32, alpha(theme::HIGHLIGHT.lerp_to_gamma(Color32::WHITE, 0.45), fade))));
    }
    Shape::Vec(shapes)
}

/// The polygon version, for a small square control: little diamonds thrown out along the
/// same lines, turning, shrinking and fading as they go.
pub fn shape_burst(rays: &[(Pos2, Vec2, f32)], t: f32) -> Shape {
    let x = motion::progress(t / motion::BURST, 0.0, 1.0);
    if x >= 1.0 {
        return Shape::Noop;
    }
    let travel = motion::ease_out(x);
    let size = 3.2 * (1.0 - motion::ease_in(x));
    let fade = 1.0 - motion::ease_in(motion::progress(x, 0.45, 1.0));
    let mut shapes = Vec::with_capacity(rays.len());
    for (index, &(from, dir, length)) in rays.iter().enumerate() {
        let middle = from + dir * (length * travel);
        let spin = x * 2.4 + index as f32 * 0.7;
        let corners = (0..4)
            .map(|k| {
                let angle = spin + k as f32 * FRAC_PI_2;
                // Longer along the way it flies, so it reads as thrown.
                let reach = if k % 2 == 0 { size * 1.35 } else { size * 0.8 };
                middle + vec2(angle.cos(), angle.sin()) * reach
            })
            .collect();
        let color = if index % 2 == 0 { theme::HIGHLIGHT } else { theme::ACCENT };
        shapes.push(Shape::convex_polygon(corners, alpha(color, fade), Stroke::NONE));
    }
    Shape::Vec(shapes)
}
