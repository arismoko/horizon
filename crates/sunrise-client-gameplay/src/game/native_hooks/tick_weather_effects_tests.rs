//! `tick_weather_effects`: a client mod takes vanilla's per-tick weather effects over
//! (`CustomWeatherEffectRenderer.tickRain`).
use crate::game::{
    ClientGame, client_side::native_client_session::tests::logged_client,
    native_entity_refs::tests::with_client_game,
};
use mod_api::{ClientHost, Flow, Hooks, ModEntry, TickWeatherEffectsEvent};
use std::sync::{Arc, Mutex};
use sunrise_client::particles::ParticleType;
use sunrise_core::geometry::Vec3;
use sunrise_level::{biome::Biomes, chunk::Chunk};
use sunrise_sim::motion::PlayerMotion;

/// The dimension, game time, and rain each call heard; cancels when `cancel` is set.
#[derive(Clone, Default)]
struct Weather {
    cancel: Arc<Mutex<bool>>,
    heard: Arc<Mutex<Vec<(String, i64, f32)>>>,
}
#[mod_api::__subscriptions]
impl Hooks for Weather {
    fn tick_weather_effects(
        &self,
        _: &dyn ClientHost,
        event: &mut TickWeatherEffectsEvent<'_>,
    ) -> Flow {
        self.heard
            .lock()
            .unwrap()
            .push((event.dimension.to_owned(), event.game_time, event.rain));
        if *self.cancel.lock().unwrap() {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

/// Rain over a stone floor in a rainy biome splashes when the mod lets it, and splashes
/// nothing, with no rain sound timing, when the mod cancels; the mod hears the level and the
/// rain each tick.
#[test]
#[ignore = "daily"]
fn a_mod_takes_the_weather_effects_over() {
    let weather = Weather::default();
    let mods = vec![ModEntry::new("weather", Box::new(weather.clone()))];
    mod_platform::testing::with_mods(mods.into(), || {
        with_client_game(|game| {
            let _server = logged_client(game);
            for cx in -2..=2 {
                for cz in -2..=2 {
                    game.world
                        .chunks
                        .insert((cx, cz), Arc::new(Chunk::empty(cx, cz)));
                }
            }
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(8.5, 65.0, 8.5)).unwrap());
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            for x in -8..24 {
                for z in -8..24 {
                    game.world.set(x, 64, z, stone).unwrap();
                }
            }
            game.world.weather.rain_level = (1.0, 1.0);
            let mut biomes = Biomes::clone(game.res.biomes());
            biomes.list[0].precipitation = true;
            biomes.list[0].temperature = 0.8;
            game.world_biomes = Some(Arc::new(biomes));
            game.client.settings.particles = 0;
            game.client.settings.sunrise.weather = true;
            game.camera_view(1.0).expect("camera over the stone floor");
            let rain = |game: &mut ClientGame<'_>| {
                game.client.particles.0.requested = Some(Vec::new());
                game.tick_remote_level().unwrap();
                game.client
                    .particles
                    .0
                    .requested
                    .take()
                    .unwrap()
                    .into_iter()
                    .filter(|(options, ..)| options.ty == ParticleType::Rain)
                    .count()
            };
            game.world.game_time = 700;
            assert!(rain(game) > 0, "vanilla's rain splashes");
            *weather.cancel.lock().unwrap() = true;
            let sound_time = game.client.test_rain_sound_time();
            assert_eq!(rain(game), 0, "the mod took the splashes over");
            assert_eq!(
                game.client.test_rain_sound_time(),
                sound_time,
                "and the rain sound's timing"
            );
            let dimension = game.dim.id().to_owned();
            assert_eq!(
                *weather.heard.lock().unwrap(),
                [(dimension.clone(), 701, 1.0), (dimension, 702, 1.0)],
            );
        });
    });
}
