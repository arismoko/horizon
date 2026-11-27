use super::super::native_entity_refs::tests::with_client_game;
use super::*;
use crate::game::client_side::native_client_session::tests::logged_client;
#[derive(Debug, PartialEq)]
pub(in crate::game) struct Request {
    pub sound: String,
    pub(in crate::game) source: SoundSource,
    pub(in crate::game) emitter: Option<u32>,
    pub(in crate::game) position: Option<[f64; 3]>,
}
pub(in crate::game) struct LocalTuner(pub(in crate::game) Arc<Mutex<Vec<Request>>>);
#[mod_api::__subscriptions]
impl mod_api::Hooks for LocalTuner {
    fn play_level_sound_at_position(
        &self,
        host: HostSide<'_>,
        e: &mut mod_api::PlayLevelSoundAtPositionEvent,
    ) -> Flow {
        assert!(matches!(host, HostSide::Client(_)));
        self.0.lock().unwrap().push(Request {
            sound: e.sound.to_string(),
            source: *e.source,
            emitter: None,
            position: Some(e.position),
        });
        Flow::Cancel
    }
    fn play_level_sound_at_entity(
        &self,
        host: HostSide<'_>,
        e: &mut mod_api::PlayLevelSoundAtEntityEvent,
    ) -> Flow {
        assert!(matches!(host, HostSide::Client(_)));
        let sound = e.sound.to_string();
        self.0.lock().unwrap().push(Request {
            sound: sound.clone(),
            source: *e.source,
            emitter: Some(e.entity.id()),
            position: None,
        });
        if sound == "minecraft:block.honey_block.slide" {
            Flow::Continue
        } else {
            Flow::Cancel
        }
    }
}
use mod_api::{Flow, HostSide, SoundSource};
use std::sync::Mutex;

#[test]
#[ignore = "daily"]
fn game_events_ask_client_level_before_playback() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new(
        "game_event_tuner",
        Box::new(LocalTuner(requests.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_client_game(|game| {
            let _server = logged_client(game);
            let mut player = PlayerMotion::new(game.motion, Vec3::new(1.0, 64.0, 1.0)).unwrap();
            player.dimensions.eye_height = 1.25;
            game.player = Some(player);
            for (event, param) in [(6, 1.0), (9, 1.0), (10, 1.4)] {
                game.handle_packet(C::GameEvent(play::GameEvent { event, param }))
                    .unwrap();
            }
            assert_eq!(
                *requests.lock().unwrap(),
                [
                    Request {
                        sound: "minecraft:entity.arrow.hit_player".into(),
                        source: SoundSource::Players,
                        emitter: None,
                        position: Some([1.0, 65.25, 1.0])
                    },
                    Request {
                        sound: "minecraft:entity.puffer_fish.sting".into(),
                        source: SoundSource::Neutral,
                        emitter: None,
                        position: Some([1.0, 64.0, 1.0])
                    },
                    Request {
                        sound: "minecraft:entity.elder_guardian.curse".into(),
                        source: SoundSource::Hostile,
                        emitter: None,
                        position: Some([1.0, 64.0, 1.0])
                    },
                ]
            );
            assert!(
                game.client.audio.0.heard.is_empty(),
                "all three canceled requests stop before Audio"
            );
            game.handle_packet(C::GameEvent(play::GameEvent {
                event: 10,
                param: 1.5,
            }))
            .unwrap();
            assert_eq!(
                requests.lock().unwrap().len(),
                3,
                "a silent curse asks no sound hook"
            );
            let level = sunrise_client::particles::ParticleLevel {
                world: &game.common.world,
                facts: game.common.res.facts(),
                collision: game.common.collision,
                player: None,
                cached_chunk: std::cell::Cell::new(None),
            };
            game.client.particles.0.tick(&level);
            assert_eq!(
                game.client.particles.0.elder_guardians().len(),
                2,
                "neither silent nor canceled sound suppresses the apparition"
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn local_level_requests_can_mute_sound_without_undoing_prediction() {
    use sunrise_core::position::BlockPos;
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new(
        "local_tuner",
        Box::new(LocalTuner(requests.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_client_game(|game| {
            let _server = logged_client(game);
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(1.0, 64.0, 1.0)).unwrap());
            game.client
                .streaming
                .insert_remote(&mut game.common.world, Chunk::empty(0, 0));
            let old = game
                .pack
                .assets
                .registry
                .default_state("minecraft:oak_door")
                .unwrap();
            let new = game
                .pack
                .assets
                .registry
                .default_state("minecraft:birch_door")
                .unwrap();
            game.world.set(1, 64, 1, old).unwrap();
            let at = BlockPos::new(1, 64, 1);
            game.predict_use(
                &(
                    vec![(at, new)],
                    vec![(
                        at,
                        "block.wooden_door.open".into(),
                        mod_api::SoundSource::Blocks,
                        1.0,
                        0.9,
                    )],
                ),
                1,
            );
            assert_eq!(
                game.world.block(1, 64, 1),
                new,
                "sound cancellation does not cancel the use"
            );
            game.player_menu_sound(sunrise_items::bundle::INSERT_FAIL);
            let stack = ItemStack {
                item: "minecraft:apple".into(),
                count: 1,
                damage: 0,
                extra: None,
            };
            let consumable = game
                .items()
                .get(&stack.item)
                .unwrap()
                .consumable
                .clone()
                .unwrap();
            game.emit_consume(&consumable, &stack, 16);
            game.play_player_bound_sound("minecraft:item.goat_horn.sound.0", 2, 8.0, 1.0, true);
            assert!(
                game.client.audio.0.heard.is_empty(),
                "all four canceled requests stop before Audio"
            );
            let expected = [
                Request {
                    sound: "minecraft:block.wooden_door.open".into(),
                    source: SoundSource::Blocks,
                    emitter: None,
                    position: Some([1.5, 64.5, 1.5]),
                },
                Request {
                    sound: sunrise_items::bundle::INSERT_FAIL.into(),
                    source: SoundSource::Players,
                    emitter: Some(7),
                    position: None,
                },
                Request {
                    sound: "minecraft:entity.generic.eat".into(),
                    source: SoundSource::Players,
                    emitter: Some(7),
                    position: None,
                },
                Request {
                    sound: "minecraft:item.goat_horn.sound.0".into(),
                    source: SoundSource::Records,
                    emitter: Some(7),
                    position: None,
                },
            ];
            assert_eq!(*requests.lock().unwrap(), expected);
            game.client.audio.0.engine.loaded = true;
            game.play_client_player_sound("minecraft:block.honey_block.slide", 1.0, 1.0);
            assert_eq!(
                game.client
                    .audio
                    .0
                    .heard
                    .get("minecraft:block.honey_block.slide"),
                Some(&1)
            );
            assert!(
                game.client.audio.0.engine.bound_entities().is_empty(),
                "LocalPlayer.playSound stays positional"
            );
        });
    });
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::entity_common::EntityCommonGameExt1 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;
