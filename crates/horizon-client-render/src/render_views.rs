use glam::{DVec3, Mat4};

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
        }
    }

    pub fn with_roll(self, roll: f32) -> Self {
        Self { roll, ..self }
    }

    /// Whether it can be drawn: every value finite and in range, no side over `max_side`.
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
    /// World relative to [`WorldViewDrawn::camera`] to clip space.
    pub view_proj: Mat4,
    /// The view's camera.
    pub camera: DVec3,
}

/// The views of those asked for ([`crate::render_slots::world_views`]) that can be drawn on a device whose textures
/// are at most `max_side` pixels a side (`max_texture_dimension_2d`; never above
/// [`MAX_WORLD_VIEW_SIDE`]).
pub fn drawable_world_views(mut views: Vec<WorldView>, max_side: u32) -> Vec<WorldView> {
    let mut ids = Vec::new();
    views.retain(|v| {
        v.valid(max_side) && !ids.contains(&v.id) && {
            ids.push(v.id);
            true
        }
    });
    views.truncate(MAX_WORLD_VIEWS);
    views
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
        ];
        asked.extend((10..20).map(|id| view(id, 170.0, [1, MAX_WORLD_VIEW_SIDE])));
        let drawn: Vec<u64> = drawable_world_views(asked, MAX_WORLD_VIEW_SIDE)
            .iter()
            .map(|v| v.id)
            .collect();
        // the first id 1 only; the out-of-range ones left out; at most four in all
        assert_eq!(drawn, [1, 10, 11, 12]);
        // A device with smaller textures (downlevel, GL: 2048) leaves out what it cannot hold.
        let small = vec![view(20, 70.0, [2049, 64]), view(21, 70.0, [2048, 2048])];
        let drawn: Vec<u64> = drawable_world_views(small, 2048)
            .iter()
            .map(|v| v.id)
            .collect();
        assert_eq!(drawn, [21]);
    }
}
