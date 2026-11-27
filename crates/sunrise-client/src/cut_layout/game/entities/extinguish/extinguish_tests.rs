//! Powder snow and water cauldrons putting a burning player out: the hiss
//! (`Entity.playEntityOnFireExtinguishedSound`) plays either way, the block
//! change only where `mayInteract` allows it.
use sunrise_core::geometry::Vec3;
use sunrise_gameplay::game::game_tests::tests::server_game;
use sunrise_sim::motion::{MotionWorld, PlayerMotion};
use sunrise_world::chunk::BlockState;
use sunrise_world::entity_move::MovementRecord;
use sunrise_world::registry::BlockStateId;

/// A server's game on the prepared resources, or None when they are absent
/// (CI has none: the tests then skip quietly).
pub(crate) fn server<'a>(
    resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
) -> Option<sunrise_gameplay::game::ServerGame<'a>> {
    std::env::var_os("SUNRISE_TEST_RESOURCES")?;
    Some(server_game(resources))
}

pub(crate) fn state(
    game: &sunrise_gameplay::game::ServerGame<'_>,
    name: &str,
    level: &str,
) -> BlockStateId {
    let mut state = BlockState {
        name: name.into(),
        properties: Default::default(),
    };
    if !level.is_empty() {
        state.properties.insert("level".into(), level.into());
    }
    game.pack.assets.registry.resolve(&state).unwrap()
}

pub(crate) fn block(game: &sunrise_gameplay::game::ServerGame<'_>, pos: [i32; 3]) -> String {
    let state = game.world.block(pos[0], pos[1], pos[2]);
    game.res.facts().get(state).name.to_string()
}

/// A server's player standing in the block at `pos` (feet on its floor), on
/// fire: the tick `guest_server_tick` runs for it, its block effects and the
/// consumer of their events.
pub(crate) fn burn_in(game: &mut sunrise_gameplay::game::ServerGame<'_>, pos: [i32; 3]) {
    let at = Vec3::new(
        f64::from(pos[0]) + 0.5,
        f64::from(pos[1]),
        f64::from(pos[2]) + 0.5,
    );
    game.player = Some(PlayerMotion::new(game.motion, at).unwrap());
    game.player.as_mut().unwrap().fire_ticks = 100;
    let events = {
        let (common, state) = sunrise_gameplay_common::game::SideGame::common_and_state_mut(game);
        let player = common.player.as_mut().unwrap();
        // The move the client sent: standing still, inside the block.
        player.history.add(MovementRecord {
            from: at,
            to: at,
            original: None,
        });
        let level = common.native_lifecycle.epoch().and_then(|epoch| {
            let key = common.dim.key();
            let id = mod_api::__private::dimension_id(key)?;
            Some(mod_api::LevelRef::with_generation(
                epoch,
                key,
                sunrise_gameplay_common::game::Side::levels_generation(state, id),
            ))
        });
        let side = sunrise_gameplay_common::game_side!(
            common,
            (
                sunrise_gameplay_common::game::Side::lan_acting_uuid(state),
                level,
                false
            ),
            sunrise_gameplay_common::game::side_ops::tables::SideOps::side_level_features(
                state,
                common.level.enabled_features.as_arc()
            ),
            sunrise_gameplay_common::game::side_ops::tables::SideOps::side_game_tag_items(
                state,
                &common.res
            )
        );
        let mut w = MotionWorld {
            side: &side,
            entities: &[],
            player,
            scene: &mut common.world,
            assets: &common.pack.assets,
            collision_binding: common.collision,
            resources: common.motion,
            health: &common.health_resources,
            tag_items: Some(common.res.items()),
            loop_context: &common.loop_context,
            damage_hooks: &common.damage_hooks,
            block_registries: Some(&common.pack.block_registries),
        };
        w.server_block_damage(at).unwrap();
        std::mem::take(&mut w.player.events)
    };
    game.start_audio_capture();
    game.block_extinguish_player(&events);
}

/// Outside the world border, as `/worldborder center 1000 1000` and
/// `/worldborder set 100` put the origin.
pub(crate) fn outside_the_border(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    game.border.set_center([1000.0, 1000.0]).unwrap();
    game.border.set_size(100.0).unwrap();
}

/// `PowderSnowBlock.entityInside`: the powder snow is destroyed and the
/// burning player put out, with the hiss.
#[test]
fn powder_snow_puts_a_burning_player_out_with_the_hiss() {
    let mut resources = None;
    let Some(mut game) = server(&mut resources) else {
        return;
    };
    let pos = [4, 64, 4];
    let snow = state(&game, "minecraft:powder_snow", "");
    game.world.set(pos[0], pos[1], pos[2], snow).unwrap();
    burn_in(&mut game, pos);
    assert!(game.player.as_ref().unwrap().fire_ticks <= 0, "fire out");
    assert_eq!(block(&game, pos), "minecraft:air", "the snow is destroyed");
    let server_hiss = game
        .server
        .pending_sounds
        .iter()
        .filter(|s| s.event == "minecraft:entity.generic.extinguish_fire")
        .count();
    assert_eq!(server_hiss, 1, "one hiss");
}

/// `LayeredCauldronBlock.handleEntityOnFireInside`: the level-3 water
/// cauldron drops to level 2, and the hiss plays.
#[test]
fn a_cauldron_puts_a_burning_player_out_with_the_hiss() {
    let mut resources = None;
    let Some(mut game) = server(&mut resources) else {
        return;
    };
    let pos = [4, 64, 4];
    let water = state(&game, "minecraft:water_cauldron", "3");
    game.world.set(pos[0], pos[1], pos[2], water).unwrap();
    burn_in(&mut game, pos);
    assert!(game.player.as_ref().unwrap().fire_ticks <= 0, "fire out");
    let level = game
        .pack
        .assets
        .registry
        .state(game.world.block(pos[0], pos[1], pos[2]))
        .and_then(|s| s.properties.get("level").cloned());
    assert_eq!(level.as_deref(), Some("2"), "the water drops a level");
    let server_hiss = game
        .server
        .pending_sounds
        .iter()
        .filter(|s| s.event == "minecraft:entity.generic.extinguish_fire")
        .count();
    assert_eq!(server_hiss, 1, "one hiss");
}

/// `ServerPlayer.mayInteract` outside the world border: the fire still goes
/// out and the hiss still plays, but the powder snow stays.
#[test]
fn outside_the_border_powder_snow_stays_but_the_fire_goes_out() {
    let mut resources = None;
    let Some(mut game) = server(&mut resources) else {
        return;
    };
    let pos = [4, 64, 4];
    let snow = state(&game, "minecraft:powder_snow", "");
    game.world.set(pos[0], pos[1], pos[2], snow).unwrap();
    outside_the_border(&mut game);
    burn_in(&mut game, pos);
    assert!(game.player.as_ref().unwrap().fire_ticks <= 0, "fire out");
    assert_eq!(block(&game, pos), "minecraft:powder_snow", "it stays");
    let server_hiss = game
        .server
        .pending_sounds
        .iter()
        .filter(|s| s.event == "minecraft:entity.generic.extinguish_fire")
        .count();
    assert_eq!(server_hiss, 1, "the hiss still plays");
}

/// The same for a cauldron: level 3 stays level 3, the fire goes out and the
/// hiss plays.
#[test]
fn outside_the_border_a_cauldron_keeps_its_level_but_the_fire_goes_out() {
    let mut resources = None;
    let Some(mut game) = server(&mut resources) else {
        return;
    };
    let pos = [4, 64, 4];
    let water = state(&game, "minecraft:water_cauldron", "3");
    game.world.set(pos[0], pos[1], pos[2], water).unwrap();
    outside_the_border(&mut game);
    burn_in(&mut game, pos);
    assert!(game.player.as_ref().unwrap().fire_ticks <= 0, "fire out");
    let level = game
        .pack
        .assets
        .registry
        .state(game.world.block(pos[0], pos[1], pos[2]))
        .and_then(|s| s.properties.get("level").cloned());
    assert_eq!(level.as_deref(), Some("3"), "it keeps its level");
    let server_hiss = game
        .server
        .pending_sounds
        .iter()
        .filter(|s| s.event == "minecraft:entity.generic.extinguish_fire")
        .count();
    assert_eq!(server_hiss, 1, "the hiss still plays");
}
