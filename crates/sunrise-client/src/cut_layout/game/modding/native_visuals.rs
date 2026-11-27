#![cfg(test)]
#[cfg(test)]
mod tests {
    use sunrise_client_gameplay::game::client_side::native_visuals::rim_target_valid;
    use sunrise_world::registry::BlockStateId;

    /// A rim and a light a mod builds with their constructors and pushes from `visuals` reach
    /// the frame: the rim as outline edges in its colour and width, the light in its colour.
    #[test]
    #[ignore = "daily"]
    fn a_mod_built_rim_and_light_reach_the_frame() {
        use mod_api::{BlockRim, ClientHost, ColoredLight, Hooks, ModEntry, VisualsEvent};
        use mod_platform::testing::with_mods;

        struct Mod;
        #[mod_api::__subscriptions]
        impl Hooks for Mod {
            fn visuals(&self, _: &dyn ClientHost, event: &mut VisualsEvent<'_>) {
                event.rims.push(BlockRim::new([1, 64, 1], 0xFF00_FF00, 2.0));
                let light = ColoredLight::new([1.5, 65.5, 1.5], 0xFF0000, 4.0, 0.5);
                event.lights.push(light);
            }
        }
        let mods: mod_api::Mods = vec![ModEntry::new("visual", Box::new(Mod))].into();
        with_mods(mods, || {
            crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                let mut chunk = sunrise_level::chunk::Chunk::empty(0, 0);
                chunk.lit = true;
                game.world.chunks.insert((0, 0), std::sync::Arc::new(chunk));
                game.world.set(1, 64, 1, stone).unwrap();
                let mut player = sunrise_sim::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::ZERO,
                )
                .unwrap();
                player.position = sunrise_core::geometry::Vec3::new(0.5, 65.0, 0.5);
                game.player = Some(player);

                let mut lines = Vec::new();
                let camera = glam::DVec3::new(0.5, 66.0, 0.5);
                let lights = game.native_frame_visuals(&mut lines).finish(camera);
                assert_eq!(lines.len(), 12, "a full block's twelve edges");
                assert!(
                    lines
                        .iter()
                        .all(|l| l.color == 0xFF00_FF00 && l.width == 2.0)
                );
                assert_eq!(lights, [[[1.0, -0.5, 1.0, 4.0], [1.0, 0.0, 0.0, 0.5]]]);
            });
        });
    }

    #[test]
    fn ordinary_loaded_blocks_can_have_rims() {
        let air = BlockStateId(0);
        let stone = BlockStateId(1); // No emission requirement.
        assert!(rim_target_valid(true, stone, air));
        assert!(!rim_target_valid(false, stone, air));
        assert!(!rim_target_valid(true, air, air));
    }
}
