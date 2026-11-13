use crate::game::native_entity_refs::tests::with_game;
use horizon_core::geometry::Vec3;

#[test]
#[ignore = "needs prepared resources"]
fn relative_item_teleports_keep_only_relative_horizontal_motion() {
    with_game(|game| {
        game.ui.registries.permission = 4;
        game.spawn_item_own(
            horizon_client::items::ItemStack::new("minecraft:stone", 1),
            [0.0, 64.0, 0.0],
            10,
        );
        game.dropped[0].velocity = Vec3::new(1.0, -2.0, 3.0);
        assert!(game.run_command("tp @e[type=minecraft:item] ~ 70 12"));
        assert_eq!(game.dropped[0].velocity, Vec3::new(1.0, 0.0, 0.0));
        game.dropped[0].velocity = Vec3::new(1.0, -2.0, 3.0);
        assert!(game.run_command("tp @e[type=minecraft:item] 12 ~ ~"));
        assert_eq!(game.dropped[0].velocity, Vec3::new(0.0, 0.0, 3.0));
        assert!(game.run_command("tp @e[type=minecraft:item] 10 70 10"));
        assert_eq!(game.dropped[0].velocity, Vec3::ZERO);
    });
}

/// `/tp` into another level takes an item too (`TeleportCommand` ->
/// `Entity.teleportTo(ServerLevel, ..)` -> `teleportCrossDimension`), and its
/// transition has no delta and no `Relative.DELTA`: the item arrives still.
#[test]
#[ignore = "needs prepared resources"]
fn an_item_teleported_into_another_level_arrives_there_still() {
    with_game(|game| {
        game.ui.registries.permission = 4;
        game.spawn_item_own(
            horizon_client::items::ItemStack::new("minecraft:stick", 3),
            [0.0, 64.0, 0.0],
            10,
        );
        game.dropped[0].velocity = Vec3::new(1.0, -2.0, 3.0);
        assert!(game.run_command(
            "execute in minecraft:the_nether run tp @e[type=minecraft:item] 1.5 70 1.5"
        ));
        assert!(game.dropped.is_empty());
        game.enter_level(crate::game::Dim::Nether).unwrap();
        assert_eq!(game.dropped.len(), 1);
        let item = &game.dropped[0];
        assert_eq!(
            (&*item.stack.item, item.stack.count),
            ("minecraft:stick", 3)
        );
        assert_eq!(item.position, Vec3::new(1.5, 70.0, 1.5));
        assert_eq!(item.velocity, Vec3::ZERO);
    });
}
