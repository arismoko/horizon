use crate::game::integrated_pair::{IntegratedPair, with_integrated_pair};
use mod_api::{Flow, HostSide, SoundPlay, SoundSource};
use std::sync::{Arc, Mutex};
use sunrise_gameplay_common::game::remote::{C, PlayerMotion, Vec3, play};

#[derive(Default)]
pub(crate) struct Log {
    pub(in crate::game) requests: Vec<&'static str>,
    pub(in crate::game) played: Vec<SoundPlay>,
}
pub(crate) struct Tuner(pub(crate) Arc<Mutex<Log>>);
#[mod_api::__subscriptions]
impl mod_api::Hooks for Tuner {
    fn play_level_sound_at_position(
        &self,
        host: HostSide<'_>,
        e: &mut mod_api::PlayLevelSoundAtPositionEvent,
    ) -> Flow {
        assert!(matches!(host, HostSide::Client(_)));
        self.0.lock().unwrap().requests.push("position");
        assert_eq!(e.position, [1.0, 64.0, -2.0]);
        assert_eq!((e.original_volume, e.original_pitch), (1.0, 0.75));
        if e.sound.to_string() == "minecraft:entity.pig.ambient" {
            return Flow::Cancel;
        }
        *e.sound = mod_api::Identifier::parse("minecraft:entity.cow.ambient").unwrap();
        *e.source = SoundSource::Ambient;
        (e.volume, e.pitch) = (0.25, 1.5);
        Flow::Continue
    }
    fn play_level_sound_at_entity(
        &self,
        host: HostSide<'_>,
        e: &mut mod_api::PlayLevelSoundAtEntityEvent,
    ) -> Flow {
        assert!(matches!(host, HostSide::Client(_)));
        assert_eq!(e.entity.id(), 7);
        assert_eq!((e.original_volume, e.original_pitch), (2.0, 0.5));
        self.0.lock().unwrap().requests.push("entity");
        *e.source = SoundSource::Records;
        (e.volume, e.pitch) = (0.5, 2.0);
        Flow::Continue
    }
    fn play_sound(&self, _: &mut dyn mod_api::ClientHost, e: &mut mod_api::PlaySoundEvent<'_>) {
        // Active sounds also refresh their playback parameters on ticks.
        if e.starting {
            self.0.lock().unwrap().played.push(e.original.clone());
        }
    }
}

fn with_sound_playback_client(test: impl FnOnce(&mut IntegratedPair<'_, '_, '_>)) {
    with_integrated_pair(|pair| {
        // The test delivers packets directly to the client using main's logged-client ID.
        pair.client.client.remote.as_mut().unwrap().entity_id = 7;
        test(pair);
    });
}

#[test]
#[ignore = "daily"]
fn packets_ask_client_level_before_playback_not_ui() {
    use sunrise_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs, LoopSide};
    let log = Arc::new(Mutex::new(Log::default()));
    let mods = vec![mod_api::ModEntry::new(
        "tuner",
        Box::new(Tuner(log.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_sound_playback_client(|pair| {
            let game = &mut *pair.client;
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(3.0, 64.0, 4.0)).unwrap());
            let inputs = Arc::new(LoadInputs::new(
                Default::default(),
                game.options.resources.clone(),
            ));
            let context = LoadContext::new(DataLoadOrigin::ClientPreload, inputs, []);
            game.test_audio_mut()
                .set_loop_context(context.loop_context(LoopSide::Client, []));
            // Exercise the real engine and mixer bookkeeping without opening a device.
            game.test_audio_mut().engine.loaded = true;
            game.test_audio_mut().engine.registry.add_file(
                "minecraft",
                br#"{
                "entity.cow.ambient": {"sounds": ["cow"]},
                "item.goat_horn.sound.0": {"sounds": ["horn"]}
            }"#,
                &|_, _| true,
            );
            let deliver = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>, packet: C| {
                let body = packet.encode();
                let mut reader = sunrise_protocol::buf::Reader::new(&body);
                let id = reader.varint().unwrap();
                game.handle_packet(C::decode(id, reader.rest()).unwrap())
                    .unwrap();
            };
            let position = |name: &str| {
                C::Sound(play::Sound {
                    sound: (0, Some((name.into(), None))),
                    source: SoundSource::Blocks as i32,
                    x: 8,
                    y: 512,
                    z: -16,
                    volume: 1.0,
                    pitch: 0.75,
                    seed: 91,
                })
            };
            deliver(game, position("minecraft:block.chest.locked"));
            {
                let log = log.lock().unwrap();
                assert_eq!(log.requests, ["position"]);
                let heard = &log.played[0];
                assert_eq!(heard.identifier, "minecraft:entity.cow.ambient");
                assert_eq!(
                    (heard.source, heard.volume, heard.pitch),
                    (SoundSource::Ambient, 0.25, 1.5)
                );
                assert_eq!(heard.position, [1.0, 64.0, -2.0]);
            }
            deliver(game, position("minecraft:entity.pig.ambient"));
            assert_eq!(
                log.lock().unwrap().played.len(),
                1,
                "cancel precedes the engine hook"
            );
            let entity = |id| {
                C::SoundEntity(play::SoundEntity {
                    sound: (0, Some(("minecraft:item.goat_horn.sound.0".into(), None))),
                    source: SoundSource::Players as i32,
                    id,
                    volume: 2.0,
                    pitch: 0.5,
                    seed: -17,
                })
            };
            deliver(game, entity(7));
            {
                let log = log.lock().unwrap();
                assert_eq!(log.requests, ["position", "position", "entity"]);
                let heard = &log.played[1];
                assert_eq!(
                    (heard.source, heard.volume, heard.pitch),
                    (SoundSource::Records, 0.5, 2.0)
                );
                assert_eq!(heard.position, [3.0, 64.0, 4.0]);
            }
            let bound = game.test_audio_mut().engine.bound_entities();
            assert_eq!(bound.len(), 1);
            assert_eq!(bound[0].id, 7);
            game.player.as_mut().unwrap().position.x = 9.0;
            game.tick_sound_engine();
            assert_eq!(bound[0].state().position, [9.0, 64.0, 4.0]);
            deliver(game, entity(999));
            {
                let log = log.lock().unwrap();
                assert_eq!(
                    log.requests,
                    ["position", "position", "entity"],
                    "absent emitter asks no level hook"
                );
                assert_eq!(log.played.len(), 2, "absent emitter starts no playback");
            }
            game.test_play_audio_button_click();
            let log = log.lock().unwrap();
            assert_eq!(log.requests.len(), 3, "UI playback adds no level request");
            assert_eq!(
                log.played.last().unwrap().identifier,
                "minecraft:ui.button.click"
            );
            assert!(
                pair.call(|server| server.server.bound_sounds.is_empty()),
                "client playback is never broadcast"
            );
        });
    });
}
