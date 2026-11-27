use crate::game::minecarts::minecart_tnt_tests::server;
use crate::game::minecarts::minecart_tnt_tests::*;
use sunrise_gameplay_common::game::InteractTarget;
use sunrise_items::items::ItemStack;

#[test]
fn flint_and_steel_passes_on_a_tnt_cart_without_lighting_it() {
    let mut resources = None;
    let Some(mut game) = server(&mut resources) else {
        return;
    };
    tnt_cart(&mut game, 42, false);
    let selected = game.selected;
    game.inventory[selected] = Some(ItemStack::new("minecraft:flint_and_steel", 1));
    game.start_audio_capture();
    let used = game.interact_cart(Some(InteractTarget {
        id: 42,
        location: [0.0, 0.0, 0.0],
    }));
    assert!(!used, "Entity.interact passes to the held item");
    assert_eq!(game.carts[0].fuse, -1, "flint and steel lit the cart");
    let sounds = std::mem::take(&mut game.server.pending_sounds);
    game.stop_audio_capture();
    assert!(sounds.is_empty(), "{sounds:?}");
}
