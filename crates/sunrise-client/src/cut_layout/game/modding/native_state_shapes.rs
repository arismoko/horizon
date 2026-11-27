#![cfg(test)]
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

#[cfg(test)]
pub(crate) mod tests {
    use std::ops::ControlFlow;
    use sunrise_gameplay_common::game::api_values::TryIntoApi;
    use {
        crate::game::native_state_shapes::*,
        sunrise_gameplay_common::game::native_state_shapes::BlockShapeKind,
    };

    /// `BlockState.getVisualShape`: nothing for glass, iron bars and powder snow, a full
    /// block for soul sand and mud, the outline for a snow layer and a fence, the collision
    /// shape for any other block.
    #[test]
    #[ignore = "daily"]
    fn visual_shape_follows_vanilla_overrides() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let level = game.level("minecraft:overworld").unwrap();
            for (block, expected) in [
                ("minecraft:glass", Vec::new()),
                ("minecraft:iron_bars", Vec::new()),
                ("minecraft:powder_snow", Vec::new()),
                ("minecraft:soul_sand", vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]]),
                ("minecraft:mud", vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]]),
                ("minecraft:snow", vec![[0.0, 0.0, 0.0, 1.0, 0.125, 1.0]]),
                (
                    "minecraft:oak_fence",
                    vec![[0.375, 0.0, 0.375, 0.625, 1.0, 0.625]],
                ),
                ("minecraft:stone_slab", vec![[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]]),
            ] {
                let state = game.pack.assets.registry.default_state(block).unwrap();
                let mut boxes = Vec::new();
                assert_eq!(
                    level.visit_block_shape(
                        game,
                        state.try_into_api().unwrap(),
                        [0, 64, 0],
                        BlockShapeKind::Visual,
                        &mut |bounds| {
                            boxes.push(bounds);
                            ControlFlow::Continue(())
                        }
                    ),
                    Some(ControlFlow::Continue(())),
                    "{block}"
                );
                assert_eq!(boxes, expected, "{block}");
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn supplied_shape_state_does_not_replace_world_state() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let level = game.level("minecraft:overworld").unwrap();
            let pos = [0, 64, 0];
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            let slab = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone_slab")
                .unwrap();
            let before = game.world.block(pos[0], pos[1], pos[2]);
            assert_ne!(before, stone);
            for kind in [BlockShapeKind::Outline, BlockShapeKind::Collision] {
                for (state, expected) in [
                    (stone, vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]]),
                    (slab, vec![[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]]),
                    (before, Vec::new()),
                ] {
                    let mut boxes = Vec::new();
                    assert_eq!(
                        level.visit_block_shape(
                            game,
                            state.try_into_api().unwrap(),
                            pos,
                            kind,
                            &mut |bounds| {
                                boxes.push(bounds);
                                ControlFlow::Continue(())
                            }
                        ),
                        Some(ControlFlow::Continue(()))
                    );
                    assert_eq!(boxes, expected);
                }
            }
            assert_eq!(game.world.block(pos[0], pos[1], pos[2]), before);
        });
    }
}
