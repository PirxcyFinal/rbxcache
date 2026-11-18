//! HLSL sources for each sky effect.

use super::gif;
use anyhow::Result;
use std::collections::BTreeMap;
use std::path::Path;

const PIXEL: &str = include_str!("hlsl/pixel.hlsl");
const VERTEX: &str = include_str!("hlsl/vertex.hlsl");
/// The classic sky, drawn when a game has no Atmosphere; see [`classic_pixel_source`].
const CLASSIC_PIXEL: &str = include_str!("hlsl/classic_pixel.hlsl");
const CLASSIC_VERTEX: &str = include_str!("hlsl/classic_vertex.hlsl");

pub struct EffectSrc {
    pub functions: String,
    pub pixel: String,
    /// `None` leaves Roblox's vertex shader in place.
    pub vertex: Option<&'static str>,
    pub defaults: BTreeMap<&'static str, f32>,
}

/// The storm's sheets of rain under a grey overcast, slower and blurred, without the lightning.
pub fn rain() -> EffectSrc {
    EffectSrc {
        functions: include_str!("hlsl/rain_functions.hlsl").to_string(),
        pixel: include_str!("hlsl/rain_effect.hlsl").to_string(),
        vertex: None,
        // RAIN and SLANT match the storm's. SOFT draws the streaks blurred and always easing in or out.
        defaults: map(&[("RAIN", 2.8), ("SPEED", 0.5), ("SLANT", 0.34), ("SOFT", 1.0), ("DARK", 1.0), ("MIST", 1.0)]),
    }
}

/// Heavy wind-driven rain, dark cloud and lightning.
pub fn storm() -> EffectSrc {
    EffectSrc {
        functions: include_str!("hlsl/rain_functions.hlsl").to_string(),
        pixel: include_str!("hlsl/storm_effect.hlsl").to_string(),
        vertex: None,
        defaults: map(&[("RAIN", 2.8), ("LIGHTNING", 1.0), ("SPEED", 1.0), ("SLANT", 0.34), ("SOFT", 0.0)]),
    }
}

pub fn snow() -> EffectSrc {
    EffectSrc {
        functions: include_str!("hlsl/snow_functions.hlsl").to_string(),
        pixel: include_str!("hlsl/snow_effect.hlsl").to_string(),
        vertex: None,
        defaults: map(&[("AMOUNT", 1.0), ("SPEED", 1.0), ("DENSITY", 0.45), ("SLANT", 0.14)]),
    }
}

pub fn aurora() -> EffectSrc {
    EffectSrc {
        functions: include_str!("hlsl/aurora_functions.hlsl").to_string(),
        pixel: include_str!("hlsl/aurora_effect.hlsl").to_string(),
        vertex: None,
        // SPEED is how far the curtains sway. It replaced a fast ripple, so it is larger than it was.
        defaults: map(&[("SPEED", 0.9), ("TOP", 62.0), ("DIM", 0.08), ("GLOW", 0.75), ("STARS", 0.8)]),
    }
}

/// Looking up into a cherry tree in bloom, with more round the horizon, and petals on the air.
pub fn sakura() -> EffectSrc {
    EffectSrc {
        functions: include_str!("hlsl/sakura_functions.hlsl").to_string(),
        pixel: include_str!("hlsl/sakura_effect.hlsl").to_string(),
        vertex: None,
        // PETALS is the share of places that have a petal in them; TREES scales the trees.
        // LEVEL brings the whole sky down to an ordinary sky's brightness.
        // GLIDE scales how far the petals glide down the sky each time the clock runs up, and
        // SWAYS is how many times they swing side to side on the way.
        defaults: map(&[("PETALS", 0.70), ("SPEED", 1.0), ("GLIDE", 1.0), ("SWAYS", 0.8), ("TREES", 1.0), ("LEVEL", 0.58)]),
    }
}

/// A city at night round the horizon: windows and neon that come and go, in a very light rain.
pub fn neo_tokyo() -> EffectSrc {
    EffectSrc {
        // The city draws its rain with the rain's own functions. The glyphs are the characters
        // on its signs, made by `skies/neotokyo_glyphs.py`.
        functions: [
            include_str!("hlsl/rain_functions.hlsl"),
            include_str!("hlsl/neotokyo_glyphs.hlsl"),
            include_str!("hlsl/neotokyo_functions.hlsl"),
        ]
        .concat(),
        pixel: include_str!("hlsl/neotokyo_effect.hlsl").to_string(),
        vertex: None,
        // RAIN, SPEED, SLANT and SOFT are the rain's (see `rain`), turned well down: a drizzle
        // that falls nearly straight. NEON is how bright the signs burn; LEVEL scales the whole sky.
        defaults: map(&[("RAIN", 0.45), ("SPEED", 0.4), ("SLANT", 0.10), ("SOFT", 1.0), ("NEON", 1.0), ("LEVEL", 1.0)]),
    }
}

/// A clear blue night full of stars, with now and then a shooting star.
pub fn shooting_stars() -> EffectSrc {
    EffectSrc {
        functions: include_str!("hlsl/stars_functions.hlsl").to_string(),
        pixel: include_str!("hlsl/stars_effect.hlsl").to_string(),
        vertex: None,
        // STARS scales the stars, MILKY the band of the galaxy, METEORS how many shooting stars
        // there are, and LEVEL the whole sky.
        defaults: map(&[("STARS", 1.0), ("MILKY", 1.0), ("METEORS", 1.0), ("LEVEL", 1.0)]),
    }
}

/// Paper lanterns rising into a night sky over a still lake that gives them back.
pub fn lanterns() -> EffectSrc {
    EffectSrc {
        functions: include_str!("hlsl/lanterns_functions.hlsl").to_string(),
        pixel: include_str!("hlsl/lanterns_effect.hlsl").to_string(),
        vertex: None,
        // LANTERNS scales how many there are; LEVEL the whole sky.
        defaults: map(&[("LANTERNS", 1.0), ("LEVEL", 1.0)]),
    }
}

/// The sun low over the sea, clouds lit from beneath, the sea glittering toward the sun.
pub fn sunset_ocean() -> EffectSrc {
    EffectSrc {
        functions: include_str!("hlsl/sunset_functions.hlsl").to_string(),
        pixel: include_str!("hlsl/sunset_effect.hlsl").to_string(),
        vertex: None,
        // SUN scales the sun's disc and the glints off the sea, CLOUDS the clouds, LEVEL the whole sky.
        defaults: map(&[("SUN", 1.0), ("CLOUDS", 1.0), ("LEVEL", 0.9)]),
    }
}

/// A striped neon sun behind wireframe mountains over a glowing grid.
pub fn synthwave() -> EffectSrc {
    EffectSrc {
        functions: include_str!("hlsl/synthwave_functions.hlsl").to_string(),
        pixel: include_str!("hlsl/synthwave_effect.hlsl").to_string(),
        vertex: None,
        // SIZE scales the sun, GRID how close the grid's lines are, LEVEL the whole sky.
        defaults: map(&[("SIZE", 1.0), ("GRID", 1.0), ("LEVEL", 0.9)]),
    }
}

/// A huge red moon over bare trees, fog low on the ground and bats.
pub fn blood_moon() -> EffectSrc {
    EffectSrc {
        functions: include_str!("hlsl/bloodmoon_functions.hlsl").to_string(),
        pixel: include_str!("hlsl/bloodmoon_effect.hlsl").to_string(),
        vertex: None,
        // SIZE scales the moon, CLOUDS the cloud, FOG the fog, LEVEL the whole sky.
        defaults: map(&[("SIZE", 1.0), ("CLOUDS", 1.0), ("FOG", 1.0), ("LEVEL", 1.0)]),
    }
}

pub fn gif(path: &Path) -> Result<EffectSrc> {
    gif::make(path)
}

fn map(pairs: &[(&'static str, f32)]) -> BTreeMap<&'static str, f32> {
    pairs.iter().copied().collect()
}

fn defines(defaults: &BTreeMap<&'static str, f32>, flags: &[(&str, i32)]) -> String {
    let mut out = String::new();
    for (key, value) in defaults {
        out.push_str(&format!("#define {key} {value}\n"));
    }
    for (key, value) in flags {
        out.push_str(&format!("#define {key} {value}\n"));
    }
    out
}

pub fn pixel_source(name: &str, effect: &EffectSrc) -> String {
    let text = PIXEL.replace("@FUNCTIONS@", &effect.functions).replace("@EFFECT@", &effect.pixel);
    let head = defines(
        &effect.defaults,
        &[
            ("RGBE", i32::from(name.contains("RGBE"))),
            ("LINEAR", i32::from(name.contains("Linear"))),
            ("ENV", i32::from(name.contains("Env"))),
            ("HAS_EFFECT", 1),
        ],
    );
    head + &text
}

pub fn vertex_source(effect: &EffectSrc) -> Option<String> {
    let move_body = effect.vertex?;
    Some(defines(&effect.defaults, &[]) + &VERTEX.replace("@MOVE@", move_body))
}

/// A pixel shader of the classic sky (`SkyFS`, `SkyLinearFS`, `SkyRGBEFS` and the rest), the
/// one a game without an Atmosphere draws its skybox with. Its gamma variants always use the
/// vertex colour; its linear ones only when the name says `VColor`.
pub fn classic_pixel_source(name: &str, effect: &EffectSrc) -> String {
    let linear = name.contains("Linear");
    let text = CLASSIC_PIXEL.replace("@FUNCTIONS@", &effect.functions).replace("@EFFECT@", &effect.pixel);
    let head = defines(
        &effect.defaults,
        &[
            ("RGBE", i32::from(name.contains("RGBE"))),
            ("LINEAR", i32::from(linear)),
            ("VCOLOR", i32::from(!linear || name.contains("VColor"))),
            ("ENV", 0),
            ("HAS_EFFECT", 1),
        ],
    );
    head + &text
}

/// The classic sky's vertex shader. It is always replaced: Roblox's own gives its pixel
/// shaders no direction to draw an effect by.
pub fn classic_vertex_source(effect: &EffectSrc) -> String {
    defines(&effect.defaults, &[]) + &CLASSIC_VERTEX.replace("@MOVE@", effect.vertex.unwrap_or("return position;"))
}
