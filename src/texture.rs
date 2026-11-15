//! Roblox's streamed textures, and replacements written the way it stores them.
//!
//! The client does not cache an image as one file. It asks for KTX2 "packs":
//! the texture cut into groups of mip levels, the smallest first, each cached
//! as its own entry and fetched as the texture gets closer. Every pack carries
//! the same `contentHash` in its metadata, which ties the entries of one
//! texture together.
//!
//! So a replacement has to go into every pack, at that pack's size, block
//! compressed and Zstandard-wrapped like the original, with the original's
//! metadata kept. Swapping only the first pack changes the blurry far-away
//! version and leaves the texture looking as it did up close.

use crate::store::{Entry, Id, Store};
use image::{imageops, RgbaImage};
use md5::{Digest, Md5};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

const MAGIC: &[u8; 12] = b"\xabKTX 20\xbb\r\n\x1a\n";
/// Vulkan format numbers. Each has an sRGB variant one above it.
const BC1_RGB: u32 = 131;
const BC1_RGBA: u32 = 133;
const BC3: u32 = 137;
/// Most texels a pack may claim (8192 x 8192); a header asking for more is not believed.
const MAX_TEXELS: u64 = 1 << 26;
/// Supercompression scheme 2: each level is a Zstandard frame.
const ZSTD: u32 = 2;

/// The data format descriptor Roblox writes for BC3, used when a pack has to
/// change format to carry transparency. Byte 14 is the transfer function.
const BC3_DFD: [u8; 60] = [
    0x3c, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0x38, 0, 0x82, 1, 1, 1, 3, 3, 0, 0, 0x10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x3f, 0x0f, 0, 0, 0, 0, 0, 0, 0, 0, 0xff,
    0xff, 0xff, 0xff, 0x40, 0, 0x3f, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff,
];

/// Metadata the replacement recomputes; everything else is copied.
const RECOMPUTED: [&str; 8] = ["avgRed", "avgGreen", "avgBlue", "avgAlpha", "weightedAvgRed", "weightedAvgGreen", "weightedAvgBlue", "constantColor"];

/// One cached pack of a streamed texture, as far as its header describes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Pack {
    /// Shared by every pack of the same texture.
    pub content_hash: String,
    /// The Vulkan format number of the pixel data.
    pub format: u32,
    /// Size of the largest mip level in this pack.
    pub width: u32,
    pub height: u32,
    pub levels: u32,
    dfd: Vec<u8>,
    /// Key and raw value of each metadata pair, in file order.
    pairs: Vec<(String, Vec<u8>)>,
}

fn u32at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// The value of a metadata pair as text, without its terminator.
fn text(pairs: &[(String, Vec<u8>)], key: &str) -> Option<String> {
    let (_, value) = pairs.iter().find(|(k, _)| k == key)?;
    Some(String::from_utf8_lossy(value).trim_end_matches('\0').to_string())
}

impl Pack {
    /// Reads a pack from the start of a KTX2 file. `None` unless it is one of
    /// Roblox's: block-compressed levels and a `contentHash`.
    pub fn parse(head: &[u8]) -> Option<Self> {
        if !head.starts_with(MAGIC) {
            return None;
        }
        let (format, width, height, levels) = (u32at(head, 12)?, u32at(head, 20)?, u32at(head, 24)?.max(1), u32at(head, 40)?.max(1));
        let (dfd_at, dfd_len, kvd_at, kvd_len) =
            (u32at(head, 48)? as usize, u32at(head, 52)? as usize, u32at(head, 56)? as usize, u32at(head, 60)? as usize);
        if width == 0 || width as u64 * height as u64 > MAX_TEXELS || levels > 16 {
            return None;
        }
        let dfd = head.get(dfd_at..dfd_at.checked_add(dfd_len)?)?.to_vec();
        let kvd = head.get(kvd_at..kvd_at.checked_add(kvd_len)?)?;
        let mut pairs = Vec::new();
        let mut at = 0;
        while at + 4 <= kvd.len() {
            let len = u32at(kvd, at)? as usize;
            let pair = kvd.get(at + 4..(at + 4).checked_add(len)?)?;
            let split = pair.iter().position(|&c| c == 0)?;
            pairs.push((String::from_utf8_lossy(&pair[..split]).into_owned(), pair[split + 1..].to_vec()));
            at += 4 + len.next_multiple_of(4);
        }
        let content_hash = text(&pairs, "contentHash").filter(|hash| !hash.is_empty())?;
        Some(Self { content_hash, format, width, height, levels, dfd, pairs })
    }

    fn text(&self, key: &str) -> Option<String> {
        text(&self.pairs, key)
    }

    /// The size the whole texture has at full detail, when the pack records it.
    pub fn full_size(&self) -> Option<(u32, u32)> {
        Some((self.text("transcodedWidth")?.parse().ok()?, self.text("transcodedHeight")?.parse().ok()?))
    }

    /// What decides the result of [`Pack::refit`] besides the image: everything
    /// in the pack that a replacement keeps.
    fn identity(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for v in [self.format, self.width, self.height, self.levels] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&self.dfd);
        for (key, value) in self.pairs.iter().filter(|(key, _)| !RECOMPUTED.contains(&key.as_str())) {
            out.extend_from_slice(key.as_bytes());
            out.push(0);
            out.extend_from_slice(value);
        }
        out
    }

    /// Writes `image` as this pack: the same size, level count and metadata,
    /// with the average colours recomputed. The format is kept, except that a
    /// pack with no alpha channel becomes BC3 when the image is transparent.
    pub fn refit(&self, image: &RgbaImage) -> Vec<u8> {
        let transparent = image.pixels().any(|p| p[3] < 255);
        let opaque_bc1 = BC1_RGB..BC1_RGBA;
        let (format, bc, dfd) = match self.format {
            // No room for alpha: the BC3 format of the same colour space takes over.
            f if opaque_bc1.contains(&f) && transparent => {
                let mut dfd = BC3_DFD.to_vec();
                dfd[14] = self.dfd.get(14).copied().unwrap_or(1);
                (f + (BC3 - BC1_RGB), texpresso::Format::Bc3, dfd)
            }
            f if (BC1_RGB..BC1_RGBA + 2).contains(&f) => (f, texpresso::Format::Bc1, self.dfd.clone()),
            f if f == BC3 || f == BC3 + 1 => (f, texpresso::Format::Bc3, self.dfd.clone()),
            _ => (BC3, texpresso::Format::Bc3, BC3_DFD.to_vec()),
        };

        // Each level is the one before it at half the size.
        let mut levels: Vec<Vec<u8>> = Vec::new();
        let mut level = imageops::resize(image, self.width, self.height, imageops::FilterType::Triangle);
        for i in 0..self.levels {
            if i > 0 {
                level = imageops::resize(&level, (self.width >> i).max(1), (self.height >> i).max(1), imageops::FilterType::Triangle);
            }
            let (w, h) = (level.width() as usize, level.height() as usize);
            let mut blocks = vec![0u8; bc.compressed_size(w, h)];
            let params = texpresso::Params { algorithm: texpresso::Algorithm::RangeFit, ..Default::default() };
            bc.compress(level.as_raw(), w, h, params, &mut blocks);
            levels.push(blocks);
        }

        let mut kvd = Vec::new();
        let stats = Stats::of(image);
        for (key, value) in &self.pairs {
            let value = match stats.value(key) {
                Some(number) => {
                    // Roblox ends some values with two NULs; the count is kept as it was.
                    let nuls = value.iter().rev().take_while(|&&c| c == 0).count().max(1);
                    let mut text = number.to_string().into_bytes();
                    text.resize(text.len() + nuls, 0);
                    text
                }
                None => value.clone(),
            };
            kvd.extend_from_slice(&((key.len() + 1 + value.len()) as u32).to_le_bytes());
            kvd.extend_from_slice(key.as_bytes());
            kvd.push(0);
            kvd.extend_from_slice(&value);
            kvd.resize(kvd.len().next_multiple_of(4), 0);
        }

        let dfd_at = 80 + 24 * levels.len();
        let kvd_at = dfd_at + dfd.len();
        let mut out = MAGIC.to_vec();
        for v in [format, 1, self.width, self.height, 0, 0, 1, self.levels, ZSTD, dfd_at as u32, dfd.len() as u32, kvd_at as u32, kvd.len() as u32] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        // No supercompression global data.
        out.extend_from_slice(&[0; 16]);

        // The file holds the smallest level first; the index lists the largest first.
        let packed: Vec<Vec<u8>> =
            levels.iter().map(|l| ruzstd::encoding::compress_to_vec(&l[..], ruzstd::encoding::CompressionLevel::Fastest)).collect();
        let mut offset = kvd_at + kvd.len();
        let mut index = vec![(0usize, 0usize, 0usize); levels.len()];
        for i in (0..levels.len()).rev() {
            index[i] = (offset, packed[i].len(), levels[i].len());
            offset += packed[i].len();
        }
        for (at, stored, raw) in index {
            for v in [at, stored, raw] {
                out.extend_from_slice(&(v as u64).to_le_bytes());
            }
        }
        out.extend_from_slice(&dfd);
        out.extend_from_slice(&kvd);
        for level in packed.iter().rev() {
            out.extend_from_slice(level);
        }
        out
    }
}

/// The colour figures Roblox keeps beside a texture.
struct Stats {
    average: [u64; 4],
    weighted: [u64; 3],
    constant: bool,
}

impl Stats {
    fn of(image: &RgbaImage) -> Self {
        let count = (image.width() as u64 * image.height() as u64).max(1);
        let (mut sum, mut by_alpha) = ([0u64; 4], [0u64; 3]);
        for p in image.pixels() {
            for c in 0..4 {
                sum[c] += p[c] as u64;
            }
            for c in 0..3 {
                by_alpha[c] += p[c] as u64 * p[3] as u64;
            }
        }
        let first = image.pixels().next().copied();
        Self {
            average: sum.map(|s| s / count),
            // Weighted by how opaque each pixel is, so a transparent background does not darken it.
            weighted: by_alpha.map(|s| s.checked_div(sum[3]).unwrap_or(0)),
            constant: image.pixels().all(|p| Some(*p) == first),
        }
    }

    fn value(&self, key: &str) -> Option<u64> {
        Some(match key {
            "avgRed" => self.average[0],
            "avgGreen" => self.average[1],
            "avgBlue" => self.average[2],
            "avgAlpha" => self.average[3],
            "weightedAvgRed" => self.weighted[0],
            "weightedAvgGreen" => self.weighted[1],
            "weightedAvgBlue" => self.weighted[2],
            "constantColor" => self.constant as u64,
            _ => return None,
        })
    }
}

/// The `contentHash` of a KTX2 file, if it is one of Roblox's packs.
pub fn content_hash(head: &[u8]) -> Option<String> {
    Pack::parse(head).map(|pack| pack.content_hash)
}

/// The content hash of a cached entry. Reading it means opening the entry, so
/// the answer is remembered for as long as the entry keeps its size. An entry
/// that could not be read is asked again next time.
pub fn hash_of(store: &Store, entry: &Entry) -> Option<String> {
    /// Cache folder and entry -> the entry's stored size when read, and the hash found.
    type Known = HashMap<(PathBuf, Id), (u64, Option<String>)>;
    static KNOWN: Mutex<Option<Known>> = Mutex::new(None);
    if !entry.wrapped || entry.kind != "ktx2" {
        return None;
    }
    let key = (store.dir.clone(), entry.id);
    if let Some((size, hash)) = KNOWN.lock().ok()?.get_or_insert_with(HashMap::new).get(&key) {
        if *size == entry.size {
            return hash.clone();
        }
    }
    let hash = content_hash(&store.read_body_head(&entry.id).ok()?);
    KNOWN.lock().ok()?.get_or_insert_with(HashMap::new).insert(key, (entry.size, hash.clone()));
    hash
}

/// A fully transparent image, which is what removing a texture leaves.
pub fn blank() -> RgbaImage {
    RgbaImage::new(4, 4)
}

/// What [`Pack::refit`] gives for an image with this digest, without doing the
/// work twice: the MD5 of the result is remembered per pack and image.
pub fn refit_digest(pack: &Pack, image_digest: &[u8; 16], image: &RgbaImage) -> [u8; 16] {
    static DONE: Mutex<Option<HashMap<[u8; 16], [u8; 16]>>> = Mutex::new(None);
    let mut key = Md5::new();
    key.update(image_digest);
    key.update(pack.identity());
    let key: [u8; 16] = key.finalize().into();
    if let Some(digest) = DONE.lock().ok().and_then(|done| done.as_ref()?.get(&key).copied()) {
        return digest;
    }
    let digest: [u8; 16] = Md5::digest(pack.refit(image)).into();
    if let Ok(mut done) = DONE.lock() {
        done.get_or_insert_with(HashMap::new).insert(key, digest);
    }
    digest
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::viewers::decode_ktx2;

    /// A pack laid out like Roblox's, with stand-in pixel data: `levels` mips starting at `width`.
    pub(crate) fn sample_pack(format: u32, width: u32, levels: u32, hash: &str) -> Vec<u8> {
        let source = Pack {
            content_hash: hash.to_string(),
            format,
            width,
            height: width,
            levels,
            dfd: BC3_DFD.to_vec(),
            pairs: [
                ("RobloxOriginalWidth", "400"),
                ("avgAlpha", "9"),
                ("avgRed", "9"),
                ("constantColor", "1"),
                ("contentHash", hash),
                ("packIndex", "0"),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), format!("{v}\0").into_bytes()))
            .collect(),
        };
        source.refit(&RgbaImage::from_pixel(8, 8, image::Rgba([10, 20, 30, 255])))
    }

    #[test]
    fn a_replacement_keeps_the_pack_shape_and_metadata() {
        let original = sample_pack(137, 48, 6, "907c9ff26aefa3e509ded92674486d7b");
        let pack = Pack::parse(&original).unwrap();
        assert_eq!((pack.format, pack.width, pack.height, pack.levels), (137, 48, 48, 6));
        assert_eq!(pack.content_hash, "907c9ff26aefa3e509ded92674486d7b");

        let red = RgbaImage::from_pixel(100, 60, image::Rgba([200, 0, 0, 255]));
        let written = pack.refit(&red);
        let back = Pack::parse(&written).unwrap();
        assert_eq!((back.format, back.width, back.height, back.levels), (137, 48, 48, 6));
        assert_eq!(back.text("contentHash").as_deref(), Some("907c9ff26aefa3e509ded92674486d7b"));
        assert_eq!(back.text("RobloxOriginalWidth").as_deref(), Some("400"));
        assert_eq!((back.text("avgRed").as_deref(), back.text("avgAlpha").as_deref()), (Some("200"), Some("255")));
        // Writing the same image into the result changes nothing, so a second run leaves the entry alone.
        assert_eq!(back.refit(&red), written);
        assert_eq!(refit_digest(&pack, &[1; 16], &red), <[u8; 16]>::from(Md5::digest(&written)));

        // Every level decodes to the colour that was written.
        for max_side in [48, 12, 1] {
            let decoded = decode_ktx2(&written, max_side).unwrap();
            assert_eq!(decoded.width as u32, max_side);
            let p = &decoded.pixels[..4];
            assert!(p[0] > 190 && p[1] < 12 && p[2] < 12 && p[3] == 255, "level of side {max_side} decoded to {p:?}");
        }
    }

    #[test]
    fn transparency_moves_an_opaque_format_to_bc3() {
        let pack = Pack::parse(&sample_pack(131, 16, 3, "aa")).unwrap();
        assert_eq!(Pack::parse(&pack.refit(&RgbaImage::from_pixel(4, 4, image::Rgba([1, 2, 3, 255])))).unwrap().format, 131);
        let cleared = pack.refit(&blank());
        let back = Pack::parse(&cleared).unwrap();
        assert_eq!((back.format, back.text("avgAlpha").as_deref(), back.text("constantColor").as_deref()), (137, Some("0"), Some("1")));
        assert!(decode_ktx2(&cleared, 16).unwrap().pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 0));
    }

    #[test]
    fn other_ktx2_files_are_not_packs() {
        assert_eq!(content_hash(&crate::viewers::encode_ktx2(2, 2, &[0; 16]).unwrap()), None);
        assert_eq!(content_hash(b"not a texture"), None);
    }
}
