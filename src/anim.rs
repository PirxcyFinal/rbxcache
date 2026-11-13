//! Keyframe animations and the R6 / R15 rigs they are played on.
//!
//! A `KeyframeSequence` stores, per keyframe, a tree of `Pose` instances named
//! after the body part they move. Each pose is the `Transform` of the joint
//! that attaches that part to its parent, so a part ends up at
//! `parent * C0 * transform * C1^-1`.

use crate::viewers::{bounding_sphere, Mesh, MAX_DEPTH};
use rbx_dom_weak::types::{Ref, Variant};
use rbx_dom_weak::WeakDom;
use std::collections::HashMap;

/// A rotation (rows of a 3x3 matrix) and a position, as Roblox's `CFrame`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CFrame {
    pub r: [[f32; 3]; 3],
    pub p: [f32; 3],
}

impl CFrame {
    pub const IDENTITY: CFrame = CFrame { r: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], p: [0.0; 3] };

    const fn at(x: f32, y: f32, z: f32) -> CFrame {
        CFrame { p: [x, y, z], ..CFrame::IDENTITY }
    }

    pub fn point(&self, v: [f32; 3]) -> [f32; 3] {
        let r = &self.r;
        [
            r[0][0] * v[0] + r[0][1] * v[1] + r[0][2] * v[2] + self.p[0],
            r[1][0] * v[0] + r[1][1] * v[1] + r[1][2] * v[2] + self.p[1],
            r[2][0] * v[0] + r[2][1] * v[1] + r[2][2] * v[2] + self.p[2],
        ]
    }

    pub fn mul(&self, other: &CFrame) -> CFrame {
        let mut r = [[0.0; 3]; 3];
        for (i, row) in r.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell = (0..3).map(|k| self.r[i][k] * other.r[k][j]).sum();
            }
        }
        CFrame { r, p: self.point(other.p) }
    }

    pub fn inverse(&self) -> CFrame {
        let r = &self.r;
        let t = [[r[0][0], r[1][0], r[2][0]], [r[0][1], r[1][1], r[2][1]], [r[0][2], r[1][2], r[2][2]]];
        let p = [
            -(t[0][0] * self.p[0] + t[0][1] * self.p[1] + t[0][2] * self.p[2]),
            -(t[1][0] * self.p[0] + t[1][1] * self.p[1] + t[1][2] * self.p[2]),
            -(t[2][0] * self.p[0] + t[2][1] * self.p[1] + t[2][2] * self.p[2]),
        ];
        CFrame { r: t, p }
    }

    fn quaternion(&self) -> [f32; 4] {
        let m = &self.r;
        let trace = m[0][0] + m[1][1] + m[2][2];
        let q = if trace > 0.0 {
            let s = (trace + 1.0).sqrt() * 2.0;
            [(m[2][1] - m[1][2]) / s, (m[0][2] - m[2][0]) / s, (m[1][0] - m[0][1]) / s, s / 4.0]
        } else if m[0][0] > m[1][1] && m[0][0] > m[2][2] {
            let s = (1.0 + m[0][0] - m[1][1] - m[2][2]).sqrt() * 2.0;
            [s / 4.0, (m[0][1] + m[1][0]) / s, (m[0][2] + m[2][0]) / s, (m[2][1] - m[1][2]) / s]
        } else if m[1][1] > m[2][2] {
            let s = (1.0 + m[1][1] - m[0][0] - m[2][2]).sqrt() * 2.0;
            [(m[0][1] + m[1][0]) / s, s / 4.0, (m[1][2] + m[2][1]) / s, (m[0][2] - m[2][0]) / s]
        } else {
            let s = (1.0 + m[2][2] - m[0][0] - m[1][1]).sqrt() * 2.0;
            [(m[0][2] + m[2][0]) / s, (m[1][2] + m[2][1]) / s, s / 4.0, (m[1][0] - m[0][1]) / s]
        };
        normalize(q)
    }

    fn from_quaternion([x, y, z, w]: [f32; 4], p: [f32; 3]) -> CFrame {
        let r = [
            [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - z * w), 2.0 * (x * z + y * w)],
            [2.0 * (x * y + z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - x * w)],
            [2.0 * (x * z - y * w), 2.0 * (y * z + x * w), 1.0 - 2.0 * (x * x + y * y)],
        ];
        CFrame { r, p }
    }

    /// Blends toward `other`: positions linearly, rotations along the shortest arc.
    pub fn lerp(&self, other: &CFrame, t: f32) -> CFrame {
        let (a, mut b) = (self.quaternion(), other.quaternion());
        let mut dot: f32 = (0..4).map(|i| a[i] * b[i]).sum();
        if dot < 0.0 {
            b = b.map(|c| -c);
            dot = -dot;
        }
        let (wa, wb) = if dot > 0.9995 {
            (1.0 - t, t)
        } else {
            let angle = dot.clamp(-1.0, 1.0).acos();
            (((1.0 - t) * angle).sin() / angle.sin(), (t * angle).sin() / angle.sin())
        };
        let q = normalize([0, 1, 2, 3].map(|i| a[i] * wa + b[i] * wb));
        let p = [0, 1, 2].map(|i| self.p[i] + (other.p[i] - self.p[i]) * t);
        CFrame::from_quaternion(q, p)
    }
}

fn normalize(q: [f32; 4]) -> [f32; 4] {
    let length = q.iter().map(|c| c * c).sum::<f32>().sqrt();
    if length > 0.0 {
        q.map(|c| c / length)
    } else {
        [0.0, 0.0, 0.0, 1.0]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Rig {
    R6,
    R15,
}

impl Rig {
    pub fn name(self) -> &'static str {
        match self {
            Rig::R6 => "R6",
            Rig::R15 => "R15",
        }
    }
}

/// A body part and the joint that hangs it off its parent.
struct Joint {
    part: &'static str,
    parent: &'static str,
    c0: CFrame,
    c1: CFrame,
    size: [f32; 3],
    tint: [f32; 3],
}

const SKIN: [f32; 3] = [0.96, 0.80, 0.42];
const SHIRT: [f32; 3] = [0.36, 0.52, 0.98];
const LEGS: [f32; 3] = [0.42, 0.78, 0.50];

/// R6 joints carry a fixed rotation in both attachment frames.
const fn turned(x: f32, y: f32, z: f32, r: [[f32; 3]; 3]) -> CFrame {
    CFrame { r, p: [x, y, z] }
}

const ROOT_TURN: [[f32; 3]; 3] = [[-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]];
const RIGHT_TURN: [[f32; 3]; 3] = [[0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [-1.0, 0.0, 0.0]];
const LEFT_TURN: [[f32; 3]; 3] = [[0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]];

/// The classic six-part rig. Parents always come before their children.
const R6: &[Joint] = &[
    Joint {
        part: "Torso",
        parent: "HumanoidRootPart",
        c0: turned(0.0, 0.0, 0.0, ROOT_TURN),
        c1: turned(0.0, 0.0, 0.0, ROOT_TURN),
        size: [2.0, 2.0, 1.0],
        tint: SHIRT,
    },
    Joint {
        part: "Head",
        parent: "Torso",
        c0: turned(0.0, 1.0, 0.0, ROOT_TURN),
        c1: turned(0.0, -0.5, 0.0, ROOT_TURN),
        size: [1.2, 1.0, 1.0],
        tint: SKIN,
    },
    Joint {
        part: "Right Arm",
        parent: "Torso",
        c0: turned(1.0, 0.5, 0.0, RIGHT_TURN),
        c1: turned(-0.5, 0.5, 0.0, RIGHT_TURN),
        size: [1.0, 2.0, 1.0],
        tint: SKIN,
    },
    Joint {
        part: "Left Arm",
        parent: "Torso",
        c0: turned(-1.0, 0.5, 0.0, LEFT_TURN),
        c1: turned(0.5, 0.5, 0.0, LEFT_TURN),
        size: [1.0, 2.0, 1.0],
        tint: SKIN,
    },
    Joint {
        part: "Right Leg",
        parent: "Torso",
        c0: turned(1.0, -1.0, 0.0, RIGHT_TURN),
        c1: turned(0.5, 1.0, 0.0, RIGHT_TURN),
        size: [1.0, 2.0, 1.0],
        tint: LEGS,
    },
    Joint {
        part: "Left Leg",
        parent: "Torso",
        c0: turned(-1.0, -1.0, 0.0, LEFT_TURN),
        c1: turned(-0.5, 1.0, 0.0, LEFT_TURN),
        size: [1.0, 2.0, 1.0],
        tint: LEGS,
    },
];

/// The fifteen-part rig, with the proportions of the default blocky character.
const R15: &[Joint] = &[
    Joint {
        part: "LowerTorso",
        parent: "HumanoidRootPart",
        c0: CFrame::at(0.0, -1.0, 0.0),
        c1: CFrame::at(0.0, -0.2, 0.0),
        size: [2.0, 0.4, 1.0],
        tint: SHIRT,
    },
    Joint {
        part: "UpperTorso",
        parent: "LowerTorso",
        c0: CFrame::at(0.0, 0.2, 0.0),
        c1: CFrame::at(0.0, -0.8, 0.0),
        size: [2.0, 1.6, 1.0],
        tint: SHIRT,
    },
    Joint { part: "Head", parent: "UpperTorso", c0: CFrame::at(0.0, 0.8, 0.0), c1: CFrame::at(0.0, -0.5, 0.0), size: [1.2, 1.0, 1.0], tint: SKIN },
    Joint {
        part: "RightUpperArm",
        parent: "UpperTorso",
        c0: CFrame::at(1.0, 0.563, 0.0),
        c1: CFrame::at(-0.5, 0.394, 0.0),
        size: [1.0, 1.169, 1.0],
        tint: SKIN,
    },
    Joint {
        part: "RightLowerArm",
        parent: "RightUpperArm",
        c0: CFrame::at(0.0, -0.334, 0.0),
        c1: CFrame::at(0.0, 0.259, 0.0),
        size: [1.0, 1.052, 1.0],
        tint: SKIN,
    },
    Joint {
        part: "RightHand",
        parent: "RightLowerArm",
        c0: CFrame::at(0.0, -0.502, 0.0),
        c1: CFrame::at(0.0, 0.125, 0.0),
        size: [1.0, 0.3, 1.0],
        tint: SKIN,
    },
    Joint {
        part: "LeftUpperArm",
        parent: "UpperTorso",
        c0: CFrame::at(-1.0, 0.563, 0.0),
        c1: CFrame::at(0.5, 0.394, 0.0),
        size: [1.0, 1.169, 1.0],
        tint: SKIN,
    },
    Joint {
        part: "LeftLowerArm",
        parent: "LeftUpperArm",
        c0: CFrame::at(0.0, -0.334, 0.0),
        c1: CFrame::at(0.0, 0.259, 0.0),
        size: [1.0, 1.052, 1.0],
        tint: SKIN,
    },
    Joint {
        part: "LeftHand",
        parent: "LeftLowerArm",
        c0: CFrame::at(0.0, -0.502, 0.0),
        c1: CFrame::at(0.0, 0.125, 0.0),
        size: [1.0, 0.3, 1.0],
        tint: SKIN,
    },
    Joint {
        part: "RightUpperLeg",
        parent: "LowerTorso",
        c0: CFrame::at(0.5, -0.2, 0.0),
        c1: CFrame::at(0.0, 0.421, 0.0),
        size: [1.0, 1.217, 1.0],
        tint: LEGS,
    },
    Joint {
        part: "RightLowerLeg",
        parent: "RightUpperLeg",
        c0: CFrame::at(0.0, -0.401, 0.0),
        c1: CFrame::at(0.0, 0.379, 0.0),
        size: [1.0, 1.193, 1.0],
        tint: LEGS,
    },
    Joint {
        part: "RightFoot",
        parent: "RightLowerLeg",
        c0: CFrame::at(0.0, -0.547, 0.0),
        c1: CFrame::at(0.0, 0.102, 0.0),
        size: [1.0, 0.3, 1.0],
        tint: LEGS,
    },
    Joint {
        part: "LeftUpperLeg",
        parent: "LowerTorso",
        c0: CFrame::at(-0.5, -0.2, 0.0),
        c1: CFrame::at(0.0, 0.421, 0.0),
        size: [1.0, 1.217, 1.0],
        tint: LEGS,
    },
    Joint {
        part: "LeftLowerLeg",
        parent: "LeftUpperLeg",
        c0: CFrame::at(0.0, -0.401, 0.0),
        c1: CFrame::at(0.0, 0.379, 0.0),
        size: [1.0, 1.193, 1.0],
        tint: LEGS,
    },
    Joint {
        part: "LeftFoot",
        parent: "LeftLowerLeg",
        c0: CFrame::at(0.0, -0.547, 0.0),
        c1: CFrame::at(0.0, 0.102, 0.0),
        size: [1.0, 0.3, 1.0],
        tint: LEGS,
    },
];

#[derive(Clone, Copy, Debug, PartialEq)]
enum Style {
    Linear,
    Constant,
    Elastic,
    Cubic,
    Bounce,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Direction {
    In,
    Out,
    InOut,
}

fn bounce_out(t: f32) -> f32 {
    const N: f32 = 7.5625;
    const D: f32 = 2.75;
    if t < 1.0 / D {
        N * t * t
    } else if t < 2.0 / D {
        let t = t - 1.5 / D;
        N * t * t + 0.75
    } else if t < 2.5 / D {
        let t = t - 2.25 / D;
        N * t * t + 0.9375
    } else {
        let t = t - 2.625 / D;
        N * t * t + 0.984375
    }
}

fn elastic_out(t: f32) -> f32 {
    if t <= 0.0 || t >= 1.0 {
        return t.clamp(0.0, 1.0);
    }
    2f32.powf(-10.0 * t) * ((t - 0.075) * std::f32::consts::TAU / 0.3).sin() + 1.0
}

/// How far along a transition is at `t` (0..1) for a pose's easing settings.
fn ease(style: Style, direction: Direction, t: f32) -> f32 {
    let out = |t: f32| match style {
        Style::Cubic => 1.0 - (1.0 - t).powi(3),
        Style::Bounce => bounce_out(t),
        Style::Elastic => elastic_out(t),
        Style::Linear | Style::Constant => t,
    };
    match (style, direction) {
        (Style::Linear, _) => t,
        // A constant pose holds until the next keyframe.
        (Style::Constant, _) => 0.0,
        (_, Direction::Out) => out(t),
        (_, Direction::In) => 1.0 - out(1.0 - t),
        (_, Direction::InOut) if t < 0.5 => (1.0 - out(1.0 - 2.0 * t)) / 2.0,
        (_, Direction::InOut) => 0.5 + out(2.0 * t - 1.0) / 2.0,
    }
}

struct Key {
    time: f32,
    transform: CFrame,
    style: Style,
    direction: Direction,
}

pub struct Animation {
    pub rig: Rig,
    pub length: f32,
    pub looped: bool,
    pub keyframes: usize,
    /// Keys per body part, ordered by time.
    tracks: HashMap<String, Vec<Key>>,
}

fn pose_key(instance: &rbx_dom_weak::Instance, time: f32) -> Option<Key> {
    let property = |name: &str| instance.properties.iter().find(|(k, _)| k.as_str() == name).map(|(_, v)| v);
    // A pose with no weight is only there to hold the tree together.
    if matches!(property("Weight"), Some(Variant::Float32(w)) if *w <= 0.0) {
        return None;
    }
    let Some(Variant::CFrame(cf)) = property("CFrame") else { return None };
    let row = |v: rbx_dom_weak::types::Vector3| [v.x, v.y, v.z];
    let transform = CFrame { r: [row(cf.orientation.x), row(cf.orientation.y), row(cf.orientation.z)], p: row(cf.position) };
    let number = |name: &str| match property(name) {
        Some(Variant::Enum(e)) => e.to_u32(),
        _ => 0,
    };
    let style = match number("EasingStyle") {
        1 => Style::Constant,
        2 => Style::Elastic,
        3 => Style::Cubic,
        4 => Style::Bounce,
        _ => Style::Linear,
    };
    let direction = match number("EasingDirection") {
        1 => Direction::Out,
        2 => Direction::InOut,
        _ => Direction::In,
    };
    Some(Key { time, transform, style, direction })
}

fn collect_poses(dom: &WeakDom, referent: Ref, time: f32, depth: usize, tracks: &mut HashMap<String, Vec<Key>>) {
    let Some(instance) = dom.get_by_ref(referent) else { return };
    if depth > MAX_DEPTH {
        return;
    }
    for &child in instance.children() {
        let Some(pose) = dom.get_by_ref(child) else { continue };
        if pose.class.as_str() != "Pose" {
            continue;
        }
        if let Some(key) = pose_key(pose, time) {
            tracks.entry(pose.name.clone()).or_default().push(key);
        }
        collect_poses(dom, child, time, depth + 1, tracks);
    }
}

impl Animation {
    /// Reads the first `KeyframeSequence` in a model, if it targets an R6 or R15 rig.
    pub fn from_dom(dom: &WeakDom) -> Option<Animation> {
        let sequence = dom.descendants().find(|i| i.class.as_str() == "KeyframeSequence")?;
        let mut tracks: HashMap<String, Vec<Key>> = HashMap::new();
        let (mut length, mut keyframes) = (0.0f32, 0);
        for &child in sequence.children() {
            let Some(keyframe) = dom.get_by_ref(child) else { continue };
            if keyframe.class.as_str() != "Keyframe" {
                continue;
            }
            let time = keyframe
                .properties
                .iter()
                .find_map(|(k, v)| match v {
                    Variant::Float32(t) if k.as_str() == "Time" => Some(*t),
                    _ => None,
                })
                .unwrap_or(0.0);
            length = length.max(time);
            keyframes += 1;
            collect_poses(dom, child, time, 0, &mut tracks);
        }
        for keys in tracks.values_mut() {
            keys.sort_by(|a, b| a.time.total_cmp(&b.time));
        }
        let rig = if tracks.contains_key("UpperTorso") || tracks.contains_key("LowerTorso") {
            Rig::R15
        } else if tracks.contains_key("Torso") {
            Rig::R6
        } else {
            return None;
        };
        let looped = sequence.properties.iter().any(|(k, v)| k.as_str() == "Loop" && matches!(v, Variant::Bool(true)));
        Some(Animation { rig, length, looped, keyframes, tracks })
    }

    /// The joint transform of `part` at `time`, eased between its two nearest keys.
    fn transform(&self, part: &str, time: f32) -> CFrame {
        let Some(keys) = self.tracks.get(part) else { return CFrame::IDENTITY };
        let next = keys.partition_point(|k| k.time <= time);
        match (next.checked_sub(1).map(|i| &keys[i]), keys.get(next)) {
            (Some(a), Some(b)) => {
                let span = b.time - a.time;
                let t = if span > 0.0 { (time - a.time) / span } else { 1.0 };
                a.transform.lerp(&b.transform, ease(a.style, a.direction, t))
            }
            (Some(only), None) | (None, Some(only)) => only.transform,
            (None, None) => CFrame::IDENTITY,
        }
    }

    /// The rig posed at `time`, as one box per body part.
    pub fn pose(&self, time: f32) -> Mesh {
        let joints = match self.rig {
            Rig::R6 => R6,
            Rig::R15 => R15,
        };
        let mut world: HashMap<&str, CFrame> = HashMap::from([("HumanoidRootPart", CFrame::IDENTITY)]);
        let mut mesh = Mesh { version: String::new(), positions: Vec::new(), faces: Vec::new(), tints: Vec::new() };
        for joint in joints {
            let parent = world[joint.parent];
            let frame = parent.mul(&joint.c0).mul(&self.transform(joint.part, time)).mul(&joint.c1.inverse());
            world.insert(joint.part, frame);
            add_box(&mut mesh, &frame, joint.size, joint.tint);
        }
        mesh
    }

    /// Centre and radius of a view that keeps the whole animation in frame.
    pub fn bounds(&self) -> ([f32; 3], f32) {
        const SAMPLES: usize = 24;
        let poses = (0..=SAMPLES).flat_map(|step| self.pose(self.length * step as f32 / SAMPLES as f32).positions);
        bounding_sphere(poses)
    }
}

fn add_box(mesh: &mut Mesh, frame: &CFrame, size: [f32; 3], tint: [f32; 3]) {
    let base = mesh.positions.len() as u32;
    for corner in 0..8 {
        let sign = |bit: u32| if corner & bit != 0 { 0.5 } else { -0.5 };
        mesh.positions.push(frame.point([size[0] * sign(1), size[1] * sign(2), size[2] * sign(4)]));
    }
    // Two triangles for each of the six faces.
    const FACES: [[u32; 3]; 12] =
        [[0, 1, 3], [0, 3, 2], [4, 6, 7], [4, 7, 5], [0, 4, 5], [0, 5, 1], [2, 3, 7], [2, 7, 6], [0, 2, 6], [0, 6, 4], [1, 5, 7], [1, 7, 3]];
    for face in FACES {
        mesh.faces.push(face.map(|i| base + i));
        mesh.tints.push(tint);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-4)
    }

    fn rest(rig: Rig) -> Animation {
        Animation { rig, length: 1.0, looped: false, keyframes: 0, tracks: HashMap::new() }
    }

    /// Centre of the box built for the `index`-th joint of a posed rig.
    fn centre(mesh: &Mesh, index: usize) -> [f32; 3] {
        let corners = &mesh.positions[index * 8..index * 8 + 8];
        [0, 1, 2].map(|k| corners.iter().map(|c| c[k]).sum::<f32>() / 8.0)
    }

    #[test]
    fn inverse_undoes_a_frame() {
        let frame = turned(1.0, 2.0, 3.0, RIGHT_TURN);
        let back = frame.mul(&frame.inverse());
        assert!(close(back.p, [0.0; 3]));
        assert!(close(back.point([4.0, 5.0, 6.0]), [4.0, 5.0, 6.0]));
    }

    #[test]
    fn r6_stands_upright_at_rest() {
        let mesh = rest(Rig::R6).pose(0.0);
        assert_eq!(mesh.faces.len(), 6 * 12);
        assert!(close(centre(&mesh, 0), [0.0, 0.0, 0.0]), "torso");
        assert!(close(centre(&mesh, 1), [0.0, 1.5, 0.0]), "head");
        assert!(close(centre(&mesh, 2), [1.5, 0.0, 0.0]), "right arm");
        assert!(close(centre(&mesh, 3), [-1.5, 0.0, 0.0]), "left arm");
        assert!(close(centre(&mesh, 4), [0.5, -2.0, 0.0]), "right leg");
        assert!(close(centre(&mesh, 5), [-0.5, -2.0, 0.0]), "left leg");
    }

    #[test]
    fn r15_stacks_from_feet_to_head_at_rest() {
        let mesh = rest(Rig::R15).pose(0.0);
        assert_eq!(mesh.faces.len(), 15 * 12);
        let y = |i: usize| centre(&mesh, i)[1];
        // foot < lower leg < upper leg < lower torso < upper torso < head
        assert!(y(11) < y(10) && y(10) < y(9) && y(9) < y(0) && y(0) < y(1) && y(1) < y(2));
        assert!(centre(&mesh, 3)[0] > 1.0 && centre(&mesh, 6)[0] < -1.0, "arms on either side");
    }

    #[test]
    fn interpolates_between_keys() {
        let key = |time, y| Key { time, transform: CFrame::at(0.0, y, 0.0), style: Style::Linear, direction: Direction::In };
        let mut animation = rest(Rig::R6);
        animation.tracks.insert("Torso".into(), vec![key(0.0, 0.0), key(1.0, 2.0)]);
        assert!(close(animation.transform("Torso", 0.5).p, [0.0, 1.0, 0.0]));
        assert!(close(animation.transform("Torso", 5.0).p, [0.0, 2.0, 0.0]));
        assert_eq!(animation.transform("Head", 0.5), CFrame::IDENTITY);
    }

    #[test]
    fn rotations_blend_along_the_arc() {
        let quarter = CFrame { r: [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]], p: [0.0; 3] };
        let half = CFrame::IDENTITY.lerp(&quarter, 0.5);
        let s = std::f32::consts::FRAC_1_SQRT_2;
        assert!(close(half.point([1.0, 0.0, 0.0]), [s, s, 0.0]));
    }

    #[test]
    fn easing_hits_both_ends() {
        for style in [Style::Linear, Style::Elastic, Style::Cubic, Style::Bounce] {
            for direction in [Direction::In, Direction::Out, Direction::InOut] {
                assert!(ease(style, direction, 0.0).abs() < 1e-4, "{style:?} {direction:?} start");
                assert!((ease(style, direction, 1.0) - 1.0).abs() < 1e-4, "{style:?} {direction:?} end");
            }
        }
        assert_eq!(ease(Style::Constant, Direction::In, 0.9), 0.0);
    }
}
