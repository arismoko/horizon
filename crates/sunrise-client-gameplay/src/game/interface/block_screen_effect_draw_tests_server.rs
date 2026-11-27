use std::sync::Arc;
use sunrise_level::chunk::Chunk;
use sunrise_sim::motion::{MotionWorld, WATER};

/// Sample the player's fluid state as a tick would, without advancing their fire timer.
pub(in crate::game) fn sample_eye_in_water(game: &mut crate::game::ClientGame<'_>) -> bool {
    // The fluid scan requires the chunks around the interaction box to be loaded too.
    let layout = game.common.world.layout();
    for (x, z) in [(-1, -1), (-1, 0), (0, -1)] {
        if !game.common.world.chunks.contains_key(&(x, z)) {
            game.common
                .world
                .chunks
                .insert((x, z), Arc::new(Chunk::with_layout(x, z, layout)));
        }
    }
    let side = client_game_side!(game, game.game_side_client());
    let mut motion = MotionWorld {
        side: &side,
        entities: &[],
        player: game.common.player.as_mut().unwrap(),
        scene: &mut game.common.world,
        assets: &game.common.pack.assets,
        collision_binding: game.common.collision,
        resources: game.common.motion,
        health: &game.common.health_resources,
        loop_context: &game.common.loop_context,
        damage_hooks: &game.common.damage_hooks,
        tag_items: None,
        block_registries: None,
    };
    motion.update_fluids().unwrap();
    motion.player.is_eye_in_fluid(WATER)
}
