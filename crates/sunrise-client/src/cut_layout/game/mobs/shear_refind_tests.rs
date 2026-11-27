//! After `player_shear_entity` returns without cancelling, the game finds the mob again by id and
//! checks that it is still there and the shears are unchanged; if not, the click is spent and
//! nothing is sheared.
use crate::game::server_side::hook_meddler_tests::with_meddler;
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};
use sunrise_gameplay_common::game::test_fixtures::fixture_resident_column;
use sunrise_items::items::ItemStack;

#[test]
#[ignore = "daily"]
fn a_mob_the_listener_removed_is_not_sheared() {
    let sheep = Arc::new(AtomicU32::new(0));
    let target = sheep.clone();
    with_meddler(
        move |host| {
            let mob = host.entity(target.load(Ordering::SeqCst)).unwrap();
            assert!(mob.discard(host));
        },
        |game, gate| {
            fixture_resident_column(&mut game.world, (0, 0));
            assert!(game.run_command("summon sheep 8.5 100 8.5 {NoAI:1b,Silent:1b}"));
            sheep.store(game.mobs[0].id, Ordering::SeqCst);
            game.server.lan_acting_uuid = Some(7);
            game.server.lan_acting_name = Some("Alex".into());
            let selected = game.selected;
            game.inventory[selected] = Some(ItemStack::new("minecraft:shears", 1));
            gate.arm();
            assert_eq!(game.player_shear(0), Some(true));
            assert!(gate.fired());
            assert!(game.mobs.iter().all(|m| m.removed), "the sheep is gone");
            assert_eq!(
                game.held().map(|h| h.damage),
                Some(0),
                "the shears are not worn"
            );
        },
    );
}
