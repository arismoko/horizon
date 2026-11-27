use crate::game::fireworks::impact::tests::launch;
use crate::game::server_side::native_projectiles::impact_probe::{Action, with_probe};

#[test]
#[ignore = "daily"]
fn rocket_impact_listener_can_save_the_advanced_rocket() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::server_side::fireworks_server::impact::tests::rocket_impact_listener_can_save_the_advanced_rocket",
        || {
            with_probe(Action::SaveOnImpact, true, |game, observed| {
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(2, 64, 0, stone).unwrap();
                launch(game, 100);
                game.tick_rockets();

                let hits = observed.hits.lock().unwrap();
                assert_eq!(hits.len(), 1);
                assert_eq!(hits[0].projectile, 71);
                let saved = game.server.save.0.load_entities(0, 0).unwrap();
                let rocket = saved
                    .iter()
                    .find(|tag| {
                        sunrise_sim::entity_nbt::kind(tag).as_deref()
                            == Some("minecraft:firework_rocket")
                    })
                    .expect("impact save includes the resident rocket");
                let saved_position = sunrise_sim::entity_nbt::position(rocket).unwrap();
                assert_eq!(
                    [saved_position.x, saved_position.y, saved_position.z].map(f64::to_bits),
                    hits[0].projectile_pos.map(f64::to_bits),
                );
                assert!(hits[0].projectile_pos[0] > 0.5);
            });
        },
    );
}
