//! Decoders that turn cached payloads into something viewable: KTX2 textures,
//! Roblox meshes (with a small software renderer) and Roblox model trees.

use anyhow::{anyhow, bail, ensure, Context, Result};
use std::io::Read;

fn u16at(b: &[u8], o: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(b.get(o..o + 2).context("unexpected end of data")?.try_into()?))
}

fn u32at(b: &[u8], o: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(b.get(o..o + 4).context("unexpected end of data")?.try_into()?))
}

fn u64at(b: &[u8], o: usize) -> Result<u64> {
    Ok(u64::from_le_bytes(b.get(o..o + 8).context("unexpected end of data")?.try_into()?))
}

fn f32at(b: &[u8], o: usize) -> Result<f32> {
    Ok(f32::from_bits(u32at(b, o)?))
}

// ---------------------------------------------------------------- textures

pub struct Rgba {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

/// Largest texture, in pixels, that is decoded (8192 x 8192).
const MAX_TEXELS: usize = 1 << 26;

/// Decodes a KTX2 texture to RGBA. Roblox ships block-compressed (BC) data with
/// Zstandard on top; the largest mip level no bigger than `max_side` is used.
pub fn decode_ktx2(b: &[u8], max_side: u32) -> Result<Rgba> {
    ensure!(b.len() >= 104 && b.starts_with(b"\xabKTX 20\xbb\r\n\x1a\n"), "not a KTX2 file");
    let format = u32at(b, 12)?;
    let (width, height) = (u32at(b, 20)?, u32at(b, 24)?.max(1));
    let levels = u32at(b, 40)?.max(1);
    let supercompression = u32at(b, 44)?;
    ensure!(width > 0 && width <= 16384 && height <= 16384, "unreasonable texture size");

    // Level 0 is the full image; each later level halves it.
    let mut level = 0;
    while level + 1 < levels && (width >> level).max(height >> level) > max_side {
        level += 1;
    }
    let (w, h) = ((width >> level).max(1) as usize, (height >> level).max(1) as usize);
    ensure!(w * h <= MAX_TEXELS, "the texture is too large to decode ({w} x {h})");
    let decode = match format {
        131..=134 => texture2ddecoder::decode_bc1,
        137 | 138 => texture2ddecoder::decode_bc3,
        139 | 140 => texture2ddecoder::decode_bc4,
        141 | 142 => texture2ddecoder::decode_bc5,
        145 | 146 => texture2ddecoder::decode_bc7,
        other => bail!("unsupported texture format (vkFormat {other})"),
    };
    let block_bytes = if matches!(format, 131..=134 | 139 | 140) { 8 } else { 16 };
    let expected = w.div_ceil(4) * h.div_ceil(4) * block_bytes;

    let entry = 80 + 24 * level as usize;
    let (offset, length) = (u64at(b, entry)? as usize, u64at(b, entry + 8)? as usize);
    let stored = b.get(offset..offset.checked_add(length).context("bad level size")?).context("level data lies outside the file")?;
    let data = match supercompression {
        0 => stored.to_vec(),
        2 => {
            // Never inflate more than the level can hold, whatever the stream claims.
            let mut out = Vec::new();
            let decoder = ruzstd::decoding::StreamingDecoder::new(stored).map_err(|e| anyhow!("zstd: {e}"))?;
            decoder.take(expected as u64).read_to_end(&mut out)?;
            out
        }
        other => bail!("unsupported KTX2 supercompression scheme {other}"),
    };
    ensure!(data.len() >= expected, "texture data is truncated");
    let mut texels = vec![0u32; w * h];
    decode(&data, w, h, &mut texels).map_err(|e| anyhow!("texture decode failed: {e}"))?;
    // The decoder packs texels as 0xAARRGGBB.
    let mut pixels = Vec::with_capacity(w * h * 4);
    for t in texels {
        pixels.extend_from_slice(&[(t >> 16) as u8, (t >> 8) as u8, t as u8, (t >> 24) as u8]);
    }
    Ok(Rgba { width: w, height: h, pixels })
}

// ------------------------------------------------------------------ meshes

/// Colour meshes are drawn in when they carry no colours of their own.
pub const MESH_TINT: [f32; 3] = [0.74, 0.79, 0.98];

/// Yaw and pitch a mesh is first shown from.
pub const MESH_ANGLES: (f32, f32) = (0.6, 0.35);

/// Yaw and pitch that show a character from the front; Roblox characters face -Z.
pub const FIGURE_ANGLES: (f32, f32) = (std::f32::consts::PI - 0.5, 0.2);

/// Vertex positions and the triangles that index them.
type Geometry = (Vec<[f32; 3]>, Vec<[u32; 3]>);

/// Where a mesh is seen from: the sphere to frame, the camera angles and a zoom factor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub centre: [f32; 3],
    pub radius: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
}

impl View {
    /// A view of `bounds` (as from [`Mesh::bounds`]) at the given yaw and pitch, unzoomed.
    pub fn new((centre, radius): ([f32; 3], f32), (yaw, pitch): (f32, f32)) -> Self {
        Self { centre, radius, yaw, pitch, zoom: 1.0 }
    }
}

pub struct Mesh {
    pub version: String,
    pub positions: Vec<[f32; 3]>,
    pub faces: Vec<[u32; 3]>,
    /// One colour per face; empty means the whole mesh takes the render tint.
    pub tints: Vec<[f32; 3]>,
}

/// Parses any Roblox mesh version (1.x text, 2-5 binary, 6-7 chunked with Draco).
pub fn parse_mesh(b: &[u8]) -> Result<Mesh> {
    ensure!(b.starts_with(b"version "), "not a Roblox mesh");
    let newline = b.iter().position(|&c| c == b'\n').context("missing version line")?;
    let version = String::from_utf8_lossy(&b[8..newline]).trim().to_string();
    let rest = &b[newline + 1..];
    let (positions, faces) = match version.as_str() {
        v if v.starts_with("1.") => mesh_v1(rest, if v == "1.00" { 0.5 } else { 1.0 })?,
        "2.00" | "3.00" | "3.01" | "4.00" | "4.01" | "5.00" => mesh_binary(rest, &version)?,
        "6.00" | "7.00" => mesh_chunked(rest)?,
        other => bail!("unsupported mesh version {other}"),
    };
    ensure!(!positions.is_empty() && !faces.is_empty(), "the mesh is empty");
    let n = positions.len() as u32;
    ensure!(faces.iter().all(|f| f.iter().all(|&i| i < n)), "a face refers to a missing vertex");
    Ok(Mesh { version, positions, faces, tints: Vec::new() })
}

/// Version 1: a face count line, then `[x,y,z][nx,ny,nz][u,v,w]` three times per face.
fn mesh_v1(rest: &[u8], scale: f32) -> Result<Geometry> {
    let text = String::from_utf8_lossy(rest);
    let mut groups = text.split('[').skip(1).map(|g| g.split(']').next().unwrap_or(""));
    let mut positions = Vec::new();
    // Every third bracket group is a position; the other two are normal and UV.
    while let Some(group) = groups.next() {
        let v: Vec<f32> = group.split(',').filter_map(|n| n.trim().parse().ok()).collect();
        ensure!(v.len() == 3, "malformed vertex in a version 1 mesh");
        positions.push([v[0] * scale, v[1] * scale, v[2] * scale]);
        groups.next();
        groups.next();
    }
    let faces = (0..positions.len() as u32 / 3).map(|i| [i * 3, i * 3 + 1, i * 3 + 2]).collect();
    Ok((positions, faces))
}

fn read_vertices(b: &[u8], at: usize, count: usize, stride: usize) -> Result<Vec<[f32; 3]>> {
    ensure!(stride >= 12 && count <= 20_000_000, "bad vertex layout");
    ensure!(b.len() >= at + count * stride, "vertex data is truncated");
    (0..count).map(|i| Ok([f32at(b, at + i * stride)?, f32at(b, at + i * stride + 4)?, f32at(b, at + i * stride + 8)?])).collect()
}

fn read_faces(b: &[u8], at: usize, count: usize) -> Result<Vec<[u32; 3]>> {
    ensure!(count <= 40_000_000 && b.len() >= at + count * 12, "face data is truncated");
    (0..count).map(|i| Ok([u32at(b, at + i * 12)?, u32at(b, at + i * 12 + 4)?, u32at(b, at + i * 12 + 8)?])).collect()
}

/// Keeps only the most detailed level of detail: faces `lods[0]..lods[1]`.
fn best_lod(mut faces: Vec<[u32; 3]>, lods: &[u32]) -> Vec<[u32; 3]> {
    if let [first, second, ..] = *lods {
        let (first, second) = (first as usize, second as usize);
        if first < second && second <= faces.len() {
            faces.truncate(second);
            faces.drain(..first);
        }
    }
    faces
}

/// Versions 2 to 5: a fixed header, then vertices, optional skinning, faces and LOD offsets.
fn mesh_binary(b: &[u8], version: &str) -> Result<Geometry> {
    let header = u16at(b, 0)? as usize;
    let (stride, verts, faces, lods, bones);
    match version {
        "2.00" => {
            stride = b.get(2).copied().context("truncated header")? as usize;
            (verts, faces, lods, bones) = (u32at(b, 4)?, u32at(b, 8)?, 0, 0);
        }
        "3.00" | "3.01" => {
            stride = b.get(2).copied().context("truncated header")? as usize;
            (lods, verts, faces, bones) = (u16at(b, 6)?, u32at(b, 8)?, u32at(b, 12)?, 0);
        }
        _ => {
            stride = 40;
            (verts, faces, lods, bones) = (u32at(b, 4)?, u32at(b, 8)?, u16at(b, 12)?, u16at(b, 14)?);
        }
    }
    let (verts, faces) = (verts as usize, faces as usize);
    let positions = read_vertices(b, header, verts, stride)?;
    let mut at = header + verts * stride;
    if bones > 0 {
        at += verts * 8;
    }
    let all_faces = read_faces(b, at, faces)?;
    at += faces * 12;
    let offsets: Vec<u32> = (0..lods as usize).map_while(|i| u32at(b, at + i * 4).ok()).collect();
    Ok((positions, best_lod(all_faces, &offsets)))
}

/// Versions 6 and 7: named chunks. `COREMESH` holds the geometry, raw or Draco-compressed.
fn mesh_chunked(b: &[u8]) -> Result<Geometry> {
    let mut at = 0;
    let (mut core, mut lods) = (None, Vec::new());
    while at + 16 <= b.len() {
        let (name, version, size) = (&b[at..at + 8], u32at(b, at + 8)?, u32at(b, at + 12)? as usize);
        let data = b.get(at + 16..at + 16 + size).context("a mesh chunk runs past the end of the file")?;
        match name {
            b"COREMESH" => core = Some((version, data)),
            // u16 type, u8 high-quality count, u32 offset count, then the offsets.
            b"LODS\0\0\0\0" if version == 1 && data.len() >= 7 => {
                let n = u32at(data, 3)? as usize;
                lods = (0..n).map_while(|i| u32at(data, 7 + i * 4).ok()).collect();
            }
            _ => {}
        }
        at += 16 + size;
    }
    let (version, data) = core.context("the mesh has no COREMESH chunk")?;
    let (positions, faces) = match version {
        1 => {
            let verts = u32at(data, 0)? as usize;
            let positions = read_vertices(data, 4, verts, 40)?;
            let faces_at = 4 + verts * 40;
            (positions, read_faces(data, faces_at + 4, u32at(data, faces_at)? as usize)?)
        }
        2 => decode_draco(data.get(4..).context("empty Draco chunk")?)?,
        other => bail!("unsupported COREMESH version {other}"),
    };
    Ok((positions, best_lod(faces, &lods)))
}

fn decode_draco(data: &[u8]) -> Result<Geometry> {
    use draco_core::{DecoderBuffer, FaceIndex, GeometryAttributeType, MeshDecoder};
    let mut mesh = draco_core::Mesh::new();
    let mut buffer = DecoderBuffer::new(data);
    MeshDecoder::new().decode(&mut buffer, &mut mesh).map_err(|e| anyhow!("Draco: {e}"))?;
    let points = mesh.num_points();
    let attribute = mesh.named_attribute(GeometryAttributeType::Position).context("the Draco mesh has no positions")?;
    let flat = attribute.read_f32s(points, 3);
    let positions = flat.as_chunks::<3>().0.to_vec();
    let faces = (0..mesh.num_faces()).map(|i| mesh.face(FaceIndex(i as u32)).map(|p| p.0)).collect();
    Ok((positions, faces))
}

/// Centre and radius of the box around `points`, ignoring any that are not finite.
pub fn bounding_sphere(points: impl Iterator<Item = [f32; 3]>) -> ([f32; 3], f32) {
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for p in points.filter(|p| p.iter().all(|c| c.is_finite())) {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    if lo[0] > hi[0] {
        return ([0.0; 3], 1.0);
    }
    let centre = [0, 1, 2].map(|k| (lo[k] + hi[k]) / 2.0);
    let radius = (0..3).map(|k| (hi[k] - lo[k]).powi(2)).sum::<f32>().sqrt() / 2.0;
    (centre, radius.max(1e-6))
}

impl Mesh {
    /// Centre and radius of the bounding box.
    pub fn bounds(&self) -> ([f32; 3], f32) {
        bounding_sphere(self.positions.iter().copied())
    }

    /// Renders the mesh, lit from the viewer, into an RGBA image with a transparent background.
    pub fn render(&self, view: &View, width: usize, height: usize, tint: [f32; 3]) -> Rgba {
        let View { centre, radius, yaw, pitch, zoom } = *view;
        let scale = width.min(height) as f32 * 0.46 * zoom / radius;
        let (sy, cy, sp, cp) = (yaw.sin(), yaw.cos(), pitch.sin(), pitch.cos());
        // Rotate about the vertical axis, then tilt; keep depth for the z-buffer.
        let projected: Vec<[f32; 3]> = self
            .positions
            .iter()
            .map(|p| {
                let (x, y, z) = (p[0] - centre[0], p[1] - centre[1], p[2] - centre[2]);
                let (x1, z1) = (x * cy + z * sy, z * cy - x * sy);
                let (y2, z2) = (y * cp - z1 * sp, z1 * cp + y * sp);
                [width as f32 / 2.0 + x1 * scale, height as f32 / 2.0 - y2 * scale, z2 * scale]
            })
            .collect();
        let mut pixels = vec![0u8; width * height * 4];
        let mut depth = vec![f32::MIN; width * height];
        for (index, face) in self.faces.iter().enumerate() {
            let tint = self.tints.get(index).copied().unwrap_or(tint);
            let [a, b, c] = face.map(|i| projected[i as usize]);
            let (e1, e2) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
            let normal = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
            let area = normal[2];
            let length = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
            if area.abs() < 1e-9 || !length.is_finite() || length == 0.0 {
                continue;
            }
            // Two-sided shading with a touch of light from above.
            let facing = (normal[2] / length).abs();
            let top = (-normal[1] / length * area.signum()).max(0.0);
            let light = 0.22 + 0.62 * facing + 0.16 * top;
            let color = [(tint[0] * light * 255.0) as u8, (tint[1] * light * 255.0) as u8, (tint[2] * light * 255.0) as u8];
            let min_x = a[0].min(b[0]).min(c[0]).floor().max(0.0) as usize;
            let max_x = (a[0].max(b[0]).max(c[0]).ceil() as isize).min(width as isize - 1);
            let min_y = a[1].min(b[1]).min(c[1]).floor().max(0.0) as usize;
            let max_y = (a[1].max(b[1]).max(c[1]).ceil() as isize).min(height as isize - 1);
            if max_x < 0 || max_y < 0 {
                continue;
            }
            for y in min_y..=max_y as usize {
                let py = y as f32 + 0.5;
                for x in min_x..=max_x as usize {
                    let px = x as f32 + 0.5;
                    // Barycentric weights, normalised by the signed area so either winding works.
                    let w0 = ((b[0] - px) * (c[1] - py) - (b[1] - py) * (c[0] - px)) / area;
                    let w1 = ((c[0] - px) * (a[1] - py) - (c[1] - py) * (a[0] - px)) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let z = w0 * a[2] + w1 * b[2] + w2 * c[2];
                    let i = y * width + x;
                    if z > depth[i] {
                        depth[i] = z;
                        pixels[i * 4..i * 4 + 4].copy_from_slice(&[color[0], color[1], color[2], 255]);
                    }
                }
            }
        }
        Rgba { width, height, pixels }
    }
}

// ------------------------------------------------------------- model trees

pub struct Node {
    pub class: String,
    pub name: String,
    pub properties: Vec<(String, String)>,
    pub children: Vec<Node>,
}

pub struct Model {
    pub roots: Vec<Node>,
    pub instances: usize,
    /// Most common classes, most frequent first.
    pub classes: Vec<(String, usize)>,
    /// True when the tree was cut short to stay responsive.
    pub truncated: bool,
    /// The keyframe animation inside, when it targets an R6 or R15 rig.
    pub animation: Option<crate::anim::Animation>,
}

const MAX_NODES: usize = 4000;
const MAX_VALUE_CHARS: usize = 220;
/// Deepest nesting followed in a model; real ones are a handful of levels deep.
pub const MAX_DEPTH: usize = 64;

fn describe_value(value: &rbx_dom_weak::types::Variant) -> String {
    use rbx_dom_weak::types::Variant;
    let text = match value {
        Variant::String(s) => s.clone(),
        Variant::BinaryString(b) => format!("<{} bytes>", AsRef::<[u8]>::as_ref(b).len()),
        Variant::Bool(v) => v.to_string(),
        Variant::Int32(v) => v.to_string(),
        Variant::Int64(v) => v.to_string(),
        Variant::Float32(v) => v.to_string(),
        Variant::Float64(v) => v.to_string(),
        other => format!("{other:?}"),
    };
    if text.chars().count() > MAX_VALUE_CHARS {
        format!("{}...", text.chars().take(MAX_VALUE_CHARS).collect::<String>())
    } else {
        text
    }
}

fn build_node(dom: &rbx_dom_weak::WeakDom, referent: rbx_dom_weak::types::Ref, depth: usize, budget: &mut usize) -> Option<Node> {
    let instance = dom.get_by_ref(referent)?;
    if *budget == 0 || depth > MAX_DEPTH {
        return None;
    }
    *budget -= 1;
    let mut properties: Vec<(String, String)> = instance.properties.iter().map(|(k, v)| (k.to_string(), describe_value(v))).collect();
    properties.sort();
    let children = instance.children().iter().filter_map(|&c| build_node(dom, c, depth + 1, budget)).collect();
    Some(Node { class: instance.class.to_string(), name: instance.name.clone(), properties, children })
}

/// Parses a binary (`rbxm`) or XML (`rbxmx`) Roblox model into an instance tree.
pub fn parse_model(body: &[u8], xml: bool) -> Result<Model> {
    let dom = if xml {
        rbx_xml::from_reader_default(body).map_err(|e| anyhow!("{e}"))?
    } else {
        rbx_binary::from_reader(body).map_err(|e| anyhow!("{e}"))?
    };
    let mut counts = std::collections::HashMap::new();
    let mut instances = 0usize;
    for instance in dom.descendants().skip(1) {
        instances += 1;
        *counts.entry(instance.class.to_string()).or_insert(0usize) += 1;
    }
    let mut classes: Vec<(String, usize)> = counts.into_iter().collect();
    classes.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    classes.truncate(8);
    let mut budget = MAX_NODES;
    let roots = dom.root().children().iter().filter_map(|&c| build_node(&dom, c, 0, &mut budget)).collect();
    let animation = crate::anim::Animation::from_dom(&dom);
    Ok(Model { roots, instances, classes, truncated: instances > MAX_NODES, animation })
}

/// Converts a payload to a widely supported format: KTX2 to PNG, mesh to OBJ.
/// Returns the new bytes and their file extension.
pub fn convert(kind: &str, body: &[u8]) -> Result<(Vec<u8>, &'static str)> {
    match kind {
        "ktx2" => {
            let image = decode_ktx2(body, u32::MAX)?;
            let mut png = Vec::new();
            image::RgbaImage::from_raw(image.width as u32, image.height as u32, image.pixels)
                .context("decoded texture has the wrong size")?
                .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)?;
            Ok((png, "png"))
        }
        "mesh" => {
            use std::fmt::Write;
            let mesh = parse_mesh(body)?;
            let mut obj = String::new();
            writeln!(obj, "# Roblox mesh version {}", mesh.version)?;
            for p in &mesh.positions {
                writeln!(obj, "v {} {} {}", p[0], p[1], p[2])?;
            }
            for f in &mesh.faces {
                writeln!(obj, "f {} {} {}", f[0] + 1, f[1] + 1, f[2] + 1)?;
            }
            Ok((obj.into_bytes(), "obj"))
        }
        other => bail!("{other} entries have no conversion; export them as they are"),
    }
}

/// The extension [`convert`] produces for this content type, if it supports it.
pub fn converted_extension(kind: &str) -> Option<&'static str> {
    match kind {
        "ktx2" => Some("png"),
        "mesh" => Some("obj"),
        _ => None,
    }
}

/// Re-indents JSON for reading; returns `None` if the text is not JSON.
pub fn pretty_json(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    serde_json::to_string_pretty(&value).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_version_1_mesh() {
        let text = b"version 1.00\n1\n[0,0,0][0,1,0][0,0,0][2,0,0][0,1,0][1,0,0][0,2,0][0,1,0][0,1,0]";
        let mesh = parse_mesh(text).unwrap();
        assert_eq!(mesh.positions, [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
        assert_eq!(mesh.faces, [[0, 1, 2]]);
    }

    fn v2_mesh() -> Vec<u8> {
        let mut b = b"version 2.00\n".to_vec();
        b.extend_from_slice(&12u16.to_le_bytes());
        b.extend_from_slice(&[36, 12]);
        b.extend_from_slice(&3u32.to_le_bytes());
        b.extend_from_slice(&1u32.to_le_bytes());
        for p in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            for c in p {
                b.extend_from_slice(&c.to_le_bytes());
            }
            b.extend_from_slice(&[0u8; 24]);
        }
        for i in [0u32, 1, 2] {
            b.extend_from_slice(&i.to_le_bytes());
        }
        b
    }

    #[test]
    fn parses_version_2_mesh_and_renders_it() {
        let mesh = parse_mesh(&v2_mesh()).unwrap();
        assert_eq!((mesh.positions.len(), mesh.faces.len()), (3, 1));
        let image = mesh.render(&View::new(mesh.bounds(), (0.0, 0.0)), 64, 64, [1.0, 1.0, 1.0]);
        let covered = image.pixels.chunks_exact(4).filter(|p| p[3] == 255).count();
        assert!(covered > 200 && covered < 64 * 64, "covered {covered}");
    }

    #[test]
    fn rejects_bad_meshes() {
        assert!(parse_mesh(b"not a mesh").is_err());
        let mut bad = v2_mesh();
        let n = bad.len();
        bad[n - 4] = 9; // face index out of range
        assert!(parse_mesh(&bad).is_err());
        assert!(parse_mesh(&v2_mesh()[..40]).is_err());
    }

    #[test]
    fn picks_the_first_lod() {
        let faces = vec![[0, 1, 2], [3, 4, 5], [6, 7, 8]];
        assert_eq!(best_lod(faces.clone(), &[0, 2, 3]), [[0, 1, 2], [3, 4, 5]]);
        assert_eq!(best_lod(faces.clone(), &[]), faces);
        assert_eq!(best_lod(faces.clone(), &[0, 9]), faces);
    }

    #[test]
    fn decodes_an_uncompressed_bc1_ktx2() {
        // One 4x4 BC1 block: both endpoint colours pure red, all texels index 0.
        let mut b = b"\xabKTX 20\xbb\r\n\x1a\n".to_vec();
        for v in [131u32, 1, 4, 4, 0, 0, 1, 1, 0] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.resize(80, 0);
        for v in [104u64, 8, 8] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&[0x00, 0xf8, 0x00, 0xf8, 0, 0, 0, 0]);
        let image = decode_ktx2(&b, 4096).unwrap();
        assert_eq!((image.width, image.height), (4, 4));
        assert_eq!(&image.pixels[..4], &[255, 0, 0, 255]);
        assert!(decode_ktx2(&b[..60], 4096).is_err());
    }

    #[test]
    fn formats_json() {
        assert_eq!(pretty_json(b"{\"a\":[1,2]}").unwrap(), "{\n  \"a\": [\n    1,\n    2\n  ]\n}");
        assert!(pretty_json(b"nope").is_none());
    }
}
