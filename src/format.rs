//! The `RBXH` wrapper Roblox puts around cached HTTP responses, plus content sniffing.
//!
//! Layout (little endian):
//! ```text
//! "RBXH" | u32 version | u32 urlLen | url | u8 flag | u32 httpStatus
//! | u32 headerLen | u32 headerSum | u32 bodyLen | u32 bodySum | u32 reserved
//! | response headers | body
//! ```
//! Both sums are [`checksum`] of the bytes they cover.

use crate::integrity::Permit;

pub const MAGIC: &[u8; 4] = b"RBXH";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Header {
    pub url: String,
    pub url_len: usize,
    pub status: u32,
    pub headers: String,
    pub header_sum: u32,
    pub body_len: u32,
    pub body_sum: u32,
    /// Offset of the first body byte inside the blob.
    pub body_offset: usize,
}

fn u32le(buf: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([buf[at], buf[at + 1], buf[at + 2], buf[at + 3]])
}

/// Parses the wrapper. `buf` may be just the start of a blob as long as it
/// covers the URL and the response headers. Returns `None` for blobs that are
/// not wrapped (or whose header does not fit in `buf`).
pub fn parse_header(buf: &[u8]) -> Option<Header> {
    if buf.len() < 37 || &buf[..4] != MAGIC {
        return None;
    }
    let url_len = u32le(buf, 8) as usize;
    let o = 12usize.checked_add(url_len)?;
    if buf.len() < o.checked_add(25)? {
        return None;
    }
    let header_len = u32le(buf, o + 5) as usize;
    let headers_at = o + 25;
    let body_offset = headers_at.checked_add(header_len)?;
    if buf.len() < body_offset {
        return None;
    }
    Some(Header {
        url: String::from_utf8_lossy(&buf[12..o]).into_owned(),
        url_len,
        status: u32le(buf, o + 1),
        headers: String::from_utf8_lossy(&buf[headers_at..body_offset]).into_owned(),
        header_sum: u32le(buf, o + 9),
        body_len: u32le(buf, o + 13),
        body_sum: u32le(buf, o + 17),
        body_offset,
    })
}

/// Total length of the wrapper (everything before the body) as far as `buf`
/// reveals it, or `None` if `buf` is not wrapped. When `buf` is too short to
/// hold the response-header length, the result only covers up to that field.
pub fn wrapper_len(buf: &[u8]) -> Option<usize> {
    if buf.len() < 12 || &buf[..4] != MAGIC {
        return None;
    }
    let fixed = 12usize.checked_add(u32le(buf, 8) as usize)?.checked_add(25)?;
    if buf.len() < fixed {
        return Some(fixed);
    }
    fixed.checked_add(u32le(buf, fixed - 20) as usize)
}

/// The checksum Roblox stores for the response headers and for the body:
/// XXH32 with a fixed key (see [`crate::integrity::key`]), and zero for empty data.
pub fn checksum(data: &[u8]) -> u32 {
    if data.is_empty() {
        0
    } else {
        xxhash_rust::xxh32::xxh32(data, crate::integrity::key())
    }
}

/// Rebuilds a wrapped blob around a new body, keeping URL, status and headers
/// and updating the body's length and checksum. `permit` is the leave to write
/// this body into this entry.
pub fn with_new_body(old: &[u8], header: &Header, body: &[u8], permit: &Permit) -> Vec<u8> {
    let mut out = Vec::with_capacity(header.body_offset + body.len());
    out.extend_from_slice(&old[..header.body_offset]);
    let len_at = 12 + header.url_len + 13;
    out[len_at..len_at + 4].copy_from_slice(&(body.len() as u32).to_le_bytes());
    out[len_at + 4..len_at + 8].copy_from_slice(&permit.checksum(body).to_le_bytes());
    out.extend_from_slice(&permit.seal(body));
    out
}

/// Value of the `roblox-assettypeid` response header, if present.
pub fn asset_type_id(headers: &str) -> Option<u32> {
    headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.trim().eq_ignore_ascii_case("roblox-assettypeid") {
            value.trim().parse().ok()
        } else {
            None
        }
    })
}

pub fn asset_type_name(id: u32) -> &'static str {
    match id {
        1 => "Image",
        2 => "TShirt",
        3 => "Audio",
        4 => "Mesh",
        5 => "Lua",
        8 => "Hat",
        9 => "Place",
        10 => "Model",
        11 => "Shirt",
        12 => "Pants",
        13 => "Decal",
        17 => "Head",
        18 => "Face",
        19 => "Gear",
        21 => "Badge",
        24 => "Animation",
        27 => "Torso",
        28 => "RightArm",
        29 => "LeftArm",
        30 => "LeftLeg",
        31 => "RightLeg",
        32 => "Package",
        34 => "GamePass",
        38 => "Plugin",
        40 => "MeshPart",
        41 => "HairAccessory",
        42 => "FaceAccessory",
        43 => "NeckAccessory",
        44 => "ShoulderAccessory",
        45 => "FrontAccessory",
        46 => "BackAccessory",
        47 => "WaistAccessory",
        48 => "ClimbAnimation",
        49 => "DeathAnimation",
        50 => "FallAnimation",
        51 => "IdleAnimation",
        52 => "JumpAnimation",
        53 => "RunAnimation",
        54 => "SwimAnimation",
        55 => "WalkAnimation",
        56 => "PoseAnimation",
        61 => "EmoteAnimation",
        62 => "Video",
        63 => "TexturePack",
        64 => "TShirtAccessory",
        65 => "ShirtAccessory",
        66 => "PantsAccessory",
        67 => "JacketAccessory",
        68 => "SweaterAccessory",
        69 => "ShortsAccessory",
        70 => "LeftShoeAccessory",
        71 => "RightShoeAccessory",
        72 => "DressSkirtAccessory",
        73 => "FontFamily",
        74 => "FontFace",
        75 => "MeshHiddenSurfaceRemoval",
        76 => "EyebrowAccessory",
        77 => "EyelashAccessory",
        78 => "MoodAnimation",
        79 => "DynamicHead",
        _ => "unknown",
    }
}

/// Guesses the content type from the first bytes of a body.
pub fn sniff(b: &[u8]) -> &'static str {
    let at = |o: usize, pat: &[u8]| b.len() >= o + pat.len() && &b[o..o + pat.len()] == pat;
    if b.is_empty() {
        "empty"
    } else if at(0, b"<roblox!") {
        "rbxm"
    } else if at(0, b"<roblox") {
        "rbxmx"
    } else if at(0, b"\x89PNG") {
        "png"
    } else if at(0, b"\xff\xd8\xff") {
        "jpeg"
    } else if at(0, b"GIF8") {
        "gif"
    } else if at(0, b"RIFF") && at(8, b"WEBP") {
        "webp"
    } else if at(0, b"RIFF") && at(8, b"WAVE") {
        "wav"
    } else if at(0, b"\xabKTX 20") {
        "ktx2"
    } else if at(0, b"\xabKTX") {
        "ktx"
    } else if at(0, b"OggS") {
        "ogg"
    } else if at(0, b"ID3") || (b.len() > 1 && b[0] == 0xff && b[1] & 0xe0 == 0xe0) {
        "mp3"
    } else if at(0, b"version ") {
        "mesh"
    } else if at(0, b"\x1a\x45\xdf\xa3") {
        "webm"
    } else if at(4, b"ftyp") {
        "mp4"
    } else if at(0, b"\x00\x01\x00\x00\x00") {
        "ttf"
    } else if at(0, b"OTTO") {
        "otf"
    } else if at(0, b"\x28\xb5\x2f\xfd") {
        "zstd"
    } else if at(0, b"{") || at(0, b"[") {
        "json"
    } else if b.iter().take(64).all(|c| matches!(c, 9 | 10 | 13 | 32..=126)) {
        "text"
    } else {
        "binary"
    }
}

/// File extension used when exporting a body of the given kind.
pub fn extension(kind: &str) -> &'static str {
    match kind {
        "png" => "png",
        "jpeg" => "jpg",
        "gif" => "gif",
        "webp" => "webp",
        "wav" => "wav",
        "ktx2" => "ktx2",
        "ktx" => "ktx",
        "ogg" => "ogg",
        "mp3" => "mp3",
        "mesh" => "mesh",
        "rbxm" => "rbxm",
        "rbxmx" => "rbxmx",
        "webm" => "webm",
        "mp4" => "mp4",
        "ttf" => "ttf",
        "otf" => "otf",
        "zstd" | "csg-physics" | "zstd-blob" => "zst",
        "json" | "asset-map" => "json",
        "text" => "txt",
        _ => "bin",
    }
}

pub fn is_image(kind: &str) -> bool {
    matches!(kind, "png" | "jpeg" | "gif" | "webp" | "ktx2" | "ktx")
}

pub fn is_audio(kind: &str) -> bool {
    matches!(kind, "ogg" | "mp3" | "wav")
}

/// `n` followed by the right form of a noun, e.g. `count(1, "entry", "entries")`.
pub fn count(n: usize, singular: &str, plural: &str) -> String {
    format!("{n} {}", if n == 1 { singular } else { plural })
}

pub fn is_text(kind: &str) -> bool {
    matches!(kind, "json" | "asset-map" | "text" | "rbxmx")
}

pub fn human_size(n: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

/// Classic 16-bytes-per-line hex dump of the first `max` bytes.
pub fn hex_dump(b: &[u8], max: usize) -> String {
    let mut out = String::new();
    for (i, chunk) in b[..b.len().min(max)].chunks(16).enumerate() {
        out.push_str(&format!("{:06x}  ", i * 16));
        for j in 0..16 {
            match chunk.get(j) {
                Some(c) => out.push_str(&format!("{c:02x} ")),
                None => out.push_str("   "),
            }
        }
        out.push(' ');
        out.extend(chunk.iter().map(|&c| if (32..127).contains(&c) { c as char } else { '.' }));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blob(url: &str, status: u32, headers: &str, body: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(MAGIC);
        v.extend_from_slice(&2u32.to_le_bytes());
        v.extend_from_slice(&(url.len() as u32).to_le_bytes());
        v.extend_from_slice(url.as_bytes());
        v.push(0);
        v.extend_from_slice(&status.to_le_bytes());
        v.extend_from_slice(&(headers.len() as u32).to_le_bytes());
        v.extend_from_slice(&0x1111_1111u32.to_le_bytes());
        v.extend_from_slice(&(body.len() as u32).to_le_bytes());
        v.extend_from_slice(&0x2222_2222u32.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(headers.as_bytes());
        v.extend_from_slice(body);
        v
    }

    #[test]
    fn parses_plain_asset() {
        let b = blob("", 200, "", b"\x89PNG....");
        let h = parse_header(&b).unwrap();
        assert_eq!((h.status, h.body_offset, h.body_len), (200, 37, 8));
        assert_eq!(sniff(&b[h.body_offset..]), "png");
    }

    #[test]
    fn parses_redirect() {
        let b = blob("https://example.com/x", 302, "roblox-assettypeid: 4\r\n", b"");
        let h = parse_header(&b).unwrap();
        assert_eq!(h.url, "https://example.com/x");
        assert_eq!(h.status, 302);
        assert_eq!(asset_type_id(&h.headers), Some(4));
        assert_eq!(h.body_offset, b.len());
    }

    #[test]
    fn rejects_unwrapped_and_truncated() {
        assert!(parse_header(b"{\"a\":1}").is_none());
        let b = blob("https://example.com/x", 200, "a: b\r\n", b"body");
        assert!(parse_header(&b[..40]).is_none());
    }

    #[test]
    fn replaces_body() {
        let old = blob("https://example.com/x", 200, "a: b\r\n", b"old body");
        let h = parse_header(&old).unwrap();
        let target = crate::integrity::Target { body: b"old body", asset_type: None, category: 10 };
        let permit = crate::integrity::permit(&target, b"a much longer new body").unwrap();
        let new = with_new_body(&old, &h, b"a much longer new body", &permit);
        let h2 = parse_header(&new).unwrap();
        assert_eq!(&new[h2.body_offset..], b"a much longer new body");
        assert_eq!(h2.body_len, 22);
        assert_eq!((h2.url.as_str(), h2.status, h2.headers.as_str()), ("https://example.com/x", 200, "a: b\r\n"));
        assert_eq!((h2.header_sum, h2.body_sum), (h.header_sum, checksum(b"a much longer new body")));
        let permit = crate::integrity::permit(&target, b"").unwrap();
        assert_eq!(parse_header(&with_new_body(&old, &h, b"", &permit)).unwrap().body_sum, 0);
    }

    #[test]
    fn checksum_matches_values_roblox_stored() {
        assert_eq!(checksum(b"roblox-assettypeid: 1\r\n"), 0xaf46_d420);
        assert_eq!(checksum(b"roblox-assettypeid: 4\r\n"), 0xe472_056a);
        assert_eq!(checksum(b"roblox-assettypeid: 84\r\n"), 0x5992_45d5);
        assert_eq!(checksum(b""), 0);
    }
}
