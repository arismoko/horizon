//! `LevelEventHandler` 1039 (`LevelEvent.SOUND_PHANTOM_BITE`): the sound a
//! phantom's bite posts on the client.
use mod_api::{ClientHost, Hooks, SoundPlay, SoundSource};
use std::sync::{Arc, Mutex};
use sunrise_core::random::{LegacyRandom, RandomSource};
use sunrise_gameplay_common::game::World;

pub(crate) fn with_game(test: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>)) {
    sunrise_client_gameplay::game::native_entity_refs::tests::with_client_game(|g| {
        g.world = World::new(g.res.facts().air, 1);
        g.client.audio.0.capture = Some(Vec::new());
        test(g);
    });
}

/// Level event 1039 is `playLocalSound(pos, SoundEvents.PHANTOM_BITE,
/// SoundSource.HOSTILE, 0.3F, random.nextFloat() * 0.1F + 0.9F, false)` with
/// `random` the client level's, at the block's centre.
#[test]
#[ignore = "daily"]
fn a_phantom_bite_level_event_plays_the_bite_at_0_3_volume() {
    use sunrise_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs, LoopSide};

    struct ObserveBite(Arc<Mutex<Vec<SoundPlay>>>);

    #[mod_api::__subscriptions]
    impl Hooks for ObserveBite {
        fn play_sound(&self, _: &mut dyn ClientHost, event: &mut mod_api::PlaySoundEvent<'_>) {
            if event.starting && event.original.identifier == "minecraft:entity.phantom.bite" {
                self.0.lock().unwrap().push(event.original.clone());
            }
        }
    }

    let sounds = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new(
        "phantom-bite-observer",
        Box::new(ObserveBite(sounds.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|g| {
            let inputs = Arc::new(LoadInputs::new(
                Default::default(),
                g.options.resources.clone(),
            ));
            let context = LoadContext::new(DataLoadOrigin::ClientPreload, inputs, []);
            g.client
                .audio
                .0
                .set_loop_context(context.loop_context(LoopSide::Client, []));
            g.client.audio.0.engine.loaded = true;
            *g.client.particles.0.level_random() = LegacyRandom::new(7);
            let pitch = LegacyRandom::new(7).next_float() * 0.1 + 0.9;
            g.client_level_event_at(1039, [0, 64, 0], 0);
            let sounds = sounds.lock().unwrap();
            assert_eq!(sounds.len(), 1);
            let bite = &sounds[0];
            assert_eq!(bite.volume, 0.3);
            assert_eq!(bite.pitch, pitch);
            assert_eq!(bite.position, [0.5, 64.5, 0.5]);
            assert_eq!(bite.source, SoundSource::Hostile);
            assert!(g.client.audio.0.capture.as_ref().unwrap().is_empty());
        });
    });
}
