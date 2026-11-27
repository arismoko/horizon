use super::{
    Hit,
    item_draw::{BlockItemDraw, block_entity_pose},
    view_vector,
};
use sunrise_client::{block_entity_render::vertex_light, camera::rotate_degrees, mesher::Vertex};
use sunrise_client_render::block_entity_reach::Reach;
use sunrise_core::random::RandomSource;
use sunrise_level::containers::BlockEntity;
use sunrise_level::particle_options::{ParticleOptions, ParticleType};

impl crate::game::ClientGame<'_> {
    /// Level event 3008 (`LevelEventHandler`): the finished block's sound
    /// and break particles.
    pub(in crate::game) fn brush_complete_event(&mut self, pos: [i32; 3], data: i32) {
        let state = sunrise_world::registry::BlockStateId(data as u32);
        let sound = match &*self.common.res.facts().get(state).name {
            "minecraft:suspicious_sand" => "minecraft:item.brush.brushing.sand.complete",
            "minecraft:suspicious_gravel" => "minecraft:item.brush.brushing.gravel.complete",
            _ => "",
        };
        if !sound.is_empty() {
            let at = pos.map(|v| f64::from(v) + 0.5);
            // Each client plays it from the event itself.
            self.play_audio_unshared(sound, Some(at), 1.0, 1.0);
        }
        self.destroy_particles(pos, state);
    }
    /// `BrushItem.spawnDustParticles`.
    pub(in crate::game::client_side) fn brush_dust(&mut self, hit: &Hit, off_hand: bool) {
        let state = self.common.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        let Some((sprite, tint)) = self.particle_sprite(state, hit.pos) else {
            return;
        };
        let options = ParticleOptions::block(ParticleType::Block, sprite, tint);
        // The arm that brushes: the main arm, or the other one for the off hand.
        let flip = if off_hand == self.client.settings.main_hand_left {
            1.0
        } else {
            -1.0
        };
        let view = view_vector(self.common.yaw, self.common.pitch);
        // DustParticlesDelta.fromDirection.
        let (dx, dz) = match hit.face {
            [0, _, 0] => (view[2], -view[0]),
            [0, 0, -1] => (1.0, -0.1),
            [0, 0, 1] => (-1.0, 0.1),
            [-1, 0, 0] => (-0.1, -1.0),
            _ => (0.1, 1.0),
        };
        let count = 7 + self
            .client
            .particles
            .0
            .level_random()
            .next_int_bound(5)
            .unwrap_or(0);
        let x = hit.point[0] - if hit.face == [-1, 0, 0] { 1.0e-6 } else { 0.0 };
        let z = hit.point[2] - if hit.face == [0, 0, -1] { 1.0e-6 } else { 0.0 };
        for _ in 0..count {
            let random = self.client.particles.0.level_random();
            let v = [
                dx * flip * 3.0 * random.next_double(),
                0.0,
                dz * flip * 3.0 * random.next_double(),
            ];
            self.client
                .particles
                .0
                .add(&options, [x, hit.point[1], z], v);
        }
    }
    /// `BrushableBlockRenderer`: reveal the item from the first brushed face.
    pub fn brushable_geometry(&self, camera: glam::DVec3, out: &mut Vec<Vertex>) {
        self.brushable_items(camera, |d| {
            self.submit_item_world(d.state, d.pose, d.light, out);
        });
    }
    /// The items [`Self::brushable_geometry`] draws.
    pub fn brushable_items(&self, camera: glam::DVec3, mut draw: impl FnMut(BlockItemDraw)) {
        use sunrise_assets::item_model::DisplayContext;
        let _items = sunrise_level::spans::raw_flight_guard("block_entity_items");
        let reach = Reach::Default;
        sunrise_level::spans::note_block_entity_item_visits(|| {
            self.common.world.block_entities.len() as u64
        });
        for (key, entity) in &self.common.world.block_entities {
            let BlockEntity::Brushable(b) = entity else {
                continue;
            };
            let (Some(stack), Some(face)) = (&b.item, b.hit_direction) else {
                continue;
            };
            if !reach.should_render([key.0, key.1, key.2], camera, 0) {
                continue;
            }
            let state = self.common.world.block(key.0, key.1, key.2);
            let progress = self
                .common
                .pack
                .assets
                .registry
                .state(state)
                .and_then(|s| s.properties.get("dusted"))
                .and_then(|s| s.parse::<u8>().ok())
                .unwrap_or(0);
            if progress == 0 {
                continue;
            }
            // `BrushableBlockRenderer.translations`.
            let offset = f32::from(progress) / 10.0 * 0.75;
            let (translation, adjacent, east_west) = match face {
                0 => ([0.5, -0.23 - offset, 0.5], [0, -1, 0], false),
                1 => ([0.5, 0.25 + offset, 0.5], [0, 1, 0], false),
                2 => ([0.5, 0.0, 0.25 - offset], [0, 0, -1], false),
                3 => ([0.5, 0.0, 0.73 + offset], [0, 0, 1], false),
                4 => ([0.25 - offset, 0.0, 0.5], [-1, 0, 0], true),
                5 => ([0.73 + offset, 0.0, 0.5], [1, 0, 0], true),
                _ => continue,
            };
            let item = self.resolve_item(stack, DisplayContext::Fixed, None, 0);
            if item.is_empty() {
                continue;
            }
            let mut pose = block_entity_pose(*key, camera);
            pose.translate(0.0, 0.5, 0.0);
            pose.translate(translation[0], translation[1], translation[2]);
            rotate_degrees(&mut pose, 1, 75.0);
            rotate_degrees(&mut pose, 1, if east_west { 101.0 } else { 11.0 });
            pose.scale(0.5, 0.5, 0.5);
            let light = self.common.world.light(
                key.0 + adjacent[0],
                key.1 + adjacent[1],
                key.2 + adjacent[2],
            );
            draw(BlockItemDraw {
                #[cfg(any(test, feature = "test-support"))]
                stack,
                #[cfg(any(test, feature = "test-support"))]
                display: DisplayContext::Fixed,
                #[cfg(any(test, feature = "test-support"))]
                seed: 0,
                state: &item,
                pose: pose.to_glam(),
                light: vertex_light(light),
            });
        }
    }
}
