use glam::{DVec3, Mat4, Vec4};

/// The most world views ([`crate::render_slots::world_views`]) one frame draws.
pub const MAX_WORLD_VIEWS: usize = 4;
/// The largest side of a world view's target, in pixels.
pub const MAX_WORLD_VIEW_SIDE: u32 = 4096;

/// A view of the world a mod asks for ([`crate::render_slots::world_views`]): the same world, drawn again from a
/// camera of its own into a target of its own (a camera monitor, a second camera's preview, a
/// still for a film strip).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct WorldView {
    /// The mod's name for the view: [`crate::render_slots::world_view_drawn`] hands its picture back with it, and
    /// the target stays the view's while each frame asks for it again.
    pub id: u64,
    /// Where the camera is in the world.
    pub camera: DVec3,
    /// Degrees, as the player's camera turns: yaw, pitch, and a roll about the view direction
    /// (positive turns the picture counterclockwise).
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    /// The vertical field of view, in degrees (1 to 170).
    pub fov_degrees: f32,
    /// The target's width and height, in pixels (1 to [`MAX_WORLD_VIEW_SIDE`]).
    pub size: [u32; 2],
    /// The plane behind which nothing is drawn ([`WorldView::with_clip_plane`]).
    pub clip_plane: Option<[f64; 4]>,
    /// The picture is mirrored left to right ([`WorldView::with_mirror`]).
    pub mirror: bool,
}

impl WorldView {
    pub fn new(
        id: u64,
        camera: DVec3,
        yaw: f32,
        pitch: f32,
        fov_degrees: f32,
        size: [u32; 2],
    ) -> Self {
        Self {
            id,
            camera,
            yaw,
            pitch,
            roll: 0.0,
            fov_degrees,
            size,
            clip_plane: None,
            mirror: false,
        }
    }

    pub fn with_roll(self, roll: f32) -> Self {
        Self { roll, ..self }
    }

    /// Draws only what lies in front of a world-space plane `[x, y, z, d]`: the points `p` with
    /// `x * p.x + y * p.y + z * p.z + d >= 0`. Terrain, entities and block entities behind it are
    /// left out, as a wall behind a portal's far side is. The camera must be behind the plane
    /// (where `x * camera.x + y * camera.y + z * camera.z + d < 0`): the plane becomes the
    /// view's near plane, which every backend draws. A view whose camera is not behind it, or
    /// whose plane has a normal `[x, y, z]` of zero, a value that is not finite, or a scaled form
    /// or distance from the camera that is not (as f32 too), is out of range and left out.
    pub fn with_clip_plane(self, plane: [f64; 4]) -> Self {
        Self {
            clip_plane: Some(plane),
            ..self
        }
    }

    /// The camera's picture mirrored left to right, as a mirror shows it: give the camera
    /// reflected through the mirror. A mirrored picture turns every triangle's winding round,
    /// so the view draws with face culling flipped to make up for it: the faces culled are the
    /// ones an unmirrored view culls, and back faces stay hidden.
    pub fn with_mirror(self) -> Self {
        Self {
            mirror: true,
            ..self
        }
    }

    /// Whether it can be drawn: every value finite and in range, no side over `max_side`, the
    /// camera behind its clip plane.
    fn valid(&self, max_side: u32) -> bool {
        self.camera.is_finite()
            && [self.yaw, self.pitch, self.roll]
                .iter()
                .all(|v| v.is_finite())
            && (1.0..=170.0).contains(&self.fov_degrees)
            && self
                .size
                .iter()
                .all(|&s| (1..=max_side.min(MAX_WORLD_VIEW_SIDE)).contains(&s))
            && (self.clip_plane.is_none() || self.camera_clip_plane().is_some())
    }

    /// The clip plane relative to the camera, as the projection takes it: a unit normal and the
    /// camera's distance in front of the plane (below 0, behind it), worked out in f64 so far
    /// from the origin it loses nothing. `None` without one, or for one out of range: a normal
    /// of zero, a value or its scaled or camera-relative form not finite (in f32 too), or the
    /// camera not behind it.
    pub(crate) fn camera_clip_plane(&self) -> Option<Vec4> {
        let [x, y, z, d] = self.clip_plane?;
        let length = DVec3::new(x, y, z).length();
        let normal = DVec3::new(x, y, z) / length;
        let plane = normal
            .as_vec3()
            .extend((normal.dot(self.camera) + d / length) as f32);
        (length > 0.0 && length.is_finite() && plane.is_finite() && plane.w < 0.0).then_some(plane)
    }
}

/// A world view just drawn ([`crate::render_slots::world_view_drawn`]): its picture and depth, and what a mod needs
/// to draw into it or copy it out. Drawn after the frame's world and before [`crate::render_slots::world_drawn`], so
/// a mod can show it in the frame's picture there.
pub struct WorldViewDrawn<'a> {
    /// The [`WorldView::id`] it was asked with.
    pub id: u64,
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub encoder: &'a mut wgpu::CommandEncoder,
    /// The view's picture (the world's light in `color_format`, as [`crate::render_slots::WorldDrawn::color`] holds
    /// it, before the tone curve and without the hand or the HUD), and a view of it. The target
    /// is the view's while it is asked for each frame and is drawn over the next frame: copy it
    /// (it can be copied and sampled) to keep a still.
    pub color: &'a wgpu::Texture,
    pub color_view: &'a wgpu::TextureView,
    pub color_format: wgpu::TextureFormat,
    pub size: [u32; 2],
    /// Its depth (Depth32Float, reversed: 0 is the sky), sampleable.
    pub depth: &'a wgpu::TextureView,
    /// World relative to [`WorldViewDrawn::camera`] to clip space, as the picture and `depth`
    /// were drawn: its near plane is the view's clip plane when it has one, and its x is
    /// flipped when the view is mirrored.
    pub view_proj: Mat4,
    /// The view's camera.
    pub camera: DVec3,
}

/// The views of those asked for ([`crate::render_slots::world_views`]) that can be drawn on a device whose textures
/// are at most `max_side` pixels a side (`max_texture_dimension_2d`; never above
/// [`MAX_WORLD_VIEW_SIDE`]), kept in `views` in place, without allocating.
pub fn drawable_world_views(views: &mut Vec<WorldView>, max_side: u32) {
    let mut ids = [0; MAX_WORLD_VIEWS];
    let mut kept = 0;
    views.retain(|v| {
        kept < MAX_WORLD_VIEWS && v.valid(max_side) && !ids[..kept].contains(&v.id) && {
            ids[kept] = v.id;
            kept += 1;
            true
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_drawable_views_draw() {
        let view =
            |id, fov, size| WorldView::new(id, DVec3::new(1.0, 70.0, -3.0), 30.0, 10.0, fov, size);
        let mut asked = vec![
            view(1, 70.0, [320, 180]),
            view(1, 70.0, [64, 64]),
            view(2, 0.5, [64, 64]),
            view(3, 70.0, [0, 64]),
            view(4, 70.0, [MAX_WORLD_VIEW_SIDE + 1, 64]),
            view(5, 70.0, [64, 64]).with_roll(f32::NAN),
            WorldView::new(
                6,
                DVec3::new(f64::INFINITY, 0.0, 0.0),
                0.0,
                0.0,
                70.0,
                [64, 64],
            ),
            // clip planes: a zero normal, values not finite, the camera (at z -3) in front of
            // the plane or on it
            view(30, 70.0, [64, 64]).with_clip_plane([0.0, 0.0, 0.0, 1.0]),
            view(31, 70.0, [64, 64]).with_clip_plane([0.0, f64::NAN, 1.0, 0.0]),
            view(32, 70.0, [64, 64]).with_clip_plane([0.0, 0.0, 1.0, f64::INFINITY]),
            view(33, 70.0, [64, 64]).with_clip_plane([0.0, 0.0, -1.0, 0.0]),
            view(34, 70.0, [64, 64]).with_clip_plane([0.0, 0.0, 2.0, 6.0]),
            // finite values whose scaled or camera-relative form is not (camera at 0)
            WorldView::new(35, DVec3::ZERO, 0.0, 0.0, 70.0, [64, 64])
                .with_clip_plane([0.0, 0.0, 1.0, -1e40]),
            WorldView::new(36, DVec3::ZERO, 0.0, 0.0, 70.0, [64, 64])
                .with_clip_plane([1e-160, 0.0, 0.0, -1e308]),
            // the camera behind the plane z = 0, mirrored too
            view(8, 70.0, [64, 64])
                .with_clip_plane([0.0, 0.0, 0.5, 0.0])
                .with_mirror(),
        ];
        asked.extend((10..20).map(|id| view(id, 170.0, [1, MAX_WORLD_VIEW_SIDE])));
        drawable_world_views(&mut asked, MAX_WORLD_VIEW_SIDE);
        let drawn: Vec<u64> = asked.iter().map(|v| v.id).collect();
        // the first id 1 only; the out-of-range ones left out; at most four in all
        assert_eq!(drawn, [1, 8, 10, 11]);
        // A device with smaller textures (downlevel, GL: 2048) leaves out what it cannot hold.
        let mut small = vec![view(20, 70.0, [2049, 64]), view(21, 70.0, [2048, 2048])];
        drawable_world_views(&mut small, 2048);
        let drawn: Vec<u64> = small.iter().map(|v| v.id).collect();
        assert_eq!(drawn, [21]);
    }
}
