//! Embed a GIF's frames inside the sky shader as a colour table.
//!
//! The table has to stay small: a big constant table makes the sky shader slow on every
//! pixel, so the picture is kept to a few small frames and the shader only reads it where
//! the picture actually shows.

use super::effects::EffectSrc;
use anyhow::{bail, Context, Result};
use image::imageops::FilterType;
use image::{AnimationDecoder, RgbaImage};
use std::collections::{BTreeMap, HashMap};
use std::io::Cursor;
use std::path::Path;

/// Rows of four 32-bit words a shader may hold.
const LIMIT: usize = 4096;
/// Frames read from a GIF at most; a longer one is cut off so a huge file cannot hang the build.
const MAX_FRAMES: usize = 600;
const FUNCTIONS: &str = include_str!("hlsl/gif_functions.hlsl");
const EFFECT: &str = include_str!("hlsl/gif_effect.hlsl");

const FRAMES: usize = 6;
const WIDTH: usize = 96;
const HEIGHT: usize = 48;

/// Builds the GIF effect: frames stored in the shader, wrapped round the horizon.
pub fn make(path: &Path) -> Result<EffectSrc> {
    use crate::log::write as log;
    let started = std::time::Instant::now();
    let (palette, indices) = load(path)?;
    log(&format!("gif: decoded and shrunk to {FRAMES} frames of {WIDTH}x{HEIGHT} in {} ms", started.elapsed().as_millis()));
    let words = table(&palette, &indices)?;
    let row_count = words.len() / 4;
    log(&format!("gif: shader table is {row_count} rows (limit {LIMIT})"));
    let rows = words
        .as_chunks::<4>()
        .0
        .iter()
        .map(|row| format!("    uint4({}u, {}u, {}u, {}u)", row[0], row[1], row[2], row[3]))
        .collect::<Vec<_>>()
        .join(",\n");
    let functions = FUNCTIONS.replace("@ROWS@", &row_count.to_string()).replace("@TABLE@", &rows);
    let mut defaults = BTreeMap::new();
    defaults.insert("FRAMES", FRAMES as f32);
    defaults.insert("WIDE", WIDTH as f32);
    defaults.insert("HIGH", HEIGHT as f32);
    defaults.insert("TOP", 62.0);
    defaults.insert("DIM", 0.10);
    defaults.insert("GLOW", 0.85);
    log(&format!("gif: done in {} ms", started.elapsed().as_millis()));
    Ok(EffectSrc { functions, pixel: EFFECT.to_string(), vertex: None, defaults })
}

/// Decodes the GIF, shrinking each frame as it comes so a big GIF never sits in memory whole.
fn load(path: &Path) -> Result<(Vec<[u8; 3]>, Vec<u8>)> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    crate::log::write(&format!("gif: {} is {} KB", path.display(), bytes.len() / 1024));
    let decoder = image::codecs::gif::GifDecoder::new(Cursor::new(bytes)).context("decoding GIF")?;
    let mut source: Vec<RgbaImage> = Vec::new();
    for frame in decoder.into_frames() {
        let frame = frame.context("reading GIF frames")?;
        let picture = frame.into_buffer();
        source.push(image::imageops::resize(&picture, WIDTH as u32, HEIGHT as u32, FilterType::Triangle));
        if source.len() >= MAX_FRAMES {
            crate::log::write(&format!("gif: stopped reading after {MAX_FRAMES} frames"));
            break;
        }
    }
    crate::log::write(&format!("gif: {} frames read", source.len()));
    if source.len() < 2 {
        bail!("{} has only one frame", path.display());
    }
    // Spread the frames we keep evenly through the animation.
    let picked: Vec<&RgbaImage> =
        (0..FRAMES).map(|i| &source[((i as f32) * (source.len() - 1) as f32 / (FRAMES - 1) as f32).round() as usize]).collect();

    let mut sheet = Vec::with_capacity(WIDTH * HEIGHT * FRAMES * 3);
    for picture in picked {
        for pixel in picture.pixels() {
            // Transparent areas become black, which the sky shows as nothing.
            let alpha = f32::from(pixel[3]) / 255.0;
            for channel in 0..3 {
                sheet.push((f32::from(pixel[channel]) * alpha).round() as u8);
            }
        }
    }
    Ok(quantize(&sheet, 256))
}

/// A 256-colour palette by median cut over a 5-bit-per-channel histogram, and every pixel's
/// nearest palette entry.
fn quantize(rgb: &[u8], colours: usize) -> (Vec<[u8; 3]>, Vec<u8>) {
    let key = |c: &[u8]| ((u16::from(c[0] >> 3)) << 10) | ((u16::from(c[1] >> 3)) << 5) | u16::from(c[2] >> 3);
    let mut histogram: HashMap<u16, u32> = HashMap::new();
    for c in rgb.as_chunks::<3>().0 {
        *histogram.entry(key(&c[..])).or_default() += 1;
    }
    let split = |k: u16| [((k >> 10) & 31) as i32, ((k >> 5) & 31) as i32, (k & 31) as i32];

    let mut boxes: Vec<Vec<(u16, u32)>> = vec![histogram.into_iter().collect()];
    while boxes.len() < colours {
        // The box with the most to gain from being split: widest spread, weighted by pixels.
        let mut best: Option<(usize, usize, i64)> = None;
        for (at, entries) in boxes.iter().enumerate() {
            if entries.len() < 2 {
                continue;
            }
            for channel in 0..3 {
                let low = entries.iter().map(|e| split(e.0)[channel]).min().unwrap();
                let high = entries.iter().map(|e| split(e.0)[channel]).max().unwrap();
                let weight: i64 = entries.iter().map(|e| i64::from(e.1)).sum();
                let score = i64::from(high - low) * weight;
                if high > low && best.is_none_or(|b| score > b.2) {
                    best = Some((at, channel, score));
                }
            }
        }
        let Some((at, channel, _)) = best else { break };
        let mut entries = boxes.swap_remove(at);
        entries.sort_by_key(|e| split(e.0)[channel]);
        let total: u64 = entries.iter().map(|e| u64::from(e.1)).sum();
        let mut running = 0u64;
        let mut cut = 1;
        for (i, e) in entries.iter().enumerate() {
            running += u64::from(e.1);
            if running * 2 >= total {
                cut = (i + 1).clamp(1, entries.len() - 1);
                break;
            }
        }
        let upper = entries.split_off(cut);
        boxes.push(entries);
        boxes.push(upper);
    }

    let mut palette: Vec<[u8; 3]> = boxes
        .iter()
        .map(|entries| {
            let total: u64 = entries.iter().map(|e| u64::from(e.1)).sum::<u64>().max(1);
            let mut sum = [0u64; 3];
            for e in entries {
                let parts = split(e.0);
                for c in 0..3 {
                    sum[c] += (parts[c] as u64) * u64::from(e.1);
                }
            }
            // 5-bit back to 8-bit, centred in its bucket.
            [0, 1, 2].map(|c| ((sum[c] / total) as u8) * 8 + 4)
        })
        .collect();
    while palette.len() < colours {
        palette.push([0, 0, 0]);
    }

    // Nearest palette entry, worked out once per distinct colour.
    let mut nearest: HashMap<u16, u8> = HashMap::new();
    let indices = rgb
        .as_chunks::<3>()
        .0
        .iter()
        .map(|c| {
            *nearest.entry(key(&c[..])).or_insert_with(|| {
                let mut best = 0usize;
                let mut best_distance = i32::MAX;
                for (i, colour) in palette.iter().enumerate() {
                    let distance: i32 = (0..3).map(|k| (i32::from(c[k]) - i32::from(colour[k])).pow(2)).sum();
                    if distance < best_distance {
                        best_distance = distance;
                        best = i;
                    }
                }
                best as u8
            })
        })
        .collect();
    (palette, indices)
}

fn table(palette: &[[u8; 3]], indices: &[u8]) -> Result<Vec<u32>> {
    let mut colours: Vec<u32> = palette.iter().map(|c| u32::from(c[0]) | (u32::from(c[1]) << 8) | (u32::from(c[2]) << 16)).collect();
    colours.resize(256, 0);

    let mut pixels = indices.to_vec();
    while !pixels.len().is_multiple_of(4) {
        pixels.push(0);
    }
    let mut packed: Vec<u32> = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| u32::from(p[0]) | (u32::from(p[1]) << 8) | (u32::from(p[2]) << 16) | (u32::from(p[3]) << 24))
        .collect();

    let mut words = colours;
    words.append(&mut packed);
    while !words.len().is_multiple_of(4) {
        words.push(0);
    }
    if words.len() / 4 > LIMIT {
        bail!("too many pixels: the table would have {} rows and a shader may hold {LIMIT}", words.len() / 4);
    }
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantize_keeps_two_distinct_colours_apart() {
        let mut rgb = Vec::new();
        for i in 0..200 {
            rgb.extend_from_slice(if i % 2 == 0 { &[255, 0, 0] } else { &[0, 0, 255] });
        }
        let (palette, indices) = quantize(&rgb, 256);
        assert_ne!(indices[0], indices[1]);
        let red = palette[indices[0] as usize];
        let blue = palette[indices[1] as usize];
        assert!(red[0] > 200 && red[2] < 40, "{red:?}");
        assert!(blue[2] > 200 && blue[0] < 40, "{blue:?}");
    }

    #[test]
    fn the_table_fits_a_shader() {
        let words = (WIDTH * HEIGHT * FRAMES).div_ceil(4) + 256;
        assert!(words.div_ceil(4) <= LIMIT);
    }
}
