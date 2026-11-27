use crate::game::native_entity_refs::tests::with_game;
use sunrise_gameplay_common::game::GameGameExt1 as _;
use sunrise_items::items::ItemStack;

#[test]
#[ignore = "daily"]
fn a_stack_is_thrown_from_the_eyes_as_player_drop_does() {
    use sunrise_core::geometry::Vec3;
    use sunrise_sim::motion::PlayerMotion;

    with_game(|game| {
        // A body to throw from: `with_game` makes none.
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
        let stone = ItemStack::new("minecraft:stone", 3);
        let before = game.dropped.len();
        let id = game.drop_stack_from_eyes(stone.clone(), true).unwrap();
        assert_eq!(game.dropped.len(), before + 1);
        let thrown = game.dropped.last().unwrap();
        assert_eq!((thrown.id, &thrown.stack), (id, &stone));
        assert!(thrown.thrower.is_some(), "the drop key names the thrower");
        game.drop_stack_from_eyes(stone.clone(), false).unwrap();
        assert!(game.dropped.last().unwrap().thrower.is_none());
        assert_eq!(
            game.drop_stack_from_eyes(ItemStack::new("minecraft:stone", 0), true),
            None,
            "an empty stack is no drop"
        );
    });
}
