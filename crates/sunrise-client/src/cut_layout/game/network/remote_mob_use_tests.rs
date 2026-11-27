#![cfg(feature = "client")]
use crate::game::server_side::native_entity_refs::tests::with_client_game;
use {
    sunrise_items::items::ItemStack, sunrise_sim::first_person_sim::Hand, sunrise_sim::mobs::Mob,
};

#[test]
#[ignore = "daily"]
fn remote_mob_takes_only_adult_piglin_currency_with_free_offhand() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        with_client_game(|g| {
            let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
            g.connect_local(pipe, "PiglinUseTest").unwrap();
            g.player = Some(
                sunrise_sim::motion::PlayerMotion::new(
                    g.motion,
                    sunrise_core::geometry::Vec3::new(4.5, 100.0, 4.5),
                )
                .unwrap(),
            );
            let kind = g.res.mob_types().get("minecraft:piglin").unwrap().clone();
            g.mobs.push(Mob::new(
                7,
                kind,
                sunrise_core::geometry::Vec3::new(0.0, 100.0, 0.0),
                0.0,
            ));
            g.mobs[0]
                .extra
                .set_slot_stack(sunrise_items::equipment::OFFHAND, None);
            g.mobs[0].extra.age = 0;
            g.common.inventory[g.common.selected] = Some(ItemStack::new("minecraft:gold_ingot", 1));
            assert!(g.remote_mob_takes(0, Hand::Main));
            g.mobs[0].extra.age = -24000;
            assert!(!g.remote_mob_takes(0, Hand::Main));
            g.mobs[0].extra.age = 0;
            g.common.inventory[g.common.selected] =
                Some(ItemStack::new("minecraft:golden_apple", 1));
            assert!(!g.remote_mob_takes(0, Hand::Main));
            g.remote_use_item(Hand::Main);
            assert!(g.using.is_some());
            g.common.inventory[g.common.selected] = Some(ItemStack::new("minecraft:gold_ingot", 1));
            g.mobs[0].extra.set_slot_stack(
                sunrise_items::equipment::OFFHAND,
                Some(ItemStack::new("minecraft:gold_ingot", 1)),
            );
            assert!(!g.remote_mob_takes(0, Hand::Main));
        })
    });
}
