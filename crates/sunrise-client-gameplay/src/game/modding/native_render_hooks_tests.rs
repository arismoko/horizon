//! The render crate's hook slots follow the mods: filled while an enabled mod listens,
//! cleared once it is switched off.
use crate::game::ClientGame;
use crate::game::native_entity_refs::tests::with_client_game;
use mod_api::{ClientParticleAddEvent, Flow, Hooks};
use sunrise_level::particle_options::{ParticleOptions, ParticleType};

struct Gust;
#[mod_api::__subscriptions]
impl Hooks for Gust {
    fn client_particle_add(&self, event: &mut ClientParticleAddEvent<'_>) -> Flow {
        event.pos[0] += 100.0;
        Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn particle_add_slot_follows_the_listening_mod() {
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new("gust", Box::new(Gust))].into(),
        || {
            with_client_game(|game| {
                let added_x = |game: &mut ClientGame<'_>| {
                    game.sync_render_hooks();
                    game.client.particles.0.requested = Some(Vec::new());
                    let smoke = ParticleOptions::simple(ParticleType::CampfireCosySmoke);
                    game.client
                        .particles
                        .0
                        .add(&smoke, [1.0, 2.0, 3.0], [0.0; 3]);
                    let requested = game.client.particles.0.requested.take().unwrap();
                    assert_eq!(requested.len(), 1);
                    requested[0].1[0]
                };
                assert_eq!(added_x(game), 101.0);
                assert!(mod_api::__private::set_mod_enabled("gust", false));
                assert_eq!(added_x(game), 1.0);
                assert!(mod_api::__private::set_mod_enabled("gust", true));
                assert_eq!(added_x(game), 101.0);
            });
        },
    );
}
