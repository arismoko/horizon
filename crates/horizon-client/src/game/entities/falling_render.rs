//! Falling block model caching and piston landing completion.
use super::*;
impl Game<'_> {
    /// Cache only model-dependent work, never an entity's seed, light or pose.
    pub(super) fn falling_model_vertices(
        &self,
        state: BlockStateId,
        start_pos: BlockPos,
        origin: glam::Vec3,
        light: u32,
        out: &mut Vec<Vertex>,
    ) {
        // `FallingBlockRenderer`: the model is picked by the block's starting position.
        let seed = horizon_core::random::position_seed(start_pos);
        let Ok(models) = self.pack.render.model_catalog.collect_seeded(state, seed) else {
            return;
        };
        // Baked models live at stable addresses until the pack is replaced.
        // Keep multipart order and duplicates: both affect the emitted vertices.
        let mut cache = self.falling_vertices.borrow_mut();
        let entries = cache.entry(state).or_default();
        let index = entries.iter().position(|(ptrs, _)| {
            ptrs.iter()
                .copied()
                .eq(models.iter().map(|(_, m)| *m as *const _ as usize))
        });
        let index = index.unwrap_or_else(|| {
            let mut vertices = Vec::new();
            self.block_model_vertices(state, seed, glam::Mat4::IDENTITY, 0, false, &mut vertices);
            // Preserve the original coordinates, including signed zero, rather
            // than composing an identity transform with the frame's translation.
            let positions = models.iter().flat_map(|(_, model)| {
                model
                    .faces
                    .iter()
                    .filter(|face| {
                        self.pack
                            .layer_names
                            .get(&face.layer)
                            .is_some_and(|name| self.gui.atlas.sprites.contains_key(name))
                    })
                    .flat_map(|face| face.quad.positions)
            });
            for (vertex, pos) in vertices.iter_mut().zip(positions) {
                vertex.pos = pos;
            }
            let ptrs = models
                .iter()
                .map(|(_, m)| *m as *const _ as usize)
                .collect();
            entries.push((ptrs, vertices));
            entries.len() - 1
        });
        let start = out.len();
        out.extend_from_slice(&entries[index].1);
        let matrix = glam::Mat4::from_translation(origin);
        for vertex in &mut out[start..] {
            vertex.pos = matrix
                .transform_point3(glam::Vec3::from_array(vertex.pos))
                .to_array();
            vertex.light = light;
        }
    }
    /// The end of a piston's `Entity.move(MoverType.PISTON, …)` for falling
    /// block `i` after a step of `moved`: `checkFallDamage` adds the drop and,
    /// on the ground, runs fallOn and posts HIT_GROUND after a fall, then
    /// forgets it, so its own next tick does not land again.
    pub(super) fn finish_falling_piston_move(&mut self, i: usize, moved: Vec3) {
        let mut f = self.falling[i].clone();
        if moved.y < 0.0 {
            f.fall_distance -= f64::from(moved.y as f32);
        }
        if f.on_ground {
            self.falling_block_impact(&mut f);
            if f.fall_distance > 0.0 {
                let bounds = horizon_client::falling_block::bounds(f.position);
                self.post_landing_event(f.position, bounds, false);
            }
            f.fall_distance = 0.0;
        }
        self.falling.update(i, |block| *block = f);
    }
}

#[cfg(test)]
mod geometry_tests {
    use super::*;

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn falling_model_cache_matches_direct_vertices() {
        // Geometry belongs to the client resource graph; server packs have no models.
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            for frame in 0..2 {
                for (name, pos, origin, light) in [
                    (
                        "minecraft:sand",
                        BlockPos::new(1, 80, 3),
                        glam::Vec3::new(-3.25, 12.5, 0.0),
                        0x7010,
                    ),
                    (
                        "minecraft:sand",
                        BlockPos::new(9, 90, -4),
                        glam::Vec3::new(8.125, -0.0, -9.5),
                        0xf020,
                    ),
                    (
                        "minecraft:gravel",
                        BlockPos::new(-2, 70, 5),
                        glam::Vec3::new(-0.0, -17.75, 2.5),
                        0x5080,
                    ),
                ] {
                    let state = game.pack.assets.registry.default_state(name).unwrap();
                    let seed = horizon_core::random::position_seed(pos);
                    let mut expected = Vec::new();
                    game.block_model_vertices(
                        state,
                        seed,
                        glam::Mat4::from_translation(origin),
                        light,
                        false,
                        &mut expected,
                    );
                    let mut actual = Vec::new();
                    game.falling_model_vertices(state, pos, origin, light, &mut actual);
                    let (_, tint) = game.falling_block_face_sprite(state).unwrap();
                    let tint = horizon_client::mobs::render::abgr(tint);
                    for vertices in [&mut expected, &mut actual] {
                        for v in vertices {
                            v.color = horizon_client::hud_layout::multiply(v.color, tint);
                        }
                    }
                    assert!(!expected.is_empty());
                    assert_eq!(actual.len(), expected.len());
                    for (a, b) in actual.iter().zip(&expected) {
                        assert_eq!(
                            a.pos.map(f32::to_bits),
                            b.pos.map(f32::to_bits),
                            "frame {frame}"
                        );
                        assert_eq!(a.uv.map(f32::to_bits), b.uv.map(f32::to_bits));
                        assert_eq!(a.glint_uv.map(f32::to_bits), b.glint_uv.map(f32::to_bits));
                        assert_eq!(
                            (a.color, a.light, a.normal, a.layer),
                            (b.color, b.light, b.normal, b.layer)
                        );
                    }
                }
            }
        });
    }
}
