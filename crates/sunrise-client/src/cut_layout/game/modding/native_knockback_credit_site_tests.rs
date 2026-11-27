//! Pushes that no melee path starts still carry their owner as `living_knockback`'s kill
//! credit: owned evoker fangs, a harming splash potion and a harming area cloud, each heard by
//! a real listener, with the credit it received and the motion it left.
use sunrise_core::geometry::Vec3;
use sunrise_gameplay::game::server_side::potions_server::PotionOwner;
use sunrise_items::items::ItemStack;
use sunrise_sim::mobs::Who;
use {
    crate::game::server_side::native_knockback_tests::assert_sword_credit_reached_the_push,
    crate::game::server_side::native_knockback_tests::credit_pig,
    crate::game::server_side::native_knockback_tests::heard,
    crate::game::server_side::native_knockback_tests::sword_evoker,
};

fn credited_push(hit: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
    let mut pushed = Vec3::ZERO;
    let heard = heard(|game, log| {
        log.lock().unwrap().reduce_sword_credit = true;
        credit_pig(game);
        sword_evoker(game);
        hit(game);
        pushed = game.mobs[0].velocity;
    });
    assert_sword_credit_reached_the_push(&heard, pushed);
}

#[test]
#[ignore = "daily"]
fn owned_fangs_credit_their_evoker_and_its_sword() {
    credited_push(|game| {
        // Bites on this tick, at the pig's feet.
        game.insert_fangs(900, [0.0, 0.0, 0.0], 0.0, -7, 50);
        game.tick_fangs();
    });
}

#[test]
#[ignore = "daily"]
fn a_harming_splash_potion_credits_its_thrower_and_its_sword() {
    credited_push(|game| {
        let mut potion = ItemStack::new("minecraft:splash_potion", 1);
        potion.data_mut().potion = Some("minecraft:harming".into());
        let owner = PotionOwner {
            attacker: Some(50),
            ..Default::default()
        };
        game.splash(
            &potion,
            Vec3::new(0.0, 0.0, 0.0),
            None,
            None,
            0,
            0,
            &owner,
            true,
            Vec3::ZERO,
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_harming_area_cloud_credits_its_owner_and_its_sword() {
    credited_push(|game| {
        let text =
            sunrise_game_data::text::Text::translate("entity.minecraft.area_effect_cloud", &[]);
        game.cloud_affect(
            sunrise_gameplay_common::game::potions_common::Victim::Mob(100),
            &[("instant_damage", 1, 0)],
            (&[], &[], 1.0),
            [0.0, 0.0, 0.0],
            &text,
            (Some(Who::Mob(50)), None),
            None,
        );
    });
}
