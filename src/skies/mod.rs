//! Animated skies: rebuild Roblox's Direct3D sky shaders entirely inside rbxcache.
//!
//! Rain, snow, aurora and GIF effects compile with Windows' own d3dcompiler and
//! are installed into every Roblox client on the machine. Experimental; works
//! best on NVIDIA GPUs.

#[cfg(windows)]
mod d3d;
#[cfg(windows)]
mod effects;
#[cfg(windows)]
mod gif;
#[cfg(windows)]
mod install;
#[cfg(windows)]
mod pack;

use anyhow::{bail, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Built-in sky effects the Skies page offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Rain,
    Snow,
    Storm,
    Aurora,
    Sakura,
    NeoTokyo,
    ShootingStars,
    Lanterns,
    SunsetOcean,
    Synthwave,
    BloodMoon,
    Gif,
    Off,
}

impl Effect {
    pub fn label(self) -> &'static str {
        match self {
            Effect::Rain => "Rain",
            Effect::Snow => "Snow",
            Effect::Storm => "Storm",
            Effect::Aurora => "Aurora",
            Effect::Sakura => "Sakura",
            Effect::NeoTokyo => "Neo Tokyo",
            Effect::ShootingStars => "Shooting Stars",
            Effect::Lanterns => "Lantern Festival",
            Effect::SunsetOcean => "Sunset Ocean",
            Effect::Synthwave => "Synthwave",
            Effect::BloodMoon => "Blood Moon",
            Effect::Gif => "GIF",
            Effect::Off => "Off",
        }
    }

    /// The skies on the Skies page, in the order they are shown: the best looking first, then
    /// the ones still being worked on.
    pub const OFFERED: [Effect; 11] = [
        Effect::NeoTokyo,
        Effect::Aurora,
        Effect::Synthwave,
        Effect::BloodMoon,
        Effect::Rain,
        Effect::Snow,
        Effect::Storm,
        Effect::Sakura,
        Effect::ShootingStars,
        Effect::Lanterns,
        Effect::SunsetOcean,
    ];

    /// Whether the sky is one of the best looking; the rest are marked as work in progress.
    pub fn best_looking(self) -> bool {
        matches!(self, Effect::NeoTokyo | Effect::Aurora | Effect::Synthwave | Effect::BloodMoon)
    }

    /// The sky pirxcy recommends.
    pub fn recommended(self) -> bool {
        self == Effect::NeoTokyo
    }

    /// The name an applied sky is saved under. Skies are only applied on Windows.
    #[cfg(windows)]
    fn key(self) -> &'static str {
        match self {
            Effect::Rain => "rain",
            Effect::Snow => "snow",
            Effect::Storm => "storm",
            Effect::Aurora => "aurora",
            Effect::Sakura => "sakura",
            Effect::NeoTokyo => "neotokyo",
            Effect::ShootingStars => "shootingstars",
            Effect::Lanterns => "lanterns",
            Effect::SunsetOcean => "sunsetocean",
            Effect::Synthwave => "synthwave",
            Effect::BloodMoon => "bloodmoon",
            Effect::Gif => "gif",
            Effect::Off => "off",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct State {
    #[serde(default)]
    pub effect: String,
}

/// Runs as a shader-compiling worker for a sky build when started as one, and returns the exit
/// code; `None` for an ordinary start.
#[cfg(windows)]
pub fn compile_worker() -> Option<i32> {
    pack::worker_main()
}

pub fn state_path() -> Result<PathBuf> {
    #[cfg(windows)]
    {
        Ok(install::store_dir()?.join("state.json"))
    }
    #[cfg(not(windows))]
    {
        bail!("Skies are only available on Windows")
    }
}

pub fn load_state() -> Option<State> {
    let path = state_path().ok()?;
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Lists every Roblox client that can receive a sky pack.
pub fn list_clients() -> Result<String> {
    #[cfg(windows)]
    {
        install::list_clients()
    }
    #[cfg(not(windows))]
    {
        bail!("Skies are only available on Windows")
    }
}

/// Applies `effect` to every Roblox client. For [`Effect::Gif`], `gif` must be set.
pub fn apply(effect: Effect, gif: Option<&Path>) -> Result<String> {
    #[cfg(not(windows))]
    {
        let _ = (effect, gif);
        bail!("Skies are only available on Windows")
    }
    #[cfg(windows)]
    {
        let started = std::time::Instant::now();
        crate::log::write(&format!("--- sky: applying {} (rbxcache {})", effect.label(), env!("CARGO_PKG_VERSION")));
        let result = apply_windows(effect, gif);
        match &result {
            Ok(_) => crate::log::write(&format!("sky: {} done in {} ms", effect.label(), started.elapsed().as_millis())),
            Err(e) => crate::log::write(&format!("sky: {} FAILED after {} ms: {e:#}", effect.label(), started.elapsed().as_millis())),
        }
        result
    }
}

#[cfg(windows)]
fn apply_windows(effect: Effect, gif: Option<&Path>) -> Result<String> {
    use crate::log::write as log;
    if effect == Effect::Gif && gif.is_none() {
        bail!("Pick a GIF file first");
    }
    // Roblox holds its shader pack open, so a running client is closed, but only once the new
    // pack is ready: a sky that fails to build leaves the game running.
    let close = || {
        let closed = crate::store::close_player();
        log(if closed { "closed the running Roblox client" } else { "Roblox was not running" });
        if closed {
            " Roblox was open, so it was closed."
        } else {
            ""
        }
    };
    if effect == Effect::Off {
        let note = close();
        return Ok(install::remove_pack()? + note);
    }
    let step = std::time::Instant::now();
    let src = match effect {
        Effect::Rain => effects::rain(),
        Effect::Snow => effects::snow(),
        Effect::Storm => effects::storm(),
        Effect::Aurora => effects::aurora(),
        Effect::Sakura => effects::sakura(),
        Effect::NeoTokyo => effects::neo_tokyo(),
        Effect::ShootingStars => effects::shooting_stars(),
        Effect::Lanterns => effects::lanterns(),
        Effect::SunsetOcean => effects::sunset_ocean(),
        Effect::Synthwave => effects::synthwave(),
        Effect::BloodMoon => effects::blood_moon(),
        Effect::Gif => effects::gif(gif.expect("checked"))?,
        Effect::Off => unreachable!(),
    };
    log(&format!("effect source ready in {} ms", step.elapsed().as_millis()));
    let (path, original) = install::prepare_original()?;
    log(&format!("original pack {} ({} MB)", path.display(), original.len() / (1024 * 1024)));
    let edited = pack::rebuild(&original, &src)?;
    log(&format!("rebuilt pack is {} MB", edited.len() / (1024 * 1024)));
    let note = close();
    Ok(install::install_pack(&edited, effect.key())? + note)
}

#[cfg(test)]
mod tests {
    /// Every sky is offered once, the best looking come first, and the one recommended is
    /// among them.
    #[test]
    fn the_skies_page_offers_each_sky_once() {
        use super::Effect;
        let offered = Effect::OFFERED;
        for (k, effect) in offered.iter().enumerate() {
            assert!(!offered[k + 1..].contains(effect), "{effect:?} twice");
            assert!(!matches!(effect, Effect::Gif | Effect::Off));
        }
        let first_wip = offered.iter().position(|e| !e.best_looking()).unwrap();
        assert!(offered[first_wip..].iter().all(|e| !e.best_looking()));
        assert_eq!(offered.iter().filter(|e| e.recommended()).count(), 1);
        assert!(offered.iter().filter(|e| e.recommended()).all(|e| e.best_looking()));
    }

    #[test]
    fn hlsl_templates_are_embedded() {
        assert!(include_str!("hlsl/pixel.hlsl").contains("@EFFECT@"));
        assert!(include_str!("hlsl/storm_effect.hlsl").contains("rain"));
        assert!(include_str!("hlsl/snow_effect.hlsl").contains("snow"));
        assert!(include_str!("hlsl/aurora_effect.hlsl").contains("swing"));
    }

    #[cfg(windows)]
    #[test]
    fn rain_draws_the_storms_sheets() {
        let (rain, storm) = (super::effects::rain(), super::effects::storm());
        assert_eq!(rain.functions, storm.functions);
        for key in ["RAIN", "SLANT"] {
            assert_eq!(rain.defaults[key], storm.defaults[key], "{key}");
        }
        // Soft for the rain, crisp for the storm. Anything between would make the rain pulse.
        assert_eq!((rain.defaults["SOFT"], storm.defaults["SOFT"]), (1.0, 0.0));
    }

    /// The glyph table goes to the shader compiler as it is, so it stays plain ASCII, and the
    /// city's functions come after it. Each row of a character is one uint4 (32 values of 4
    /// bits), which is what lets the shader read a character in two lookups.
    #[cfg(windows)]
    #[test]
    fn neo_tokyo_glyphs_are_plain_ascii_and_come_first() {
        let glyphs = include_str!("hlsl/neotokyo_glyphs.hlsl");
        assert!(glyphs.is_ascii());
        assert!(glyphs.contains("static const uint GLYPH_SIZE = 32;"));
        let city = super::effects::neo_tokyo();
        assert!(city.functions.find("GLYPH_DATA[").unwrap() < city.functions.find("float RowTexel").unwrap());
    }

    /// Neo Tokyo's drizzle is the rain's own soft streaks, drawn by the rain's functions.
    #[cfg(windows)]
    #[test]
    fn neo_tokyo_drizzles_with_the_rains_streaks() {
        let (rain, city) = (super::effects::rain(), super::effects::neo_tokyo());
        assert!(city.functions.starts_with(&rain.functions));
        assert_eq!(city.defaults["SOFT"], 1.0);
        assert!(city.defaults["RAIN"] < rain.defaults["RAIN"] / 4.0);
    }

    #[cfg(windows)]
    #[test]
    fn rebuilds_snow_from_a_real_pack_when_present() {
        let local = std::env::var_os("LOCALAPPDATA").expect("LOCALAPPDATA");
        let pack = std::path::PathBuf::from(local).join("Roblox/Versions/version-4ea8e85481e34f46/shaders/shaders_d3d11.pack");
        if !pack.is_file() {
            return;
        }
        let original = std::fs::read(&pack).unwrap();
        let edited = super::pack::rebuild(&original, &super::effects::snow()).unwrap();
        assert!(edited.len() > original.len());
    }

    /// A rebuilt pack is told apart from Roblox's own, so a rebuilt one is never saved as the
    /// original nor kept when the sky is turned off.
    #[cfg(windows)]
    #[test]
    fn a_rebuilt_pack_is_told_from_roblox_own_when_present() {
        let local = std::env::var_os("LOCALAPPDATA").expect("LOCALAPPDATA");
        let pack = std::path::PathBuf::from(local).join("Roblox/Versions/version-4ea8e85481e34f46/shaders/shaders_d3d11.pack");
        if !pack.is_file() {
            return;
        }
        let backup = pack.with_extension("pack.rbxcache.bak");
        let original = std::fs::read(if backup.is_file() { &backup } else { &pack }).unwrap();
        let edited = super::pack::rebuild(&original, &super::effects::synthwave()).unwrap();
        assert!(super::pack::is_rebuilt(&edited).unwrap());
        if backup.is_file() {
            assert!(!super::pack::is_rebuilt(&original).unwrap());
        }
        // A pack cut short is refused rather than read past its end.
        assert!(super::pack::is_rebuilt(&original[..original.len() / 2]).is_err());
    }

    /// Builds a GIF sky from a made-up animation and checks it compiles in a sensible time.
    #[cfg(windows)]
    #[test]
    fn a_gif_sky_builds_quickly_against_a_real_pack_when_present() {
        use image::codecs::gif::GifEncoder;
        use image::{Frame, Rgba, RgbaImage};
        let local = std::env::var_os("LOCALAPPDATA").expect("LOCALAPPDATA");
        let pack = std::path::PathBuf::from(local).join("Roblox/Versions/version-4ea8e85481e34f46/shaders/shaders_d3d11.pack");
        if !pack.is_file() {
            return;
        }
        let path = std::env::temp_dir().join("rbxcache-test-sky.gif");
        {
            let mut encoder = GifEncoder::new(std::fs::File::create(&path).unwrap());
            let frames: Vec<Frame> = (0..12u32)
                .map(|n| {
                    let image = RgbaImage::from_fn(240, 120, |x, y| Rgba([((x + n * 20) % 256) as u8, (y * 2 % 256) as u8, (n * 20) as u8, 255]));
                    Frame::new(image)
                })
                .collect();
            encoder.encode_frames(frames).unwrap();
        }
        let started = std::time::Instant::now();
        let src = super::effects::gif(&path).unwrap();
        let original = std::fs::read(&pack).unwrap();
        let edited = super::pack::rebuild(&original, &src).unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(edited.len() > original.len());
        assert!(started.elapsed().as_secs() < 60, "took {:?}", started.elapsed());
    }

    #[cfg(windows)]
    #[test]
    fn the_classic_sky_is_found_and_nothing_else() {
        use super::pack::sky_family;
        for name in ["SkyFS", "SkyLinearFS", "SkyLinearVColorFS", "SkyRGBEFS", "SkyRGBELinearFS", "SkyRGBELinearVColorFS", "SkyVS"] {
            assert_eq!(sky_family(name), Some(true), "{name}");
        }
        assert_eq!(sky_family("AdvSkyLinearFS"), Some(false));
        for name in ["SmoothClusterFS", "StarsF", "AdvSunFS", "SkyboxDebugCS", "SpecularConvFS"] {
            assert_eq!(sky_family(name), None, "{name}");
        }
    }

    /// Every classic sky variant and its vertex shader compile for every effect, and the
    /// pixel shaders read the direction the vertex shader hands on.
    #[cfg(windows)]
    #[test]
    fn the_classic_sky_compiles_for_every_effect() {
        let names = ["SkyFS", "SkyLinearFS", "SkyLinearVColorFS", "SkyRGBEFS", "SkyRGBELinearFS", "SkyRGBELinearVColorFS"];
        for (effect, src) in [
            ("rain", super::effects::rain()),
            ("storm", super::effects::storm()),
            ("snow", super::effects::snow()),
            ("aurora", super::effects::aurora()),
            ("sakura", super::effects::sakura()),
            ("neotokyo", super::effects::neo_tokyo()),
            ("shootingstars", super::effects::shooting_stars()),
            ("lanterns", super::effects::lanterns()),
            ("sunsetocean", super::effects::sunset_ocean()),
            ("synthwave", super::effects::synthwave()),
            ("bloodmoon", super::effects::blood_moon()),
        ] {
            super::d3d::compile(&super::effects::classic_vertex_source(&src), "main", "vs_5_0").unwrap_or_else(|e| panic!("{effect} SkyVS: {e:#}"));
            for name in names {
                super::d3d::compile(&super::effects::classic_pixel_source(name, &src), "main", "ps_5_0")
                    .unwrap_or_else(|e| panic!("{effect} {name}: {e:#}"));
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn every_effect_compiles_against_a_real_pack_when_present() {
        let local = std::env::var_os("LOCALAPPDATA").expect("LOCALAPPDATA");
        let pack = std::path::PathBuf::from(local).join("Roblox/Versions/version-4ea8e85481e34f46/shaders/shaders_d3d11.pack");
        if !pack.is_file() {
            return;
        }
        let original = std::fs::read(&pack).unwrap();
        for (name, src) in [
            ("rain", super::effects::rain()),
            ("storm", super::effects::storm()),
            ("aurora", super::effects::aurora()),
            ("sakura", super::effects::sakura()),
            ("neotokyo", super::effects::neo_tokyo()),
            ("shootingstars", super::effects::shooting_stars()),
            ("lanterns", super::effects::lanterns()),
            ("sunsetocean", super::effects::sunset_ocean()),
            ("synthwave", super::effects::synthwave()),
            ("bloodmoon", super::effects::blood_moon()),
        ] {
            let edited = super::pack::rebuild(&original, &src).unwrap_or_else(|e| panic!("{name}: {e:#}"));
            assert!(edited.len() > original.len(), "{name}");
        }
    }
}
