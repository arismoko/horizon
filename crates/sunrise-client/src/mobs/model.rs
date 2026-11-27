//! A mob's baked model: the parts of `entity-models.json` as polygons, and the normal stack that packs their
//! normals the way `ModelPart` does.
use serde_json::Value;

#[derive(Clone, Debug)]
pub struct Polygon {
    pub normal: [f32; 3],
    pub vertices: [[f32; 5]; 4],
    /// The model's render type culls back faces (`entityCutoutCull`: the bat,
    /// the baby turtle).
    pub cull_back: bool,
}
impl Polygon {
    pub(crate) fn vertex_normal(&self, pose: ModelNormals) -> [i8; 4] {
        let n = pose.matrix * glam::Vec3::from_array(self.normal);
        let n = if pose.trusted {
            n
        } else {
            n.normalize_or_zero()
        };
        let packed = n.to_array().map(|v| (v.clamp(-1.0, 1.0) * 127.0) as i8);
        let w = if self.cull_back {
            crate::mesher::SHADED_CULL_W
        } else {
            -127
        };
        [packed[0], packed[1], packed[2], w]
    }
}

/// ModelPart uses a separate normal stack: uniform scaling keeps normal lengths,
/// while nonuniform scaling requires normalization after the complete part transform.
/// [`render::compile_part`](super::render::compile_part) gets a part's, to pack its polygons'
/// normals with ([`Polygon`]'s `vertex_normal`).
#[derive(Clone, Copy)]
pub struct ModelNormals {
    pub(super) matrix: glam::Mat3,
    pub(super) trusted: bool,
}
impl ModelNormals {
    // shortcut: infer the root normal pose from its geometry matrix; pass an explicit
    // root normal stack when root-transform rounding is ported as well.
    pub fn new(base: glam::Mat4) -> Self {
        let m = glam::Mat3::from_mat4(base);
        let lengths = glam::Vec3::new(m.x_axis.length(), m.y_axis.length(), m.z_axis.length());
        let uniform =
            lengths.min_element() > 0.0 && lengths.max_element() - lengths.min_element() < 1.0e-6;
        Self {
            matrix: if uniform {
                m / lengths.x
            } else {
                m.inverse().transpose()
            },
            trusted: uniform,
        }
    }
    /// A rigid root pose: its linear part already transforms normals.
    pub fn from_rotation(pose: glam::Mat4) -> Self {
        Self {
            matrix: glam::Mat3::from_mat4(pose),
            trusted: true,
        }
    }

    pub fn rotate(&mut self, rotation: glam::Mat3) {
        self.matrix *= rotation;
    }

    /// `PoseStack.Pose.scale`: test uniform scale exactly and keep reciprocals otherwise.
    pub fn scale(&mut self, scale: glam::Vec3) {
        if scale.x.abs() == scale.y.abs() && scale.x.abs() == scale.z.abs() {
            if scale.min_element() < 0.0 {
                self.matrix *= glam::Mat3::from_diagonal(scale.signum());
            }
        } else {
            self.matrix *= glam::Mat3::from_diagonal(scale.recip());
            self.trusted = false;
        }
    }

    pub(super) fn rotation(x: f32, y: f32, z: f32) -> glam::Mat3 {
        // JOML derives cosine from the rounded sine, then retains quaternion length.
        // Assuming an exactly unit quaternion changes a packed normal by one byte.
        let sin_cos = |angle: f32| {
            let angle = angle * 0.5;
            let sin = f64::from(angle).sin() as f32;
            let cos = (1.0 - sin * sin).sqrt();
            let a = angle + std::f32::consts::FRAC_PI_2;
            let tau = std::f32::consts::TAU;
            let mut quadrant = a - (a / tau) as i32 as f32 * tau;
            if quadrant < 0.0 {
                quadrant += tau;
            }
            (
                sin,
                if quadrant >= std::f32::consts::PI {
                    -cos
                } else {
                    cos
                },
            )
        };
        let (sx, cx) = sin_cos(x);
        let (sy, cy) = sin_cos(y);
        let (sz, cz) = sin_cos(z);
        let (cycz, sysz, sycz, cysz) = (cy * cz, sy * sz, sy * cz, cy * sz);
        let (w, x, y, z) = (
            cx * cycz + sx * sysz,
            sx * cycz - cx * sysz,
            cx * sycz + sx * cysz,
            cx * cysz - sx * sycz,
        );
        let (ww, xx, yy, zz) = (w * w, x * x, y * y, z * z);
        let (xy, xz, xw, yz, yw, zw) = (x * y, x * z, x * w, y * z, y * w, z * w);
        glam::Mat3::from_cols_array(&[
            ww + xx - zz - yy,
            (xy + xy) + (zw + zw),
            (xz + xz) - (yw + yw),
            (xy + xy) - (zw + zw),
            yy - zz + ww - xx,
            (yz + yz) + (xw + xw),
            (yw + yw) + (xz + xz),
            (yz + yz) - (xw + xw),
            zz - yy - xx + ww,
        ])
    }
    pub(crate) fn part(self, pose: [f32; 9]) -> Self {
        self.part_rotated(pose, Self::rotation(pose[3], pose[4], pose[5]))
    }
    pub(super) fn part_rotated(mut self, pose: [f32; 9], rotation: glam::Mat3) -> Self {
        self.rotate(rotation);
        self.scale(glam::Vec3::new(pose[6], pose[7], pose[8]));
        self
    }
}

#[derive(Clone, Debug)]
pub struct Part {
    pub name: String,
    /// x, y, z, xRot, yRot, zRot, xScale, yScale, zScale
    pub pose: [f32; 9],
    pub polygons: Vec<Polygon>,
    pub children: Vec<Part>,
    /// The model layer the tree was baked from (`minecraft:player#main`), on the root of a
    /// layer read from `entity-models.json`; None on its children and
    /// on a model built another way. [`render::compile_part`](super::render::compile_part)
    /// and [`render::takes_part`](super::render::takes_part) are asked with it.
    pub layer: Option<std::sync::Arc<str>>,
}
impl Part {
    /// The model layer `key` (`minecraft:player#main`) of `entity-models.json`, as
    /// [`Part::parse`] bakes its root, recording the layer on it.
    #[cfg(test)]
    pub(crate) fn parse_layer(key: &str, v: &Value) -> Self {
        let mut part = Self::parse("root", v);
        part.layer = Some(key.into());
        part
    }
    /// The direct child named `name` as a model of its own (`PlayerModel.rightArm` drawn alone),
    /// keeping the layer this tree was baked from: its parts are asked about and drawn as the
    /// layer's.
    pub fn detached_child(&self, name: &str) -> Option<Part> {
        let mut child = self.children.iter().find(|c| c.name == name)?.clone();
        child.layer.clone_from(&self.layer);
        Some(child)
    }
    /// Marks every polygon as drawn by a back-face-culling render type.
    pub(crate) fn set_cull_back(&mut self) {
        for poly in &mut self.polygons {
            poly.cull_back = true;
        }
        for child in &mut self.children {
            child.set_cull_back();
        }
    }
    #[cfg(test)]
    pub(crate) fn parse(name: &str, value: &Value) -> Self {
        Self::decode(name, value.clone()).expect("valid test model part")
    }

    pub(super) fn empty(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            pose: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            polygons: Vec::new(),
            children: Vec::new(),
            layer: None,
        }
    }

    pub(crate) fn decode(name: &str, value: Value) -> Result<Self, String> {
        fn field<T: serde::de::DeserializeOwned>(
            fields: &mut serde_json::Map<String, Value>,
            key: &str,
        ) -> Result<Option<T>, String> {
            fields
                .remove(key)
                .map(serde_json::from_value)
                .transpose()
                .map_err(|error| format!("{key}: {error}"))
        }

        let Value::Object(mut fields) = value else {
            return Err("model part must be an object".into());
        };
        let mut part = Self::empty(name);
        if let Some(pose) = field::<Vec<f32>>(&mut fields, "pose")? {
            // The prepared exporter writes ModelPart.loadPose's three offsets, angles and scales.
            if pose.len() > part.pose.len() || pose.iter().any(|x| !x.is_finite()) {
                return Err("invalid model pose".into());
            }
            part.pose[..pose.len()].copy_from_slice(&pose);
        }
        // Vanilla ModelPart.Cube exports quads; ModelPart.Vertex has xyz and uv.
        let polygons: Vec<[[f32; 5]; 4]> = field(&mut fields, "polygons")?.unwrap_or_default();
        let normals: Option<Vec<[f32; 3]>> = field(&mut fields, "normals")?;
        if normals
            .as_ref()
            .is_some_and(|normals| normals.len() != polygons.len())
        {
            return Err("normal count differs from polygon count".into());
        }
        if polygons.iter().flatten().flatten().any(|x| !x.is_finite())
            || normals.iter().flatten().flatten().any(|x| !x.is_finite())
        {
            return Err("non-finite model geometry".into());
        }
        part.polygons = polygons
            .into_iter()
            .enumerate()
            .map(|(index, vertices)| {
                let normal = normals.as_ref().map_or_else(
                    || {
                        // Legacy exports omitted normals; collapsed faces need re-preparation.
                        let [a, b, c, _] = vertices.map(|[x, y, z, ..]| glam::Vec3::new(x, y, z));
                        (b - a).cross(c - a).normalize_or_zero().to_array()
                    },
                    |normals| normals[index],
                );
                Polygon {
                    vertices,
                    normal,
                    cull_back: false,
                }
            })
            .collect();
        if let Some(children) = fields.remove("children") {
            let Value::Object(children) = children else {
                return Err("children: expected an object".into());
            };
            for (name, child) in children {
                part.children.push(
                    Self::decode(&name, child).map_err(|error| format!("child {name}: {error}"))?,
                );
            }
            part.children.sort_by(|a, b| a.name.cmp(&b.name));
        }
        // A model whose root holds a part also named "root" (allay, vex,
        // frog...): that inner part is what `root.getChild("root")` and the
        // keyframe bones mean, so the outer one gives up the name.
        if name == "root" && part.children.iter().any(|c| c.name == "root") {
            part.name.clear();
        }
        Ok(part)
    }
}

impl super::anim::PosePart for Part {
    fn part_name(&self) -> &str {
        &self.name
    }

    fn bind_pose(&self) -> [f32; 9] {
        self.pose
    }

    fn visit_parts(&self, f: &mut impl FnMut(&str, [f32; 9])) {
        f(&self.name, self.pose);
        for child in &self.children {
            child.visit_parts(f);
        }
    }

    fn part_identity(&self) -> usize {
        self as *const Self as usize
    }
}
