//! Display/TextDisplay synced data (26.3), checked against vanilla accessors
//! and ClientboundSetEntityDataPacket's own stream codec.
use crate::text::Text;
use serde::Serialize;
use std::sync::Arc;
use sunrise_protocol::entity_data::Value;
pub use sunrise_sim::text_display_data::resident;
pub use sunrise_sim::text_display_data::{display_data, text_display_data, transformation};

/// `TextDisplay.Align`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Center,
    Left,
    Right,
}

/// `Display` and `TextDisplay`'s synced data (ids 8 to 27); the default is
/// what their `defineSynchedData` starts with.
#[derive(Clone, Debug, Serialize)]
pub struct TextDisplayData {
    pub interpolation_delay: i32,
    pub interpolation_duration: i32,
    pub teleport_duration: i32,
    pub translation: [f32; 3],
    pub scale: [f32; 3],
    pub left_rotation: [f32; 4],
    pub right_rotation: [f32; 4],
    pub billboard: u8,
    pub brightness: i32,
    pub view_range: f32,
    pub shadow_radius: f32,
    pub shadow_strength: f32,
    pub width: f32,
    pub height: f32,
    pub glow_color: i32,
    /// Shared: a frame's interpolated copy of the data must not copy it.
    #[serde(skip)]
    pub text: Arc<Text>,
    pub line_width: i32,
    pub background: i32,
    pub opacity: i8,
    pub flags: u8,
}

impl Default for TextDisplayData {
    fn default() -> Self {
        Self {
            interpolation_delay: 0,
            interpolation_duration: 0,
            teleport_duration: 0,
            translation: [0.0; 3],
            scale: [1.0; 3],
            left_rotation: [0.0, 0.0, 0.0, 1.0],
            right_rotation: [0.0, 0.0, 0.0, 1.0],
            billboard: 0,
            brightness: -1,
            view_range: 1.0,
            shadow_radius: 0.0,
            shadow_strength: 1.0,
            width: 0.0,
            height: 0.0,
            glow_color: -1,
            text: Arc::new(Text::empty()),
            line_width: 200,
            background: 0x4000_0000,
            opacity: -1,
            flags: 0,
        }
    }
}

/// `org.joml.Quaternionf.slerp(Quaternionfc, float, Quaternionf)` without unit-length assertion.
fn joml_slerp(from: [f32; 4], to: [f32; 4], alpha: f32) -> [f32; 4] {
    let cosom = from[0] * to[0] + (from[1] * to[1] + (from[2] * to[2] + from[3] * to[3]));
    let abs_cosom = cosom.abs();
    let (scale0, mut scale1) = if 1.0 - abs_cosom > 1.0e-6 {
        let sin_sqr = (-abs_cosom) * abs_cosom + 1.0;
        let sinom = 1.0 / (f64::from(sin_sqr).sqrt() as f32);
        let omega =
            sunrise_core::jmath::atan2(f64::from(sin_sqr * sinom), f64::from(abs_cosom)) as f32;
        (
            crate::camera::joml::sin((1.0 - alpha) * omega) * sinom,
            crate::camera::joml::sin(alpha * omega) * sinom,
        )
    } else {
        (1.0 - alpha, alpha)
    };
    if cosom < 0.0 {
        scale1 = -scale1;
    }
    std::array::from_fn(|i| scale0 * from[i] + scale1 * to[i])
}

impl TextDisplayData {
    /// DisplayRenderer's camera-relative pose: translate, billboard orientation, then T*L*S*R.
    pub fn pose(
        &self,
        offset: [f32; 3],
        entity: [f32; 2],
        camera: [f32; 2],
    ) -> crate::camera::joml::Mat4 {
        use crate::camera::joml::{Mat4, Quat};
        const DEG_TO_RAD: f32 = 0.017_453_292;
        let [yaw, pitch] = match self.billboard {
            1 => [camera[0] - 180.0, entity[1]],
            2 => [entity[0], -camera[1]],
            3 => [camera[0] - 180.0, -camera[1]],
            _ => entity,
        };
        let orientation = Quat::rotation_yxz(-DEG_TO_RAD * yaw, DEG_TO_RAD * pitch, 0.0);
        let mut pose = Mat4::IDENTITY;
        pose.translate(offset[0], offset[1], offset[2]);
        pose.rotate(orientation);
        let mut tx = Mat4::IDENTITY;
        tx.translation(
            self.translation[0],
            self.translation[1],
            self.translation[2],
        );
        let quat = |q: [f32; 4]| Quat {
            x: q[0],
            y: q[1],
            z: q[2],
            w: q[3],
        };
        tx.rotate(quat(self.left_rotation));
        tx.scale(self.scale[0], self.scale[1], self.scale[2]);
        tx.rotate(quat(self.right_rotation));
        pose.mul_affine(&tx)
    }

    /// DisplayRenderer's billboard orientation followed by Transformation's T*L*S*R.
    pub fn matrix(&self, entity: [f32; 2], camera: [f32; 2]) -> glam::Mat4 {
        self.pose([0.0; 3], entity, camera).to_glam()
    }

    pub fn blend(&self, from: &Self, t: f32) -> Self {
        let mut out = self.clone();
        // Display$TransformationInterpolator.get: returns the target transform verbatim at t >= 1.
        if t < 1.0 {
            for i in 0..3 {
                out.translation[i] =
                    from.translation[i] + t * (self.translation[i] - from.translation[i]);
                out.scale[i] = from.scale[i] + t * (self.scale[i] - from.scale[i]);
            }
            out.left_rotation = joml_slerp(from.left_rotation, self.left_rotation, t);
            out.right_rotation = joml_slerp(from.right_rotation, self.right_rotation, t);
        }
        // Display$LinearFloatInterpolator: Mth.lerp(t, previous, current).
        out.shadow_radius = from.shadow_radius + t * (self.shadow_radius - from.shadow_radius);
        out.shadow_strength =
            from.shadow_strength + t * (self.shadow_strength - from.shadow_strength);
        let int = |a: i32, b: i32| a + (t * (b as f32 - a as f32)).floor() as i32;
        out.opacity = int(i32::from(from.opacity), i32::from(self.opacity)) as i8;
        out.background = (0..4).fold(0, |color, channel| {
            color
                | (int(
                    (from.background as u32 >> (channel * 8) & 255) as i32,
                    (self.background as u32 >> (channel * 8) & 255) as i32,
                ) << (channel * 8))
        });
        out
    }
}

/// `java.lang.Math.min(double, double)`: NaN propagates and `-0.0 < +0.0`.
fn java_min_f64(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == 0.0 && b == 0.0 {
        f64::from_bits(a.to_bits() | b.to_bits())
    } else if a <= b {
        a
    } else {
        b
    }
}

/// `java.lang.Math.max(double, double)`: NaN propagates and `+0.0 > -0.0`.
fn java_max_f64(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == 0.0 && b == 0.0 {
        f64::from_bits(a.to_bits() & b.to_bits())
    } else if a >= b {
        a
    } else {
        b
    }
}

/// `Display.updateCulling`: either edge at 0 sets `noCulling`, so the renderer
/// skips the frustum box (`DisplayRenderer.affectedByCulling`).
pub fn no_culling(width: f32, height: f32) -> bool {
    width == 0.0 || height == 0.0
}

/// `Display.updateCulling` / `getInterpolatedBoundingBox` at an interpolated
/// origin: the AABB from `(x − width/2, y, z − width/2)` to
/// `(x + width/2, y + height, z + width/2)`, normalized like `new AABB(...)`.
pub fn culling_box(origin: glam::DVec3, width: f32, height: f32) -> (glam::DVec3, glam::DVec3) {
    let half = f64::from(width / 2.0);
    let tall = f64::from(height);
    let (x0, x1) = (origin.x - half, origin.x + half);
    let (y0, y1) = (origin.y, origin.y + tall);
    let (z0, z1) = (origin.z - half, origin.z + half);
    (
        glam::DVec3::new(
            java_min_f64(x0, x1),
            java_min_f64(y0, y1),
            java_min_f64(z0, z1),
        ),
        glam::DVec3::new(
            java_max_f64(x0, x1),
            java_max_f64(y0, y1),
            java_max_f64(z0, z1),
        ),
    )
}

impl TextDisplayData {
    /// Omitted fields retain their previous values.
    pub fn apply(&mut self, values: &[(u8, i32, Value)]) {
        for (id, _, value) in values {
            match (id, value) {
                (8, Value::Int(v)) => self.interpolation_delay = *v,
                (9, Value::Int(v)) => self.interpolation_duration = *v,
                (10, Value::Int(v)) => self.teleport_duration = *v,
                (11, Value::Floats(v)) if v.len() == 3 => self.translation.copy_from_slice(v),
                (12, Value::Floats(v)) if v.len() == 3 => self.scale.copy_from_slice(v),
                (13, Value::Floats(v)) if v.len() == 4 => self.left_rotation.copy_from_slice(v),
                (14, Value::Floats(v)) if v.len() == 4 => self.right_rotation.copy_from_slice(v),
                (15, Value::Byte(v)) => self.billboard = *v as u8,
                (16, Value::Int(v)) => self.brightness = *v,
                (17, Value::Float(v)) => self.view_range = *v,
                (18, Value::Float(v)) => self.shadow_radius = *v,
                (19, Value::Float(v)) => self.shadow_strength = *v,
                (20, Value::Float(v)) => self.width = *v,
                (21, Value::Float(v)) => self.height = *v,
                (22, Value::Int(v)) => self.glow_color = *v,
                (23, Value::Text(v)) => self.text = Arc::new(crate::entity_nbt::name_text(v)),
                (24, Value::Int(v)) => self.line_width = *v,
                (25, Value::Int(v)) => self.background = *v,
                (26, Value::Byte(v)) => self.opacity = *v,
                (27, Value::Byte(v)) => self.flags = *v as u8,
                _ => {}
            }
        }
    }
    /// `TextDisplay.getAlign`: the left flag wins over the right one.
    pub fn align(&self) -> Align {
        if self.flags & 8 != 0 {
            Align::Left
        } else if self.flags & 16 != 0 {
            Align::Right
        } else {
            Align::Center
        }
    }
}

/// `TextDisplay.CachedInfo`: each line and its width, and the widest.
pub struct Lines {
    pub lines: Vec<(Text, f32)>,
    pub width: f32,
}

impl Lines {
    /// `TextDisplayRenderer.splitLines`: Font.split, no indent on wrapped lines.
    pub fn split(font: &crate::font::Font, text: &Text, line_width: i32) -> Self {
        let lines: Vec<_> = font
            .split_formatted(text, line_width as f32)
            .into_iter()
            .map(|line| {
                let width = font.width_text(&line).ceil();
                (line, width)
            })
            .collect();
        let width = lines.iter().map(|l| l.1).fold(0.0, f32::max);
        Self { lines, width }
    }
    /// Ten units a line, less the spacing under the last.
    pub fn height(&self) -> f32 {
        self.lines.len() as f32 * 10.0 - 1.0
    }
    /// Apply `TextDisplayRenderer.submitInner`'s pose steps in JOML order.
    pub fn apply_pose(&self, pose: &mut crate::camera::joml::Mat4) {
        pose.rotate_y(std::f32::consts::PI);
        pose.scale(-0.025, -0.025, -0.025);
        pose.translate(1.0 - self.width / 2.0, -self.height(), 0.0);
    }
    /// `submitInner`'s pose for the text, after the display's own.
    pub fn pose(&self) -> glam::Mat4 {
        let mut pose = crate::camera::joml::Mat4::IDENTITY;
        self.apply_pose(&mut pose);
        pose.to_glam()
    }
}

/// Local geometry, independent of the display's interpolated world pose.
/// Line wrapping (`lines`) depends only on text, line width, font epoch and bidi,
/// while quad paint (`batch`) refreshes in place when color/flags/atlas/skins change.
pub struct CachedQuads {
    layout_key: (Arc<Text>, i32, u64, bool),
    paint_key: (i32, i8, u8, u32, u64, u64),
    /// True when the text contains obfuscated (`§k`) glyphs that randomize every frame.
    obfuscated: bool,
    /// True when the text contains an inline object (head/sprite) sensitive to `object_skins_revision`.
    has_inline_object: bool,
    pub lines: Lines,
    pub batch: crate::hud::HudBatch,
    /// Prefix vertex count in `batch.vertices` belonging to `StandaloneBackground` (0 or 6).
    pub bg_vertices: usize,
    bounds: (glam::Vec3, glam::Vec3),
}

impl CachedQuads {
    fn layout_matches(&self, d: &TextDisplayData, epoch: u64) -> bool {
        self.layout_key
            == (
                d.text.clone(),
                d.line_width,
                epoch,
                crate::lang::bidirectional(),
            )
    }

    pub fn matches(&self, d: &TextDisplayData, background: u32, epoch: u64, revision: u64) -> bool {
        !self.obfuscated
            && self.layout_matches(d, epoch)
            && (
                self.paint_key.0,
                self.paint_key.1,
                self.paint_key.2,
                self.paint_key.3,
                self.paint_key.4,
            ) == (d.background, d.opacity, d.flags, background, revision)
    }

    pub fn matches_atlas(
        &self,
        atlas: &crate::hud::HudAtlas,
        d: &TextDisplayData,
        background: u32,
    ) -> bool {
        let skins = if self.has_inline_object {
            atlas.object_skins_revision
        } else {
            self.paint_key.5
        };
        !self.obfuscated
            && self.layout_matches(d, atlas.font.epoch())
            && self.paint_key
                == (
                    d.background,
                    d.opacity,
                    d.flags,
                    background,
                    atlas.revision,
                    skins,
                )
    }

    pub fn new(atlas: &crate::hud::HudAtlas, d: &TextDisplayData, background: u32) -> Self {
        let lines = Lines::split(&atlas.font, &d.text, d.line_width);
        let mut batch = crate::hud::HudBatch::scratch([1, 1]);
        let bg_vertices = draw_inner(&mut batch, atlas, d, &lines, background);
        let bounds = batch_bounds(&batch.vertices);
        let (obfuscated, has_inline_object) = scan_dynamic(&d.text);
        Self {
            layout_key: (
                d.text.clone(),
                d.line_width,
                atlas.font.epoch(),
                crate::lang::bidirectional(),
            ),
            paint_key: (
                d.background,
                d.opacity,
                d.flags,
                background,
                atlas.revision,
                atlas.object_skins_revision,
            ),
            obfuscated,
            has_inline_object,
            lines,
            batch,
            bg_vertices,
            bounds,
        }
    }

    /// Refresh only what changed: rewrap `lines` only on layout changes, and rebuild
    /// `batch` in place (reusing its vertex buffer) on paint, atlas, skin or obfuscation changes.
    pub fn update(&mut self, atlas: &crate::hud::HudAtlas, d: &TextDisplayData, background: u32) {
        let epoch = atlas.font.epoch();
        let layout_changed = !self.layout_matches(d, epoch);
        if layout_changed {
            self.lines = Lines::split(&atlas.font, &d.text, d.line_width);
            let (obfuscated, has_inline_object) = scan_dynamic(&d.text);
            self.obfuscated = obfuscated;
            self.has_inline_object = has_inline_object;
            self.layout_key = (
                d.text.clone(),
                d.line_width,
                epoch,
                crate::lang::bidirectional(),
            );
        }
        let skins = if self.has_inline_object {
            atlas.object_skins_revision
        } else {
            self.paint_key.5
        };
        let next_paint = (
            d.background,
            d.opacity,
            d.flags,
            background,
            atlas.revision,
            skins,
        );
        if layout_changed || self.obfuscated || self.paint_key != next_paint {
            self.batch.vertices.clear();
            self.batch.patches.clear();
            self.batch.inline_profiles.clear();
            self.bg_vertices = draw_inner(&mut self.batch, atlas, d, &self.lines, background);
            self.bounds = batch_bounds(&self.batch.vertices);
            self.paint_key = next_paint;
        }
    }

    pub fn transformed_bounds(&self, matrix: glam::Mat4) -> (glam::Vec3, glam::Vec3) {
        transformed_bounds(self.bounds, matrix)
    }
}

fn scan_dynamic(text: &Text) -> (bool, bool) {
    let resolved = text.resolved();
    let mut obfuscated = false;
    let mut has_inline_object = false;
    for span in &resolved.spans {
        if span.style.obfuscated == Some(true) {
            obfuscated = true;
        }
        if span.style.inline_object.is_some() {
            has_inline_object = true;
        }
    }
    (obfuscated, has_inline_object)
}

fn batch_bounds(vertices: &[crate::hud::HudVertex]) -> (glam::Vec3, glam::Vec3) {
    let mut lo = glam::Vec3::splat(f32::INFINITY);
    let mut hi = glam::Vec3::splat(f32::NEG_INFINITY);
    for v in vertices {
        let p = glam::Vec3::new(v.pos[0], v.pos[1], v.depth);
        lo = lo.min(p);
        hi = hi.max(p);
    }
    (lo, hi)
}

fn transformed_bounds(
    (lo, hi): (glam::Vec3, glam::Vec3),
    matrix: glam::Mat4,
) -> (glam::Vec3, glam::Vec3) {
    let mut min = glam::Vec3::splat(f32::INFINITY);
    let mut max = glam::Vec3::splat(f32::NEG_INFINITY);
    for x in [lo.x, hi.x] {
        for y in [lo.y, hi.y] {
            for z in [lo.z, hi.z] {
                let p = matrix.transform_point3(glam::Vec3::new(x, y, z));
                min = min.min(p);
                max = max.max(p);
            }
        }
    }
    (min, max)
}

/// The corners of each six-vertex `HudBatch` quad that `draw` fills, in the
/// order the world draws them: `Font`'s glyphs go top left, bottom left,
/// bottom right, top right, so under `Lines::pose` they wind
/// counter-clockwise on the side the text reads from, the face the culled
/// `TEXT_POLYGON_OFFSET` and `TEXT_SEE_THROUGH` keep.
pub const CORNERS: [usize; 4] = [5, 2, 1, 0];

/// `TextDisplayRenderer.submitInner` after its `translate(1 - width / 2,
/// -height, 0)`: the background and each line, in text units, each quad's
/// `depth` its z (the background 0.01 behind, `Font.prepareBackground`).
/// `default_background` is `Options.getBackgroundOpacity(0.25)` as a colour.
pub fn draw(
    batch: &mut crate::hud::HudBatch,
    atlas: &crate::hud::HudAtlas,
    d: &TextDisplayData,
    lines: &Lines,
    default_background: u32,
) {
    draw_inner(batch, atlas, d, lines, default_background);
}

fn draw_inner(
    batch: &mut crate::hud::HudBatch,
    atlas: &crate::hud::HudAtlas,
    d: &TextDisplayData,
    lines: &Lines,
    default_background: u32,
) -> usize {
    use crate::hud::rgba;
    let (width, height) = (lines.width, lines.height());
    let bg = if d.flags & 4 != 0 {
        default_background
    } else {
        d.background as u32
    };
    let mut bg_vertices = 0;
    if bg != 0 {
        let start = batch.vertices.len();
        batch.fill(
            atlas,
            // submitTextBackground(-1, -1, width, height): corners, not a size.
            -1.0,
            -1.0,
            width + 1.0,
            height + 1.0,
            rgba(
                (bg >> 16) as u8,
                (bg >> 8) as u8,
                bg as u8,
                (bg >> 24) as u8,
            ),
        );
        for v in &mut batch.vertices[start..] {
            v.depth = -0.01;
        }
        bg_vertices = batch.vertices.len() - start;
    }
    let align = d.align();
    for (i, (line, line_width)) in lines.lines.iter().enumerate() {
        let x = match align {
            Align::Left => 0.0,
            Align::Right => width - line_width,
            Align::Center => width / 2.0 - line_width / 2.0,
        };
        batch.text_styled_world(
            atlas,
            line,
            x,
            i as f32 * 10.0,
            rgba(255, 255, 255, d.opacity as u8),
            d.flags & 1 != 0,
        );
    }
    bg_vertices
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether a text quad seen from `camera` (yaw, pitch, at `eye` from the
    /// display) faces it (counter-clockwise on screen, the face back-face
    /// culling keeps) and whether its text reads left to right.
    fn seen(d: &TextDisplayData, entity: [f32; 2], camera: [f32; 2], eye: glam::Vec3) -> [bool; 2] {
        let lines = Lines {
            lines: vec![],
            width: 10.0,
        };
        let mut batch = crate::hud::HudBatch::scratch([1, 1]);
        batch.quad(
            [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            [[0.0; 2]; 4],
            u32::MAX,
        );
        let view_proj = glam::Mat4::perspective_rh(1.2, 1.0, 1000.0, 0.05)
            * crate::camera::view_matrix(camera[0], camera[1])
            * glam::Mat4::from_translation(-eye);
        let matrix = view_proj * d.matrix(entity, camera) * lines.pose();
        let [a, b, c, _] = CORNERS.map(|i| {
            let v = batch.vertices[i];
            matrix.project_point3(glam::Vec3::new(v.pos[0], v.pos[1], v.depth))
        });
        let ccw = (b - a).truncate().perp_dot((c - a).truncate()) > 0.0;
        // The text's x axis, from the left edge to the right, on screen.
        let right =
            matrix.project_point3(glam::Vec3::X).x > matrix.project_point3(glam::Vec3::ZERO).x;
        [ccw, right]
    }

    #[test]
    fn text_quads_face_the_side_they_read_from() {
        let mut d = TextDisplayData::default();
        // A fixed display at yaw 0 reads from the south: in front it faces
        // the camera, from the north it shows its back, which the text
        // pipelines cull.
        assert_eq!(
            seen(&d, [0.0, 0.0], [180.0, 0.0], glam::Vec3::Z * 3.0),
            [true, true]
        );
        assert_eq!(
            seen(&d, [0.0, 0.0], [0.0, 0.0], glam::Vec3::Z * -3.0),
            [false, false]
        );
        assert_eq!(
            seen(&d, [90.0, 0.0], [-90.0, 0.0], glam::Vec3::X * -3.0),
            [true, true]
        );
        // A mirrored scale turns the readable side round with the front.
        d.scale = [-1.0, 1.0, 1.0];
        assert_eq!(
            seen(&d, [0.0, 0.0], [0.0, 0.0], glam::Vec3::Z * -3.0),
            [true, true]
        );
        assert_eq!(
            seen(&d, [0.0, 0.0], [180.0, 0.0], glam::Vec3::Z * 3.0),
            [false, false]
        );
        // Billboards face the camera from anywhere it looks at them.
        d.scale = [1.0; 3];
        for billboard in 1..=3 {
            d.billboard = billboard;
            for yaw in [0.0, 37.0, 90.0, 180.0, -135.0] {
                let pitch = if billboard == 1 { 0.0 } else { 30.0 };
                let forward = glam::Vec3::new(
                    -f32::to_radians(yaw).sin() * f32::to_radians(pitch).cos(),
                    -f32::to_radians(pitch).sin(),
                    f32::to_radians(yaw).cos() * f32::to_radians(pitch).cos(),
                );
                let entity = if billboard == 2 {
                    [180.0 + yaw, 0.0]
                } else {
                    [0.0, 0.0]
                };
                assert_eq!(
                    seen(&d, entity, [yaw, pitch], forward * -3.0),
                    [true, true],
                    "billboard {billboard} at yaw {yaw}"
                );
            }
        }
    }
    #[test]
    fn partial_updates_retain_values_and_bad_vectors_do_not_panic() {
        let mut d = TextDisplayData::default();
        d.apply(&[(24, 1, Value::Int(80)), (27, 0, Value::Byte(24))]);
        d.apply(&[(11, 0, Value::Floats(vec![1.0]))]);
        assert_eq!(d.line_width, 80);
        assert_eq!(d.align(), Align::Left);
        assert_eq!(d.translation, [0.0; 3]);
    }

    /// `Display.updateCulling`: width or height 0 disables the frustum box.
    #[test]
    fn zero_width_or_height_disables_display_culling() {
        assert!(no_culling(0.0, 1.0));
        assert!(no_culling(2.0, 0.0));
        assert!(no_culling(0.0, 0.0));
        assert!(!no_culling(2.0, 1.0));
    }

    /// `Display.updateCulling`: the box is width-centred on XZ and rises by height.
    #[test]
    fn display_culling_box_is_centred_on_xz_and_stands_on_y() {
        let (lo, hi) = culling_box(glam::DVec3::new(10.0, 64.0, -3.0), 2.0, 1.0);
        assert_eq!(lo, glam::DVec3::new(9.0, 64.0, -4.0));
        assert_eq!(hi, glam::DVec3::new(11.0, 65.0, -2.0));
    }

    /// `Display$LinearFloatInterpolator`: the shadow radius and strength blend
    /// across the interpolation duration with the transformation's progress.
    #[test]
    fn the_blend_interpolates_shadow_radius_and_strength() {
        let from = TextDisplayData::default();
        let shown = TextDisplayData {
            shadow_radius: 2.0,
            shadow_strength: 0.25,
            ..TextDisplayData::default()
        };
        let half = shown.blend(&from, 0.5);
        assert_eq!((half.shadow_radius, half.shadow_strength), (1.0, 0.625));
        assert_eq!(shown.blend(&from, 0.0).shadow_radius, 0.0);
        assert_eq!(shown.blend(&from, 1.0).shadow_strength, 0.25);
    }

    fn display_parity_witness() -> serde_json::Value {
        let path = std::env::var("SUNRISE_DISPLAY_PARITY_WITNESS").expect(
            "SUNRISE_DISPLAY_PARITY_WITNESS must be set (run via tools/test_displays_parity.py)",
        );
        let text =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read {path}: {e}"));
        let witness: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(witness["meta"]["game_version"], "26.3");
        witness
    }

    fn hex_u32(v: &serde_json::Value) -> u32 {
        u32::from_str_radix(v.as_str().unwrap(), 16).unwrap()
    }

    fn hex_u64(v: &serde_json::Value) -> u64 {
        u64::from_str_radix(v.as_str().unwrap(), 16).unwrap()
    }

    fn hex_u32_vec(v: &serde_json::Value) -> Vec<u32> {
        v.as_array().unwrap().iter().map(hex_u32).collect()
    }

    fn hex_u64_vec(v: &serde_json::Value) -> Vec<u64> {
        v.as_array().unwrap().iter().map(hex_u64).collect()
    }

    #[test]
    #[ignore = "run by tools/test_displays_parity.py"]
    fn non_unit_quaternion_blend_and_matrix_match_vanilla_26_3_bits() {
        let witness = display_parity_witness();
        let case = &witness["non_unit_quat_and_blend"];
        let from = TextDisplayData {
            left_rotation: [0.0, 2.0, 0.0, 0.0],
            right_rotation: [1.0, 1.0, 1.0, 1.0],
            ..Default::default()
        };
        let to = TextDisplayData {
            translation: [1.0, 2.0, 3.0],
            left_rotation: [0.0, 0.0, 2.0, 0.0],
            right_rotation: [0.0, 1.0, 0.0, 1.0],
            ..Default::default()
        };
        let at_1 = to.blend(&from, 1.0);
        assert_eq!(
            at_1.left_rotation.map(f32::to_bits).to_vec(),
            hex_u32_vec(&case["at_1_left_rot_bits"])
        );
        assert_eq!(
            at_1.right_rotation.map(f32::to_bits).to_vec(),
            hex_u32_vec(&case["at_1_right_rot_bits"])
        );
        assert_eq!(
            at_1.matrix([0.0, 0.0], [0.0, 0.0])
                .to_cols_array()
                .map(f32::to_bits)
                .to_vec(),
            hex_u32_vec(&case["at_1_matrix_bits"])
        );
        let at_half = to.blend(&from, 0.5);
        assert_eq!(
            at_half.left_rotation.map(f32::to_bits).to_vec(),
            hex_u32_vec(&case["at_half_left_rot_bits"])
        );
        assert_eq!(
            at_half.right_rotation.map(f32::to_bits).to_vec(),
            hex_u32_vec(&case["at_half_right_rot_bits"])
        );
        assert_eq!(
            at_half
                .matrix([0.0, 0.0], [0.0, 0.0])
                .to_cols_array()
                .map(f32::to_bits)
                .to_vec(),
            hex_u32_vec(&case["at_half_matrix_bits"])
        );
    }

    #[test]
    #[ignore = "run by tools/test_displays_parity.py"]
    fn negative_width_or_height_culling_box_matches_vanilla_26_3_bits() {
        let witness = display_parity_witness();
        let case = &witness["live_culling_and_no_culling"];
        let origin = glam::DVec3::new(0.5, 301.62, 0.5);
        assert_eq!(
            !no_culling(2.0, 3.0),
            case["after_wh_affected_by_culling"].as_bool().unwrap()
        );
        let (lo, hi) = culling_box(origin, 2.0, 3.0);
        assert_eq!(
            vec![
                lo.x.to_bits(),
                lo.y.to_bits(),
                lo.z.to_bits(),
                hi.x.to_bits(),
                hi.y.to_bits(),
                hi.z.to_bits(),
            ],
            hex_u64_vec(&case["wh_culling_box_bits"])
        );
        assert_eq!(
            !no_culling(-2.0, -3.0),
            case["neg_wh_affected_by_culling"].as_bool().unwrap()
        );
        let (lo, hi) = culling_box(origin, -2.0, -3.0);
        assert_eq!(
            vec![
                lo.x.to_bits(),
                lo.y.to_bits(),
                lo.z.to_bits(),
                hi.x.to_bits(),
                hi.y.to_bits(),
                hi.z.to_bits(),
            ],
            hex_u64_vec(&case["neg_wh_culling_box_bits"])
        );
    }

    #[test]
    #[ignore = "run by tools/test_displays_parity.py"]
    fn billboard_matrix_matches_vanilla_26_3_bits() {
        let witness = display_parity_witness();
        let cases = witness["billboard_camera_angles"]["cases"]
            .as_array()
            .unwrap();
        assert_eq!(cases.len(), 16);
        for case in cases {
            let d = TextDisplayData {
                billboard: case["billboard"].as_u64().unwrap() as u8,
                translation: [-1.395, 0.3885, -4.2],
                scale: [0.525, 0.525, 0.525],
                left_rotation: [0.0, 1.0, 0.0, 0.0],
                ..Default::default()
            };
            let entity = [
                f32::from_bits(hex_u32(&case["entity_yRot_bits"])),
                f32::from_bits(hex_u32(&case["entity_xRot_bits"])),
            ];
            let camera = [
                f32::from_bits(hex_u32(&case["camera_yRot_bits"])),
                f32::from_bits(hex_u32(&case["camera_xRot_bits"])),
            ];
            let got = d
                .matrix(entity, camera)
                .to_cols_array()
                .map(f32::to_bits)
                .to_vec();
            assert_eq!(
                got,
                hex_u32_vec(&case["pose_bits"]),
                "case {} billboard {}",
                case["name"],
                case["billboard"]
            );
        }
    }

    #[test]
    fn blend_at_one_preserves_exact_large_translation_and_culling_halves_in_f32() {
        let from = TextDisplayData {
            translation: [33_554_432.0, 0.0, 0.0],
            ..Default::default()
        };
        let to = TextDisplayData {
            translation: [1.0, 0.0, 0.0],
            ..Default::default()
        };
        assert_eq!(to.blend(&from, 1.0).translation[0], 1.0);

        let (lo, hi) = culling_box(glam::DVec3::ZERO, f32::from_bits(1), 1.0);
        assert_eq!((lo.x, hi.x, lo.z, hi.z), (0.0, 0.0, 0.0, 0.0));
        let (ilo, ihi) = culling_box(glam::DVec3::ZERO, f32::INFINITY, 1.0);
        assert_eq!((ilo.x, ihi.x), (f64::NEG_INFINITY, f64::INFINITY));
    }

    #[test]
    fn joml_pose_keeps_exact_affine_rounding_for_large_translation_and_rotation() {
        let angle = (-45.0_f32).to_radians();
        let d = TextDisplayData {
            translation: [16_777_216.0, 0.0, 0.0],
            left_rotation: [0.0, 0.0, (angle * 0.5).sin(), (angle * 0.5).cos()],
            ..Default::default()
        };
        let pose = d.pose([0.0; 3], [0.0; 2], [0.0; 2]);
        let pt = pose.transform_position([1.0, 1.0, 0.0]);
        assert_eq!(pt[0].to_bits(), 16_777_216.0_f32.to_bits());
    }

    #[test]
    fn cached_quads_reuses_layout_on_paint_change_and_static_inline_head() {
        let atlas = crate::hud::HudAtlas::boot();
        let mut d = TextDisplayData {
            text: std::sync::Arc::new(crate::text::Text::literal("Hello world")),
            background: 0x4000_0000,
            opacity: -1,
            ..Default::default()
        };
        let mut cache = CachedQuads::new(&atlas, &d, 0);
        let lines_ptr = cache.lines.lines.as_ptr();
        let lines_len = cache.lines.lines.len();

        // Changing opacity/background repaints without rebuilding line layout.
        d.background = 0x8011_2233u32.cast_signed();
        d.opacity = 128u8 as i8;
        assert!(!cache.matches(&d, 0, atlas.font.epoch(), atlas.revision));
        cache.update(&atlas, &d, 0);
        assert_eq!(cache.lines.lines.as_ptr(), lines_ptr);
        assert_eq!(cache.lines.lines.len(), lines_len);
        assert!(cache.matches(&d, 0, atlas.font.epoch(), atlas.revision));

        // Static inline player head stays cached across frames while atlas & skin revisions match.
        let head = crate::entity_nbt::name_text(&sunrise_core::nbt::Tag::Compound(
            [
                (
                    "object".into(),
                    sunrise_core::nbt::Tag::String("player".into()),
                ),
                (
                    "player".into(),
                    sunrise_core::nbt::Tag::String("Alex".into()),
                ),
            ]
            .into_iter()
            .collect(),
        ));
        d.text = std::sync::Arc::new(head);
        cache.update(&atlas, &d, 0);
        assert!(!cache.obfuscated);
        assert!(cache.has_inline_object);
        assert!(cache.matches_atlas(&atlas, &d, 0));
        assert!(cache.matches(&d, 0, atlas.font.epoch(), atlas.revision));
        assert!(!cache.matches(&d, 0, atlas.font.epoch(), atlas.revision + 1));
    }
}
