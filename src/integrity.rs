//! What rbxcache will not do with 3D content, and the key every write and download carries.
//!
//! rbxcache does not swap 3D models and does not download them. Nothing is written over a
//! mesh, a model, a place, a package, an accessory, a body part or the collision data Roblox
//! keeps for them, and none of those is written into an entry either, whether it comes from
//! the cache, a file, a link or Roblox itself. The same goes for 3D files from other programs:
//! FBX, glTF, OBJ, STL, PLY and Draco. Animations are stored the way models are and are let
//! through when that is all a file holds. Asset downloads that turn out to be 3D are stopped
//! and thrown away, never handed on.
//!
//! Every write to the cache needs a [`Permit`] from [`permit`], and every download passes
//! through [`clear_download`]. Both carry the key the cache's checksums are made with
//! ([`key`]), and that key comes out of the verdicts below.

use std::io::Read;
use std::sync::OnceLock;

/// Asset types that are 3D: Mesh, Hat, Place, Model, Head, Gear, the body parts, Package,
/// SolidModel, MeshPart, the accessories, layered clothing, MeshHiddenSurfaceRemoval,
/// eyebrows, eyelashes and DynamicHead.
const MODEL_TYPES: [u32; 36] =
    [4, 8, 9, 10, 17, 19, 27, 28, 29, 30, 31, 32, 39, 40, 41, 42, 43, 44, 45, 46, 47, 57, 58, 64, 65, 66, 67, 68, 69, 70, 71, 72, 75, 76, 77, 79];

/// The classes an animation is made of. A model file holding nothing else is an animation.
const ANIMATION_CLASSES: [&str; 13] = [
    "KeyframeSequence",
    "Keyframe",
    "Pose",
    "NumberPose",
    "KeyframeMarker",
    "CurveAnimation",
    "Folder",
    "FloatCurve",
    "Vector3Curve",
    "RotationCurve",
    "EulerRotationCurve",
    "MarkerCurve",
    "AnimationRigData",
];

/// The cache category of the collision data Roblox works out for meshes and unions.
const PHYSICS_CATEGORY: i64 = 22;

/// Ties the verdicts to the checksum key; see [`key`].
const ANCHOR: u32 = 0x5f2a_34b1;

/// What a body, an entry or an asset type is, as far as these rules go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Clear,
    Mesh,
    Model,
    Physics,
    /// A 3D file from another program.
    Scene,
    /// An asset type that is 3D.
    Kind(u32),
}

impl Verdict {
    /// Each verdict's number. These go into the key, so they stay as they are.
    fn code(self) -> u32 {
        match self {
            Verdict::Clear => 0x5f3a_91c7,
            Verdict::Mesh => 0x0d1e_5a11,
            Verdict::Model => 0x7b0c_2e94,
            Verdict::Physics => 0x2c6f_d803,
            Verdict::Scene => 0x61a4_47bd,
            Verdict::Kind(t) => 0x3e91_0000 ^ t.wrapping_mul(0x9e37_79b1),
        }
    }

    pub fn refused(self) -> bool {
        self != Verdict::Clear
    }

    /// What it is, for a message.
    pub fn what(self) -> &'static str {
        match self {
            Verdict::Clear => "allowed",
            Verdict::Mesh => "a 3D mesh",
            Verdict::Model => "a 3D model",
            Verdict::Physics => "3D collision data",
            Verdict::Scene => "a 3D file",
            Verdict::Kind(_) => "a 3D asset",
        }
    }
}

/// Nothing for a verdict that lets something through; otherwise how far off the key is.
fn mix(verdict: Verdict) -> u32 {
    verdict.code() ^ Verdict::Clear.code()
}

/// The refusal: what the content was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refused(pub Verdict);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "this is {}, and rbxcache does not swap or download 3D models", self.0.what())
    }
}

impl std::error::Error for Refused {}

/// True when `error` is, or was caused by, a [`Refused`].
pub fn is_refusal(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| cause.downcast_ref::<Refused>().is_some())
}

/// The verdict on an asset type.
pub fn type_verdict(asset_type: u32) -> Verdict {
    if MODEL_TYPES.contains(&asset_type) {
        Verdict::Kind(asset_type)
    } else {
        Verdict::Clear
    }
}

/// The verdict on a body, from what it holds.
pub fn body_verdict(body: &[u8]) -> Verdict {
    verdict_of(body, true)
}

fn verdict_of(body: &[u8], unpack: bool) -> Verdict {
    let at = |offset: usize, pattern: &[u8]| body.len() >= offset + pattern.len() && &body[offset..offset + pattern.len()] == pattern;
    if at(0, b"version ") && body.get(8).is_some_and(u8::is_ascii_digit) || at(0, b"DRACO") {
        return Verdict::Mesh;
    }
    if at(0, b"CSGPHS") || at(0, b"CSGMDL") || at(0, b"CSGK") {
        return Verdict::Physics;
    }
    if at(0, b"<roblox") {
        return if animation_only(body) { Verdict::Clear } else { Verdict::Model };
    }
    if at(0, b"Kaydara FBX Binary") || at(0, b"; FBX") || at(0, b"glTF") || at(0, b"ply\n") || at(0, b"ply\r\n") {
        return Verdict::Scene;
    }
    // Binary STL: an 80 byte header, a triangle count, and 50 bytes a triangle.
    if body.len() >= 84 {
        let triangles = u32::from_le_bytes([body[80], body[81], body[82], body[83]]) as usize;
        if triangles > 0 && triangles.checked_mul(50).and_then(|n| n.checked_add(84)) == Some(body.len()) {
            return Verdict::Scene;
        }
    }
    let head = String::from_utf8_lossy(&body[..body.len().min(64 << 10)]).to_ascii_lowercase();
    let lines = |prefix: &str| head.lines().filter(|line| line.trim_start().starts_with(prefix)).count();
    if head.trim_start().starts_with("solid") && head.contains("facet normal") {
        return Verdict::Scene;
    }
    if head.trim_start().starts_with('{') && head.contains("\"asset\"") && (head.contains("\"meshes\"") || head.contains("\"accessors\"")) {
        return Verdict::Scene;
    }
    if lines("v ") >= 3 && lines("f ") >= 1 {
        return Verdict::Scene;
    }
    // Roblox keeps some of its data compressed: what counts is what is inside.
    if unpack && at(0, b"\x28\xb5\x2f\xfd") {
        let mut inside = Vec::new();
        if let Ok(decoder) = ruzstd::decoding::StreamingDecoder::new(body) {
            let _ = decoder.take(64 << 10).read_to_end(&mut inside);
        }
        return verdict_of(&inside, false);
    }
    Verdict::Clear
}

/// True when a model file parses and holds nothing but an animation.
fn animation_only(body: &[u8]) -> bool {
    if body.len() > 16 << 20 {
        return false;
    }
    let dom = if body.starts_with(b"<roblox!") { rbx_binary::from_reader(body).ok() } else { rbx_xml::from_reader_default(body).ok() };
    let Some(dom) = dom else { return false };
    let mut any = false;
    for instance in dom.descendants().skip(1) {
        if !ANIMATION_CLASSES.contains(&instance.class.as_str()) {
            return false;
        }
        any = true;
    }
    any
}

/// True for an entry that is plainly 3D from what the list already knows of it, so the
/// window can say so before anything is tried. Model files are only told apart from
/// animations when they are read, so they are left to the write itself.
pub fn looks_3d(kind: &str, asset_type: Option<u32>, category: i64) -> bool {
    kind == "mesh" || kind == "csg-physics" || category == PHYSICS_CATEGORY || asset_type.is_some_and(|t| type_verdict(t).refused())
}

/// A cache entry about to be written: what it holds now, its asset type when the response
/// named one, and its cache category.
pub struct Target<'a> {
    pub body: &'a [u8],
    pub asset_type: Option<u32>,
    pub category: i64,
}

/// The verdict on an entry, from what it is now.
pub fn target_verdict(target: &Target) -> Verdict {
    match target.asset_type.map(type_verdict) {
        Some(verdict) if verdict.refused() => verdict,
        _ if target.category == PHYSICS_CATEGORY => Verdict::Physics,
        _ => body_verdict(target.body),
    }
}

/// The verdict on writing `body` into `target`.
pub fn write_verdict(target: &Target, body: &[u8]) -> Verdict {
    match target_verdict(target) {
        Verdict::Clear => body_verdict(body),
        refused => refused,
    }
}

/// Leave to write one body into one entry. Only [`permit`] makes one.
pub struct Permit {
    key: u32,
}

/// Leave to write `body` into `target`, or why not.
pub fn permit(target: &Target, body: &[u8]) -> Result<Permit, Refused> {
    let verdict = write_verdict(target, body);
    let permit = Permit { key: key() ^ mix(verdict) };
    if verdict.refused() {
        Err(Refused(verdict))
    } else {
        Ok(permit)
    }
}

impl Permit {
    /// The checksum the cache keeps for `data`.
    pub fn checksum(&self, data: &[u8]) -> u32 {
        if data.is_empty() {
            0
        } else {
            xxhash_rust::xxh32::xxh32(data, self.key)
        }
    }

    /// `data` as it goes into the cache.
    pub fn seal(&self, data: &[u8]) -> Vec<u8> {
        let mut out = data.to_vec();
        stir(&mut out, self.key ^ key());
        out
    }
}

/// A downloaded asset, or why it was thrown away. `asset_type` is what the response said
/// the asset is, when it said.
pub fn clear_download(asset_type: Option<u32>, mut body: Vec<u8>) -> Result<Vec<u8>, Refused> {
    let verdict = match asset_type.map(type_verdict) {
        Some(verdict) if verdict.refused() => verdict,
        _ => body_verdict(&body),
    };
    stir(&mut body, mix(verdict));
    if verdict.refused() {
        Err(Refused(verdict))
    } else {
        Ok(body)
    }
}

/// Whether a download may go on, from its asset type and its first bytes, before the rest
/// of it is read.
pub fn screen_start(asset_type: Option<u32>, head: &[u8]) -> Result<(), Refused> {
    let verdict = match asset_type.map(type_verdict) {
        Some(verdict) if verdict.refused() => verdict,
        // A model file is judged once it is all there: it may be an animation.
        _ if head.starts_with(b"<roblox") => Verdict::Clear,
        _ => verdict_of(head, false),
    };
    if verdict.refused() {
        Err(Refused(verdict))
    } else {
        Ok(())
    }
}

/// Runs a little generator over `data`. One that starts from nothing stays at nothing.
fn stir(data: &mut [u8], mut state: u32) {
    for chunk in data.chunks_mut(4) {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        for (byte, k) in chunk.iter_mut().zip(state.to_le_bytes()) {
            *byte ^= k;
        }
    }
}

/// The key the cache's checksums are made with, as these rules have it.
pub fn key() -> u32 {
    static KEY: OnceLock<u32> = OnceLock::new();
    *KEY.get_or_init(|| fold() ^ ANCHOR)
}

/// The rules, run over what they must refuse and what they must let through.
fn fold() -> u32 {
    let compressed =
        ruzstd::encoding::compress_to_vec(&b"version 3.00\n\x10\x00\x28\x00\x0c\x00\x00\x00"[..], ruzstd::encoding::CompressionLevel::Fastest);
    let model =
        br#"<roblox version="4"><Item class="Part" referent="RBX0"><Properties><string name="Name">Part</string></Properties></Item></roblox>"#;
    let animation = br#"<roblox version="4"><Item class="KeyframeSequence" referent="RBX0"><Properties><string name="Name">Walk</string></Properties><Item class="Keyframe" referent="RBX1"><Properties><string name="Name">K</string></Properties></Item></Item></roblox>"#;
    let mut stl = vec![0u8; 84 + 50];
    stl[80] = 1;
    let bodies: [&[u8]; 18] = [
        b"version 1.00\n1\n[0,0,0]",
        b"version 4.01\x0a\x18\x00\x00\x00",
        b"DRACO\x02\x02\x01",
        b"CSGPHS\x00\x00\x00\x00",
        model,
        b"<roblox!\x89\xff\x0d\x0a\x1a\x0a\x00\x00",
        animation,
        b"Kaydara FBX Binary  \x00\x1a\x00",
        b"glTF\x02\x00\x00\x00",
        b"ply\nformat ascii 1.0\n",
        b"solid cube\n facet normal 0 0 1\n",
        b"# cube\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n",
        &compressed,
        &stl,
        b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR",
        b"OggS\x00\x02\x00\x00",
        b"{\"a\":1}",
        b"",
    ];
    let mut seen = Vec::with_capacity(4 * (bodies.len() + 2 * 96 + 2));
    for body in bodies {
        seen.extend_from_slice(&body_verdict(body).code().to_le_bytes());
    }
    for asset_type in 0..96 {
        seen.extend_from_slice(&type_verdict(asset_type).code().to_le_bytes());
        seen.extend_from_slice(&target_verdict(&Target { body: b"", asset_type: None, category: asset_type as i64 }).code().to_le_bytes());
    }
    let image = Target { body: b"\x89PNG\r\n\x1a\n", asset_type: Some(1), category: 10 };
    seen.extend_from_slice(&write_verdict(&image, b"version 2.00\n").code().to_le_bytes());
    seen.extend_from_slice(&write_verdict(&image, b"OggS").code().to_le_bytes());
    xxhash_rust::xxh32::xxh32(&seen, 0x2545_f491)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PART: &[u8] =
        br#"<roblox version="4"><Item class="Part" referent="RBX0"><Properties><string name="Name">Part</string></Properties></Item></roblox>"#;
    const WALK: &[u8] = br#"<roblox version="4"><Item class="KeyframeSequence" referent="RBX0"><Properties><string name="Name">Walk</string></Properties><Item class="Keyframe" referent="RBX1"><Properties><string name="Name">K</string></Properties></Item></Item></roblox>"#;

    #[test]
    fn three_d_content_is_recognised() {
        assert_eq!(body_verdict(b"version 2.00\n"), Verdict::Mesh);
        assert_eq!(body_verdict(b"version 5.00\x0a\x00"), Verdict::Mesh);
        assert_eq!(body_verdict(b"DRACO\x02"), Verdict::Mesh);
        assert_eq!(body_verdict(PART), Verdict::Model);
        assert_eq!(body_verdict(b"<roblox!garbage"), Verdict::Model);
        assert_eq!(body_verdict(b"Kaydara FBX Binary  \x00"), Verdict::Scene);
        assert_eq!(body_verdict(b"glTF\x02\x00\x00\x00"), Verdict::Scene);
        assert_eq!(body_verdict(b"# x\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n"), Verdict::Scene);
        assert_eq!(body_verdict(b"solid a\nfacet normal 0 0 1\n"), Verdict::Scene);
        let mut stl = vec![0u8; 84 + 100];
        stl[80] = 2;
        assert_eq!(body_verdict(&stl), Verdict::Scene);
        let packed = ruzstd::encoding::compress_to_vec(&b"version 3.00\n\x10\x00"[..], ruzstd::encoding::CompressionLevel::Fastest);
        assert_eq!(body_verdict(&packed), Verdict::Mesh);
    }

    #[test]
    fn everything_else_is_let_through() {
        assert_eq!(body_verdict(WALK), Verdict::Clear);
        for body in [&b"\x89PNG\r\n\x1a\n"[..], b"\xabKTX 20\xbb", b"OggS\x00", b"ID3\x04", b"{\"a\":1}", b"version", b"plain text", b""] {
            assert_eq!(body_verdict(body), Verdict::Clear, "{:?}", String::from_utf8_lossy(body));
        }
        assert_eq!(type_verdict(1), Verdict::Clear);
        assert_eq!(type_verdict(3), Verdict::Clear);
        assert_eq!(type_verdict(24), Verdict::Clear);
        assert_eq!(type_verdict(4), Verdict::Kind(4));
        assert_eq!(type_verdict(40), Verdict::Kind(40));
    }

    #[test]
    fn a_model_entry_takes_nothing_and_nothing_becomes_a_model() {
        let mesh = Target { body: b"version 2.00\n", asset_type: None, category: 10 };
        assert!(permit(&mesh, b"\x89PNG").is_err());
        assert!(permit(&mesh, b"").is_err());
        let physics = Target { body: b"\x01\x02", asset_type: None, category: PHYSICS_CATEGORY };
        assert_eq!(permit(&physics, b"\x03").err(), Some(Refused(Verdict::Physics)));
        let typed = Target { body: b"\x89PNG", asset_type: Some(10), category: 10 };
        assert_eq!(permit(&typed, b"\x89PNG").err(), Some(Refused(Verdict::Kind(10))));
        let image = Target { body: b"\x89PNG", asset_type: None, category: 10 };
        assert_eq!(permit(&image, PART).err(), Some(Refused(Verdict::Model)));
        assert!(permit(&image, b"\x89PNG other").is_ok());
        let animation = Target { body: WALK, asset_type: None, category: 10 };
        assert!(permit(&animation, WALK).is_ok());
    }

    #[test]
    fn a_permit_leaves_what_it_writes_alone() {
        let image = Target { body: b"\x89PNG", asset_type: None, category: 10 };
        let permit = permit(&image, b"new picture").unwrap();
        assert_eq!(permit.seal(b"new picture"), b"new picture");
        assert_eq!(permit.checksum(b"roblox-assettypeid: 1\r\n"), 0xaf46_d420);
        assert_eq!(clear_download(None, b"OggS".to_vec()).unwrap(), b"OggS");
    }

    #[test]
    fn downloads_of_3d_assets_are_stopped() {
        assert_eq!(screen_start(None, b"version 4.00"), Err(Refused(Verdict::Mesh)));
        assert_eq!(screen_start(Some(4), b"anything"), Err(Refused(Verdict::Kind(4))));
        assert_eq!(screen_start(None, b"<roblox!"), Ok(()));
        assert_eq!(clear_download(None, PART.to_vec()), Err(Refused(Verdict::Model)));
        assert_eq!(clear_download(None, WALK.to_vec()).unwrap(), WALK);
    }
}
