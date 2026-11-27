use sunrise_core::nbt::Tag;
use sunrise_protocol::{
    connection::Pipe,
    packets::play::{BossAction, BossEvent, Clientbound},
};
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay_common::game::in_own_process,
};

#[test]
#[ignore = "daily"]
fn boss_darkening_interpolates_across_zero_and_holds_the_negative_endpoint() {
    in_own_process(
        "cut_layout::game::client_side::remote_client::client_level_tick::boss_darken_tests::boss_darkening_interpolates_across_zero_and_holds_the_negative_endpoint",
        || {
            with_client_game(|game| {
                let (client, _server) = Pipe::pair();
                game.connect_local(client, "BossDarkeningTest").unwrap();
                let packet = |action| Clientbound::BossEvent(BossEvent { id: 1, action });
                game.handle_packet(packet(BossAction::Add {
                    name: Tag::String("Test boss".into()),
                    progress: 1.0,
                    color: 2,
                    overlay: 0,
                    darken: true,
                    music: false,
                    fog: false,
                }))
                .unwrap();

                let check = |overlay: &sunrise_client::boss_overlay::BossOverlay,
                             expected: [f32; 3],
                             label| {
                    assert_eq!(
                        [0.0, 0.5, 1.0].map(|partial| overlay.darken_amount(partial).to_bits()),
                        expected.map(f32::to_bits),
                        "{label}"
                    );
                };
                game.tick_remote_level().unwrap();
                check(&game.client.boss_overlay, [0.0, 0.025, 0.05], "rising tick");

                game.handle_packet(packet(BossAction::UpdateProperties {
                    darken: false,
                    music: false,
                    fog: false,
                }))
                .unwrap();
                for _ in 0..4 {
                    game.tick_remote_level().unwrap();
                }
                // The first fade subtraction rounds up; the four fades end two
                // units of 2^-30 above zero, not at zero.
                let remainder = f32::EPSILON / 64.0;
                let above_zero = [remainder + 0.0125, remainder + 0.00625, remainder];
                check(&game.client.boss_overlay, above_zero, "four fade ticks");

                game.clock.set_frozen(true);
                game.tick_remote_level().unwrap();
                check(&game.client.boss_overlay, above_zero, "frozen tick");

                game.clock.set_frozen(false);
                game.tick_remote_level().unwrap();
                let negative = remainder - 0.0125;
                check(
                    &game.client.boss_overlay,
                    [remainder, remainder - 0.00625, negative],
                    "first fade tick below zero",
                );
                game.tick_remote_level().unwrap();
                check(
                    &game.client.boss_overlay,
                    [negative; 3],
                    "next tick leaves the negative endpoint untouched",
                );
            });
        },
    );
}
