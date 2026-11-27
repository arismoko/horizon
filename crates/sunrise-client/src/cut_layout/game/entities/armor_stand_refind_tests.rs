//! After `armor_stand_manipulate` returns without cancelling, the game finds the stand again by its
//! id and checks what the click was decided on; a change spends the click and nothing moves.
use crate::game::server_side::hook_meddler_tests::with_meddler;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::InteractTarget;
use sunrise_items::items::ItemStack;
use sunrise_sim::entities::ArmorStand;

const STAND: i32 = 21;

/// A stand with arms at the origin and a guest acting on it, holding an iron helmet.
fn set_up(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    let mut stand = ArmorStand::new(STAND as u32, Vec3::new(0.5, 64.0, 0.5), 0.0);
    stand.show_arms = true;
    game.armor_stands.push(stand);
    game.server.lan_acting_uuid = Some(7);
    game.server.lan_acting_name = Some("Alex".into());
    let selected = game.selected;
    game.inventory[selected] = Some(ItemStack::new("minecraft:iron_helmet", 1));
}

fn click() -> Option<InteractTarget> {
    Some(InteractTarget {
        id: STAND,
        location: [0.0, 1.7, 0.0],
    })
}

#[test]
#[ignore = "daily"]
fn a_stand_the_listener_removed_spends_the_click() {
    with_meddler(
        |host| {
            assert!(
                host.commands()
                    .run_command("kill @e[type=minecraft:armor_stand]")
            );
        },
        |game, gate| {
            set_up(game);
            gate.arm();
            assert!(game.interact_armor_stand(click()));
            assert!(gate.fired());
            assert!(game.armor_stands.is_empty());
            assert_eq!(
                game.held().map(|h| &*h.item),
                Some("minecraft:iron_helmet"),
                "the helmet stays in the hand"
            );
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_slot_the_listener_filled_spends_the_click() {
    with_meddler(
        |host| {
            assert!(host.commands().run_command(
                "item replace entity @e[type=minecraft:armor_stand,limit=1] armor.head with minecraft:diamond_helmet"
            ));
        },
        |game, gate| {
            set_up(game);
            gate.arm();
            assert!(game.interact_armor_stand(click()));
            assert!(gate.fired());
            let worn = game.armor_stands[0].equipment[5].as_ref();
            assert_eq!(worn.map(|w| &*w.item), Some("minecraft:diamond_helmet"));
            assert_eq!(
                game.held().map(|h| &*h.item),
                Some("minecraft:iron_helmet"),
                "the click moved nothing"
            );
        },
    );
}
