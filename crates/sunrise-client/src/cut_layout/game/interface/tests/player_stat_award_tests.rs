//! The statistics `ServerPlayer` awards for what the player does, not the
//! counters a mob or a block posts: a ground jump (`ServerPlayer.jumpFromGround`),
//! a worn-out piece of equipment (`ServerPlayer.onEquippedItemBroken`) and the
//! bed slept in (`ServerPlayer.startSleepInBed`, whose statistic the bed picks:
//! `StrawBedBlock.getSleptInBedStatType`).
use sunrise_core::geometry::Vec3;
use sunrise_gameplay::game::game_tests::tests::server_game;
use sunrise_gameplay_common::game_side;
use sunrise_items::items::ItemStack;
use sunrise_sim::motion::{MotionWorld, PlayerMotion};
use sunrise_world::registry::BlockStateId;

/// `ServerPlayer.jumpFromGround`: a jump off the ground counts as `custom.jump`.
#[test]
#[ignore = "daily"]
fn a_ground_jump_awards_the_jump_statistic() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    let mut player = PlayerMotion::new(game.motion, Vec3::new(2.5, 64.0, 2.5)).unwrap();
    player.ground.on_ground = true;
    {
        let meta =
            sunrise_gameplay_common::game::side_ops::world::WorldSideGameExt::side_game_side_meta(
                &game,
            );
        let side = game_side!(
            game.common,
            meta,
            sunrise_gameplay_common::game::side_ops::tables::SideOps::side_level_features(
                &game.server,
                game.common.level.enabled_features.as_arc(),
            ),
            sunrise_gameplay_common::game::side_ops::tables::SideOps::side_game_tag_items(
                &game.server,
                &game.common.res,
            )
        );
        let mut world = MotionWorld {
            side: &side,
            entities: &[],
            player: &mut player,
            scene: &mut game.common.world,
            assets: &game.common.pack.assets,
            collision_binding: game.common.collision,
            resources: game.common.motion,
            health: &game.common.health_resources,
            tag_items: None,
            loop_context: &game.common.loop_context,
            damage_hooks: &game.common.damage_hooks,
            block_registries: Some(&game.common.pack.block_registries),
        };
        world.jump_from_ground().unwrap();
    }
    assert!(player.velocity.y > 0.0, "the jump pushed the player up");
    let events = std::mem::take(&mut player.events);
    game.player = Some(player);
    // The player's turn on the server reads the tick's events (`guest_server_tick`).
    game.record_combat(&events);
    assert_eq!(game.stats.get("custom", "minecraft:jump"), 1);
}

/// `ServerPlayer.onEquippedItemBroken` for a helmet worn out by a hit: the
/// piece leaves the slot and `broken.<item>` counts it.
#[test]
#[ignore = "daily"]
fn a_worn_out_helmet_awards_the_broken_statistic() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 64.0, 2.5)).unwrap());
    let mut helmet = ItemStack::new("minecraft:leather_helmet", 1);
    helmet.damage = 54;
    game.armor[0] = Some(helmet);
    // `getDamageAfterArmorAbsorb` -> `Player.hurtArmor`: max(1, 4 / 4) points.
    let source = sunrise_sim::health::HurtSource {
        damage_type: "minecraft:generic",
        causing: 0,
        direct: 0,
        position: None,
        direct_type: None,
    };
    game.hurt_armor(&source, 4.0);
    assert!(game.armor[0].is_none(), "the helmet broke off the head");
    assert_eq!(game.stats.get("broken", "minecraft:leather_helmet"), 1);
}

/// A bed's state with its `part`, facing north.
fn bed_state(
    game: &sunrise_gameplay::game::ServerGame<'_>,
    kind: &str,
    part: &str,
) -> BlockStateId {
    let registry = game.pack.assets.registry;
    let mut state = registry
        .state(registry.default_state(kind).unwrap())
        .unwrap()
        .clone();
    state.properties.insert("part".into(), part.into());
    registry.id(&state).unwrap()
}

/// Both halves of a bed of `kind` at `x`, facing north: the foot at z+1, the
/// head at z. Answers the head.
fn place_bed(game: &mut sunrise_gameplay::game::ServerGame<'_>, x: i32, kind: &str) -> [i32; 3] {
    let foot = bed_state(game, kind, "foot");
    let head = bed_state(game, kind, "head");
    game.world.set(x, 64, 3, foot).unwrap();
    game.world.set(x, 64, 2, head).unwrap();
    [x, 64, 2]
}

/// `ServerPlayer.startSleepInBed` awards the bed's own statistic: a straw bed
/// counts as `sleep_in_straw_bed` (`StrawBedBlock.getSleptInBedStatType`), and
/// an ordinary bed still as `sleep_in_bed`.
#[test]
#[ignore = "daily"]
fn sleeping_in_a_straw_bed_awards_its_own_statistic() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    // `Level.isDarkOutside`: midnight, so a bed may be slept in.
    game.world.day_time = 18000;
    let straw = place_bed(&mut game, 2, "minecraft:straw_bed");
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 64.0, 3.5)).unwrap());
    game.try_sleep(straw);
    assert_eq!(
        game.sleeping,
        Some(straw),
        "the player lay in the straw bed"
    );
    assert_eq!(game.stats.get("custom", "minecraft:sleep_in_straw_bed"), 1);
    assert_eq!(game.stats.get("custom", "minecraft:sleep_in_bed"), 0);
    game.wake_up(true, true);
    // An ordinary bed keeps the plain statistic.
    let red = place_bed(&mut game, 4, "minecraft:red_bed");
    game.player.as_mut().unwrap().position = Vec3::new(4.5, 64.0, 3.5);
    game.try_sleep(red);
    assert_eq!(game.sleeping, Some(red), "the player lay in the red bed");
    assert_eq!(game.stats.get("custom", "minecraft:sleep_in_bed"), 1);
    assert_eq!(game.stats.get("custom", "minecraft:sleep_in_straw_bed"), 1);
}
