//! `LivingEntity.detectEquipmentUpdates` runs in the player's tick, after the tick's packets:
//! a slot change and an attack sent in one tick act with the old item's attributes (the
//! attribute swap, #637, #808).
use std::sync::Arc;
use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use sunrise_items::items::ItemStack;
use sunrise_level::chunk::Chunk;
use sunrise_protocol::packets::play;
use sunrise_protocol::packets::play::Serverbound as S;
use {
    sunrise_gameplay::game::server_side::native_item_places::tests::open_to_lan,
    sunrise_gameplay_common::game::server_side::native_item_places::tests::guest,
};

#[test]
#[ignore = "daily"]
fn a_new_slot_takes_its_items_attributes_at_the_guests_tick() {
    with_game(|game| {
        open_to_lan(game);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let mut g = guest(1);
        g.loaded = true;
        g.position = [8.5, 64.0, 8.5];
        g.inventory[0] = Some(ItemStack::new("minecraft:diamond_sword", 1));
        g.inventory[1] = Some(ItemStack::new("minecraft:iron_spear", 1));
        g.selected = 0;
        game.server.lan.as_mut().unwrap().hub.guests.extend([g]);
        game.tick_guests();
        let damage = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
            game.as_guest(1, |g| g.attack_damage()).unwrap()
        };
        // The player's 1 and the diamond sword's base_attack_damage of 6.
        assert_eq!(damage(game), 7.0);
        game.lan_guest_packet(1, S::SetCarriedItem(play::SetCarriedItem { slot: 1 }));
        assert_eq!(
            damage(game),
            7.0,
            "the sword's modifiers stay until the tick"
        );
        game.tick_guests();
        // The iron spear's base_attack_damage is 2.
        assert_eq!(damage(game), 3.0);
    });
}

#[test]
#[ignore = "daily"]
fn gear_swapped_and_swapped_back_before_the_tick_keeps_its_modifiers() {
    with_game(|game| {
        open_to_lan(game);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let mut g = guest(1);
        g.loaded = true;
        g.position = [8.5, 64.0, 8.5];
        g.inventory[0] = Some(ItemStack::new("minecraft:diamond_sword", 1));
        g.inventory[1] = Some(ItemStack::new("minecraft:iron_helmet", 1));
        g.armor[0] = Some(ItemStack::new("minecraft:diamond_helmet", 1));
        g.selected = 0;
        game.server.lan.as_mut().unwrap().hub.guests.extend([g]);
        game.tick_guests();
        let damage = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
            game.as_guest(1, |g| g.attack_damage()).unwrap()
        };
        assert_eq!(damage(game), 7.0);
        // The hotbar helmet equipped from the hand, then the returned one equipped back:
        // each swap rebuilds the modifiers with a helmet in hand where vanilla waits.
        game.lan_guest_packet(1, S::SetCarriedItem(play::SetCarriedItem { slot: 1 }));
        for _ in 0..2 {
            game.as_guest(1, |g| {
                std::mem::swap(&mut g.common.armor[0], &mut g.common.inventory[1]);
                let head = g.armor[0].clone();
                g.player_equipped("head", head.as_ref());
                g.apply_equipment();
            });
        }
        game.lan_guest_packet(1, S::SetCarriedItem(play::SetCarriedItem { slot: 0 }));
        game.tick_guests();
        assert_eq!(
            damage(game),
            7.0,
            "the sword's modifiers survive a swap undone between ticks"
        );
    });
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::player_equip::PlayerEquipGameExt2 as _;
