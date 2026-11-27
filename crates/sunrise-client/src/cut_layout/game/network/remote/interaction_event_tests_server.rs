use crate::game::remote::interaction_event_tests::Controls;
use crate::game::remote::interaction_event_tests::Observer;
use crate::game::remote::interaction_event_tests::Step;
use crate::game::remote::interaction_event_tests::hit;
use crate::game::remote::interaction_event_tests::*;
use mod_api::InteractionResult as Answer;
use mod_api::{self, LogicalSide, TriState, UsePhase};
#[cfg(feature = "client")]
use std::sync::Mutex;
use std::sync::{
    Arc,
    atomic::{AtomicI32, AtomicUsize},
};
use sunrise_core::geometry::Vec3;
#[cfg(feature = "client")]
use sunrise_core::nbt::Tag;
#[cfg(feature = "client")]
use sunrise_gameplay_common::game::native_hooks::item_fallthrough::ItemFallthroughGameExt1;
use sunrise_protocol::packets::configuration::ClientInformation;
use {
    crate::game::server_side::native_entity_refs::tests::with_game,
    sunrise_gameplay::game::server_side::lan::LanSettings,
};
use {
    sunrise_items::items::ItemStack, sunrise_level::chunk::Chunk,
    sunrise_sim::first_person_sim::Hand, sunrise_sim::lan::Joined, sunrise_sim::lan::NetConfig,
    sunrise_sim::lan::PlayerSender, sunrise_sim::lan::guest::Guest,
    sunrise_sim::lan::hub::HubConfig, sunrise_sim::motion::PlayerMotion,
};

#[path = "interaction_matrix_tests_server.rs"]
pub(crate) mod interaction_matrix_tests;

#[path = "item_fallthrough_tests_server.rs"]
pub(crate) mod item_fallthrough_tests;

#[path = "item_used_on_block_tests_server.rs"]
pub(crate) mod item_used_on_block_tests;

#[path = "predicted_use_tests_server.rs"]
pub(crate) mod predicted_use_tests;

#[path = "use_item_hand_tests_server.rs"]
pub(crate) mod use_item_hand_tests;

#[path = "use_item_result_tests_server.rs"]
pub(crate) mod use_item_result_tests;

#[path = "interaction_key_tests_server.rs"]
pub(crate) mod interaction_key_tests;

#[cfg(feature = "client")]
#[path = "mapping_press_tests_server.rs"]
pub(crate) mod mapping_press_tests;

/// `SolidBucketItem.getPlaceSound` changes only the event: both sides use
/// the contextual sound type's levels, even with a no-op level-sound listener.
#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn powder_snow_placement_uses_the_bucket_event_and_contextual_levels() {
    use mod_api::{
        BlockPart, BlockSoundCause, Flow, Hooks, HostSide, Registrar, SoundSource, SoundTypeEvent,
    };
    use sunrise_gameplay_common::game::server_side::native_sound_refs::tests::{guest, packets};
    use sunrise_protocol::packets::play::Clientbound;

    struct PowderSound;
    #[mod_api::sunrise_part]
    impl BlockPart for PowderSound {
        fn sound_type(&self, _: mod_api::LevelSide<'_>, event: &mut SoundTypeEvent<'_>) {
            if event.cause == BlockSoundCause::Place {
                event.sound.volume = 3.0;
                event.sound.pitch = -1.25;
            }
        }
    }
    #[mod_api::__subscriptions]
    impl Hooks for PowderSound {
        fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            let powder = r.block_key("minecraft:powder_snow");
            r.block_part(powder, PowderSound);
        }
    }
    struct Observe(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for Observe {
        fn play_level_sound_at_position(
            &self,
            _: HostSide<'_>,
            e: &mut mod_api::PlayLevelSoundAtPositionEvent,
        ) -> Flow {
            if e.sound.to_string() == EMPTY_POWDER_SNOW {
                assert_eq!((e.original_volume, e.original_pitch), (2.0, -1.0));
                self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            Flow::Continue
        }
    }
    const EMPTY_POWDER_SNOW: &str = "minecraft:item.bucket.empty_powder_snow";
    if !crate::game::native_block_sounds::tests::install_sound_fixture_in_child(
        "cut_layout::game::server_side::remote_server::interaction_event_tests::powder_snow_placement_uses_the_bucket_event_and_contextual_levels",
    ) {
        return;
    }
    for listened in [false, true] {
        let observed = Arc::new(AtomicUsize::new(0));
        let mut mods = vec![mod_api::ModEntry::new(
            "powder_sound",
            Box::new(PowderSound),
        )];
        if listened {
            mods.push(mod_api::ModEntry::new(
                "observe_sound",
                Box::new(Observe(observed.clone())),
            ));
        }
        mod_platform::testing::with_mods(mods.into(), || {
            with_interaction_server_game(|game| {
                let registry = game.pack.assets.registry;
                let stone = registry.default_state("minecraft:stone").unwrap();
                let powder = registry.default_state("minecraft:powder_snow").unwrap();
                game.world.set(0, 64, 0, stone).unwrap();
                game.set_test_forced_hit(Some(hit()));
                let selected = game.selected;
                game.inventory[selected] = Some(ItemStack::new("minecraft:powder_snow_bucket", 1));
                game.start_audio_capture();
                assert!(use_block(game));
                assert_eq!(game.world.block(0, 65, 0), powder);
                let [sound] = game.server.pending_sounds.as_slice() else {
                    panic!("expected one placement sound, client=false")
                };
                assert_eq!(sound.event, EMPTY_POWDER_SNOW);
                assert_eq!(sound.source, SoundSource::Blocks.index());
                assert_eq!((sound.volume, sound.pitch), (2.0, -1.0));
                assert_eq!(sound.except_entity, Some(12));

                let (other, rx) = guest(13, 13, [0.5, 65.0, 0.5]);
                game.server.lan.as_mut().unwrap().hub.guests.push(other);
                game.lan_broadcast_effects();
                let id = game
                    .server
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .sync_data()
                    .id("minecraft:sound_event", EMPTY_POWDER_SNOW)
                    .unwrap();
                let sounds: Vec<_> = packets(&rx)
                    .into_iter()
                    .filter_map(|packet| {
                        if let Clientbound::Sound(sound) = packet {
                            Some(sound)
                        } else {
                            None
                        }
                    })
                    .collect();
                let [sound] = sounds.as_slice() else {
                    panic!("expected one sound packet")
                };
                assert_eq!(sound.sound, (id, None), "use the registered bucket event");
                assert_eq!(sound.source, SoundSource::Blocks as i32);
                assert_eq!((sound.volume, sound.pitch), (2.0, -1.0));
            });
            with_interaction_client_game(|game| {
                let registry = game.pack.assets.registry;
                let stone = registry.default_state("minecraft:stone").unwrap();
                let powder = registry.default_state("minecraft:powder_snow").unwrap();
                game.world.set(0, 64, 0, stone).unwrap();
                game.set_test_forced_hit(Some(hit()));
                let selected = game.selected;
                game.inventory[selected] = Some(ItemStack::new("minecraft:powder_snow_bucket", 1));
                game.client.audio.0.capture = Some(Vec::new());
                assert!(use_block(game));
                assert_eq!(game.world.block(0, 65, 0), powder);
                let captured = game.client.audio.0.capture.as_ref().unwrap();
                let [sound] = captured.as_slice() else {
                    panic!("expected one placement sound, client=true")
                };
                assert_eq!(sound.event, EMPTY_POWDER_SNOW);
                assert_eq!(sound.source, SoundSource::Blocks.index());
                assert_eq!((sound.volume, sound.pitch), (2.0, -1.0));
                assert_eq!(sound.except_entity, None);
            });
        });
        assert_eq!(
            observed.load(std::sync::atomic::Ordering::Relaxed),
            if listened { 2 } else { 0 }
        );
    }
}

/// One right click on a block whose Part acts in `use_item_on` and answers `Success`, with
/// something in the off hand. The client runs the Part too, so `Minecraft.startUseItem` stops
/// at the main hand: one request goes out and the off hand gets no turn. The server runs the
/// Part once for that request. The lever stays put on both sides: the Part replaced its toggle.
#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_successful_block_part_runs_once_per_click_on_both_sides() {
    fn run<'g, S: InteractionSide<'g>>(
        observer: &Observer,
        game: &mut S,
        click: impl FnOnce(&mut S),
    ) {
        let client = S::IS_CLIENT;
        let side = S::LOGICAL_SIDE;
        let lever = game
            .pack
            .assets
            .registry
            .default_state("minecraft:lever")
            .unwrap();
        game.world.set(0, 64, 0, lever).unwrap();
        game.set_test_forced_hit(Some(hit()));
        game.offhand = Some(ItemStack::new("minecraft:stick", 1));
        *observer.0.lock().unwrap() = Controls {
            hands: Some(Vec::new()),
            block_part_answer: Some(Answer::Success),
            ..Controls::default()
        };
        let before = game.swings_started;
        click(game);
        let state = observer.0.lock().unwrap();
        assert_eq!(
            state.steps,
            [Step::Phase(UsePhase::Block, side), Step::BlockPart]
        );
        assert_eq!(state.click_hands, [Hand::Main]);
        assert!(
            state
                .hands
                .as_ref()
                .unwrap()
                .iter()
                .all(|hand| *hand == Hand::Main)
        );
        assert_eq!(state.use_item_on_packets, usize::from(client));
        assert_eq!(state.use_item_packets, 0);
        assert_eq!(game.swings_started - before, 1);
        assert_eq!(game.world.block(0, 64, 0), lever);
    }

    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "one_click",
        Box::new(observer.clone()),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_server_game(|game| run(&observer, game, |g| assert!(use_block(g))));
        with_interaction_client_game(|game| {
            run(&observer, game, |g| {
                g.remote_use(mod_api::InteractionAttempt::Press);
            });
        });
    });
}

/// `SolidBucketItem.useOn`: a powder snow bucket placed on a block leaves an empty bucket in the
/// hand that placed it, on the server and in the client's own prediction, so the client's next
/// click in the same tick uses the bucket (`use_item`) as vanilla's does.
#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_placed_powder_snow_bucket_leaves_a_bucket_in_hand_on_both_sides() {
    fn run<'g, S: InteractionSide<'g>>(game: &mut S) {
        let client = S::IS_CLIENT;
        S::set_interaction_mode(game, sunrise_command::args::GameMode::Survival);
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        game.world.set(0, 64, 0, stone).unwrap();
        game.set_test_forced_hit(Some(hit()));
        let common = game.common_mut();
        common.inventory[common.selected] = Some(ItemStack::new("minecraft:powder_snow_bucket", 1));
        use_block(game);
        assert_eq!(
            game.inventory[game.selected].as_ref().map(|s| &*s.item),
            Some("minecraft:bucket"),
            "client={client}"
        );
    }

    mod_platform::testing::with_mods(Vec::new().into(), || {
        with_interaction_server_game(|game| run(game));
        with_interaction_client_game(|game| run(game));
    });
}

/// A spectator's click fires `right_click_block` on the client like anyone's, and a cancel
/// answers in place of the server's menu turn: SUCCESS swings and ends the click, FAIL ends it,
/// PASS goes on to the other hand.
#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_spectator_fires_right_click_block_on_the_client_and_its_cancel_answers() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "spectator_observer",
        Box::new(observer.clone()),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_client_game(|game| {
            game.set_local_mode(sunrise_command::args::GameMode::Spectator);
            assert!(game.spectator());
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(0, 64, 0, stone).unwrap();
            game.set_test_forced_hit(Some(hit()));
            // Nobody answers: the event fired, and the spectator's click ends there, no swing.
            *observer.0.lock().unwrap() = Controls::default();
            let swings = game.swings_started;
            assert!(game.remote_use_item_on(Hand::Main, mod_api::InteractionAttempt::Press, None));
            assert_eq!(observer.0.lock().unwrap().clicks, 1);
            assert_eq!(game.swings_started, swings);
            for (answer, ends, swings_by) in [
                (Answer::Success, true, 1),
                (Answer::Consume, true, 0),
                (Answer::Fail, true, 0),
                (Answer::Pass, false, 0),
            ] {
                *observer.0.lock().unwrap() = Controls {
                    click_answer: Some(answer),
                    ..Controls::default()
                };
                let swings = game.swings_started;
                assert_eq!(
                    game.remote_use_item_on(Hand::Main, mod_api::InteractionAttempt::Press, None),
                    ends,
                    "{answer:?}"
                );
                assert_eq!(game.swings_started - swings, swings_by, "{answer:?}");
                assert_eq!(observer.0.lock().unwrap().clicks, 1, "{answer:?}");
            }
        });
    });
}

/// A cancelled block PASS tries both hands, but neither may send or predict item use.
#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_spectators_passing_block_click_skips_item_use_for_both_hands() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "spectator_pass_observer",
        Box::new(observer.clone()),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_client_game(|game| {
            game.set_local_mode(sunrise_command::args::GameMode::Spectator);
            assert!(game.spectator());
            game.offhand = Some(ItemStack::new("minecraft:shield", 1));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(0, 64, 0, stone).unwrap();
            game.set_test_forced_hit(Some(hit()));
            *observer.0.lock().unwrap() = Controls {
                click_answer: Some(Answer::Pass),
                // Let an erroneous item event reach the Part, which would consume the click.
                part_answer: Some(Answer::Success),
                ..Controls::default()
            };
            let swings = game.swings_started;
            // The real target turn precedes remote_use_hands' item/off-hand sequence.
            assert!(!game.remote_use_item_on(Hand::Main, mod_api::InteractionAttempt::Press, None));
            game.remote_use_hands(Hand::Main, mod_api::InteractionAttempt::Press, None);
            let state = observer.0.lock().unwrap();
            assert_eq!(state.clicks, 2);
            assert_eq!(state.click_hands, [Hand::Main, Hand::Off]);
            assert_eq!(state.use_item_packets, 0);
            assert_eq!(
                state
                    .steps
                    .iter()
                    .filter(|s| matches!(s, Step::Air(_, Hand::Main)))
                    .count(),
                0
            );
            assert_eq!(
                state
                    .steps
                    .iter()
                    .filter(|s| matches!(s, Step::Air(_, Hand::Off)))
                    .count(),
                0
            );
            assert_eq!(
                state
                    .steps
                    .iter()
                    .filter(|s| matches!(s, Step::ItemPart))
                    .count(),
                0
            );
            assert_eq!(game.swings_started - swings, 0);
            assert!(game.using.is_none());
        });
    });
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn air_event_skips_the_item_and_consuming_results_end_the_click() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "air_observer",
        Box::new(observer.clone()),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    let answers = [
        (Answer::Success, true, 1, 0),
        (Answer::SuccessServer, true, 0, 1),
        (Answer::Consume, true, 0, 0),
        (Answer::Pass, false, 0, 0),
        (Answer::Fail, false, 0, 0),
        (Answer::TryWithEmptyHand, false, 0, 0),
    ];
    mod_platform::testing::with_mods(mods, || {
        with_interaction_server_game(|game| {
            let side = LogicalSide::Server;
            for (answer, consumes, _, server_swings) in answers {
                *observer.0.lock().unwrap() = Controls {
                    air_answer: Some(answer),
                    ..Controls::default()
                };
                game.swing = Default::default();
                let swings = game.swings_started;
                assert_eq!(game.use_hand(None).unwrap(), consumes, "{answer:?}");
                assert_eq!(
                    game.swings_started - swings,
                    server_swings,
                    "{side:?} {answer:?}"
                );
                assert_eq!(
                    observer.0.lock().unwrap().steps,
                    [Step::Air(side, Hand::Main)],
                    "{side:?} {answer:?}"
                );
            }
            *observer.0.lock().unwrap() = Controls::default();
            assert!(game.use_hand(None).unwrap());
            assert_eq!(
                observer.0.lock().unwrap().steps,
                [Step::Air(side, Hand::Main), Step::ItemPart]
            );
            *observer.0.lock().unwrap() = Controls::default();
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(0, 64, 0, stone).unwrap();
            game.set_test_forced_hit(Some(hit()));
            // A passing target permits the same hand's Item.use hook.
            assert!(
                game.right_click_item(Hand::Main, &ItemStack::new("minecraft:stick", 1))
                    .is_none()
            );
            assert_eq!(
                observer.0.lock().unwrap().steps,
                [Step::Air(side, Hand::Main)]
            );
            observer.0.lock().unwrap().steps.clear();
            game.set_test_forced_hit(None);
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(sunrise_sim::mobs::Mob::new(
                42,
                kind,
                Vec3::new(0.5, 64.0, 5.5),
                0.0,
            ));
            assert!(
                game.right_click_item(Hand::Main, &ItemStack::new("minecraft:stick", 1))
                    .is_none()
            );
            assert_eq!(
                observer.0.lock().unwrap().steps,
                [Step::Air(side, Hand::Main)]
            );
        });
        with_interaction_client_game(|game| {
            let side = LogicalSide::Client;
            game.offhand = Some(ItemStack::new("minecraft:shield", 1));
            for (answer, consumes, client_swings, _) in answers {
                *observer.0.lock().unwrap() = Controls {
                    air_answer: Some(answer),
                    ..Controls::default()
                };
                game.swing = Default::default();
                let swings = game.swings_started;
                let mut expected = vec![Step::Air(side, Hand::Main)];
                game.remote_use(mod_api::InteractionAttempt::Press);
                if !consumes {
                    expected.push(Step::Air(side, Hand::Off));
                }
                assert!(game.using.is_none(), "{answer:?}");
                assert_eq!(
                    game.swings_started - swings,
                    client_swings,
                    "{side:?} {answer:?}"
                );
                assert_eq!(
                    observer.0.lock().unwrap().steps,
                    expected,
                    "{side:?} {answer:?}"
                );
            }
            *observer.0.lock().unwrap() = Controls {
                air_answer: Some(Answer::Pass),
                air_main_only: true,
                part_pass: true,
                ..Controls::default()
            };
            game.remote_use(mod_api::InteractionAttempt::Press);
            assert!(
                game.using.is_some(),
                "passing main hand must allow the shield to rise"
            );
            assert_eq!(
                observer.0.lock().unwrap().steps,
                [
                    Step::Air(side, Hand::Main),
                    Step::Air(side, Hand::Off),
                    Step::ItemPart
                ]
            );
            *observer.0.lock().unwrap() = Controls::default();
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(0, 64, 0, stone).unwrap();
            game.set_test_forced_hit(Some(hit()));
            // A passing target permits the same hand's Item.use hook.
            assert!(
                game.right_click_item(Hand::Main, &ItemStack::new("minecraft:stick", 1))
                    .is_none()
            );
            assert_eq!(
                observer.0.lock().unwrap().steps,
                [Step::Air(side, Hand::Main)]
            );
            observer.0.lock().unwrap().steps.clear();
            game.set_test_forced_hit(None);
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.spawn_entity(42, &kind.id, [0.5, 64.0, 5.5], [0.0; 3]);
            game.refresh_client_pick(1.0);
            assert!(game.entity_targeted());
            assert!(
                game.right_click_item(Hand::Main, &ItemStack::new("minecraft:stick", 1))
                    .is_none()
            );
            assert_eq!(
                observer.0.lock().unwrap().steps,
                [Step::Air(side, Hand::Main)]
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn an_item_used_from_the_off_hand_reaches_the_use_hooks_on_the_server() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "off_hand_observer",
        Box::new(observer.clone()),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_server_game(|game| {
            let side = LogicalSide::Server;
            game.offhand = Some(ItemStack::new("minecraft:stick", 1));

            // An empty main hand passes, and the off hand uses its item.
            game.common.inventory[game.common.selected] = None;
            *observer.0.lock().unwrap() = Controls::default();
            assert_eq!(game.use_item_as_player().unwrap(), Some(Hand::Off));
            assert_eq!(
                observer.0.lock().unwrap().steps,
                [Step::Air(side, Hand::Off), Step::ItemPart]
            );

            // A main hand item nothing answers for passes too: both hands' hooks, in order.
            game.common.inventory[game.common.selected] =
                Some(ItemStack::new("minecraft:stone", 1));
            *observer.0.lock().unwrap() = Controls::default();
            assert_eq!(game.use_item_as_player().unwrap(), Some(Hand::Off));
            assert_eq!(
                observer.0.lock().unwrap().steps,
                [
                    Step::Air(side, Hand::Main),
                    Step::Air(side, Hand::Off),
                    Step::ItemPart
                ]
            );

            // A guest's use packet names the off hand alone.
            game.common.inventory[game.common.selected] = None;
            *observer.0.lock().unwrap() = Controls::default();
            game.server.guest_use = Some(sunrise_gameplay_common::game::GuestUse::Item(Hand::Off));
            assert_eq!(game.use_item_as_player().unwrap(), Some(Hand::Off));
            assert_eq!(
                observer.0.lock().unwrap().steps,
                [Step::Air(side, Hand::Off), Step::ItemPart]
            );

            // A block click: the off hand's `use_item_on_block` phases and its item Part's `use_on`,
            // for the local player and for a guest's use-on-block packet.
            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            game.world.set(0, 63, 0, stone).unwrap();
            game.world.set(0, 64, 0, stone).unwrap();
            game.set_test_forced_hit(Some(hit()));
            for guest in [
                None,
                Some(sunrise_gameplay_common::game::GuestUse::on_block(Hand::Off)),
            ] {
                game.common.inventory[game.common.selected] = None;
                *observer.0.lock().unwrap() = Controls {
                    hands: Some(Vec::new()),
                    ..Controls::default()
                };
                game.server.guest_use = guest;
                assert_eq!(game.use_item_as_player().unwrap(), Some(Hand::Off));
                let state = observer.0.lock().unwrap();
                assert!(
                    state
                        .hands
                        .as_ref()
                        .unwrap()
                        .iter()
                        .all(|h| *h == Hand::Off || guest.is_none())
                );
                assert_eq!(state.hands.as_ref().unwrap().last(), Some(&Hand::Off));
                assert_eq!(state.steps.last(), Some(&Step::UseOnPart(side)));
            }
        });
    });
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn before_block_cancellation_pass_continues_but_fail_and_consuming_results_stop() {
    fn run<'g, S: InteractionSide<'g>>(observer: &Observer, game: &mut S) {
        let client = S::IS_CLIENT;
        let side = S::LOGICAL_SIDE;
        let registry = game.pack.assets.registry;
        let stone = registry.default_state("minecraft:stone").unwrap();
        let lever = game
            .with_props(
                registry.default_state("minecraft:lever").unwrap(),
                &[("face", "floor")],
            )
            .unwrap();
        game.world.set(0, 63, 0, stone).unwrap();
        game.set_test_forced_hit(Some(hit()));
        // Reference rules: PASS reaches the block; FAIL and each
        // consuming result stop before it. Expected values are literal,
        // independent of InteractionResult::consumes_action.
        for (answer, reaches_block, swings) in [
            (Answer::Pass, true, 1),
            (Answer::Fail, false, 0),
            (Answer::TryWithEmptyHand, false, 0),
            (Answer::Success, false, 1),
            (Answer::SuccessServer, false, u32::from(!client)),
            (Answer::Consume, false, 0),
        ] {
            game.world.set(0, 64, 0, lever).unwrap();
            *observer.0.lock().unwrap() = Controls {
                record_first: true,
                first_answer: Some(answer),
                // Cancellation must skip this Part even for PASS.
                first_part_answer: Some(Answer::Fail),
                ..Controls::default()
            };
            let before = game.swings_started;
            assert_eq!(
                use_block(game),
                answer != Answer::TryWithEmptyHand,
                "{side:?}: {answer:?}"
            );
            assert_eq!(game.swings_started - before, swings);
            assert_eq!(game.world.block(0, 64, 0) != lever, reaches_block);
            let mut expected = vec![Step::Phase(UsePhase::ItemBeforeBlock, side)];
            if reaches_block {
                expected.push(Step::Phase(UsePhase::Block, side));
                expected.push(Step::BlockPart);
            }
            assert_eq!(observer.0.lock().unwrap().steps, expected);
        }
        // A block that passes also reaches the after-block item phase.
        game.world.set(0, 64, 0, stone).unwrap();
        *observer.0.lock().unwrap() = Controls {
            record_first: true,
            first_answer: Some(Answer::Pass),
            first_part_answer: Some(Answer::Fail),
            item_on_answer: Some(Answer::Consume),
            ..Controls::default()
        };
        assert!(use_block(game));
        assert_eq!(game.world.block(0, 64, 0), stone);
        let expected = vec![
            Step::Phase(UsePhase::ItemBeforeBlock, side),
            Step::Phase(UsePhase::Block, side),
            Step::BlockPart,
            Step::Phase(UsePhase::ItemAfterBlock, side),
        ];
        assert_eq!(observer.0.lock().unwrap().steps, expected);
    }

    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "before_block_observer",
        Box::new(observer.clone()),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_server_game(|game| run(&observer, game));
        with_interaction_client_game(|game| run(&observer, game));
    });
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn canceled_item_results_keep_the_stack_wrappers_side_effects() {
    fn run<'g, S: InteractionSide<'g>>(
        observer: &Observer,
        cooling_stack: &impl Fn(&str) -> ItemStack,
        game: &mut S,
    ) {
        let client = S::IS_CLIENT;
        S::set_interaction_mode(game, sunrise_command::args::GameMode::Survival);
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        game.world.set(0, 64, 0, stone).unwrap();
        game.set_test_forced_hit(Some(hit()));
        for answer in [
            Answer::Success,
            Answer::SuccessServer,
            Answer::Consume,
            Answer::Pass,
            Answer::Fail,
            Answer::TryWithEmptyHand,
        ] {
            game.cooldowns.remove("test:item_events");
            let common = game.common_mut();
            common.inventory[common.selected] = Some(cooling_stack("minecraft:milk_bucket"));
            *observer.0.lock().unwrap() = Controls {
                use_block: TriState::False,
                item_on_answer: Some(answer),
                consume_item: answer.consumes_action(),
                ..Controls::default()
            };
            let used = game.stats.get("used", "minecraft:milk_bucket");
            let _ = use_block(game);
            assert_eq!(
                game.stats.get("used", "minecraft:milk_bucket"),
                used + if !client && answer.consumes_action() {
                    1
                } else {
                    0
                }
            );
            assert_eq!(
                game.cooldowns.is_on_cooldown("test:item_events"),
                answer.consumes_action()
            );
            assert_eq!(
                game.held().unwrap().item.as_ref(),
                if !client && answer.consumes_action() {
                    "minecraft:bucket"
                } else {
                    "minecraft:milk_bucket"
                }
            );
        }
        game.set_test_forced_hit(None);
        S::interaction_clear_guest_use(game);
        for item in ["minecraft:stick", "minecraft:shield"] {
            for answer in [Answer::Success, Answer::SuccessServer, Answer::Consume] {
                game.cooldowns.remove("test:item_events");
                let common = game.common_mut();
                common.inventory[common.selected] = Some(cooling_stack(item));
                *observer.0.lock().unwrap() = Controls {
                    air_answer: Some(answer),
                    ..Controls::default()
                };
                let used = game.stats.get("used", item);
                let _ = S::interaction_use_air_hand(game, Hand::Main);
                // ItemStack.use applies components only to instant use,
                // and unlike useOn does not award the used statistic.
                assert_eq!(game.stats.get("used", item), used);
                assert_eq!(
                    game.cooldowns.is_on_cooldown("test:item_events"),
                    item == "minecraft:stick"
                );
                assert!(game.using.is_none());
            }
        }
    }

    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "wrapper_observer",
        Box::new(observer.clone()),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        let cooling_stack = |item: &str| {
            let mut stack = ItemStack::new(item, 1);
            stack.data_mut().set_other(
                "minecraft:use_cooldown",
                Some(Tag::Compound(
                    [
                        ("seconds".into(), Tag::Float(1.0_f32.to_bits())),
                        (
                            "cooldown_group".into(),
                            Tag::String("test:item_events".into()),
                        ),
                    ]
                    .into(),
                )),
            );
            stack
        };
        with_interaction_server_game(|game| run(&observer, &cooling_stack, game));
        with_interaction_client_game(|game| run(&observer, &cooling_stack, game));
    });
}

#[test]
#[ignore = "daily"]
fn item_use_overlap_matrix_on_the_server() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "overlap",
        Box::new(observer.clone()),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_server_game(|game| {
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(0, 64, 0, stone).unwrap();
            game.set_test_forced_hit(Some(hit()));
            Arc::make_mut(game.world.chunks.get_mut(&(0, 0)).unwrap()).lit = true;
            item_used_on_block_tests::install_criteria(game, &[0], "minecraft:air");
            for hand in [Hand::Main, Hand::Off] {
                for held in [false, true] {
                    for sneak in [false, true] {
                        for gate in [TriState::True, TriState::Default, TriState::False] {
                            for first in [Answer::Pass, Answer::TryWithEmptyHand, Answer::Consume] {
                                for after in
                                    [Answer::Success, Answer::Fail, Answer::Pass, Answer::Consume]
                                {
                                    game.common.inventory[game.common.selected] = None;
                                    game.offhand = None;
                                    if held {
                                        let stack = Some(ItemStack::new("minecraft:stick", 1));
                                        if hand == Hand::Main {
                                            game.common.inventory[game.common.selected] = stack;
                                        } else {
                                            game.offhand = stack;
                                        }
                                    }
                                    game.server_set_acting_sneak(sneak);
                                    *observer.0.lock().unwrap() = Controls {
                                        expected_hand: Some(hand),
                                        record_first: true,
                                        first_answer: Some(first),
                                        block_answer: Some(Answer::Pass),
                                        item_on_answer: Some(after),
                                        use_item: gate,
                                        expect_empty_item_phase: !held,
                                        ..Controls::default()
                                    };
                                    let stopped =
                                        held && gate != TriState::False && first != Answer::Pass;
                                    let item_phase = !stopped
                                        && gate != TriState::False
                                        && (held || gate == TriState::True);
                                    let expected = if stopped {
                                        first == Answer::Consume
                                    } else {
                                        item_phase
                                            && matches!(
                                                after,
                                                Answer::Success | Answer::Fail | Answer::Consume
                                            )
                                    };
                                    game.server.guest_use = Some(
                                        sunrise_gameplay_common::game::GuestUse::on_block(hand),
                                    );
                                    game.adv.progress = Default::default();
                                    let swings = game.swings_started;
                                    let used =
                                        game.with_hand(hand, |g| g.use_hand(Some(&hit())).unwrap());
                                    assert_eq!(
                                        used, expected,
                                        "{hand:?} held={held} sneak={sneak} {gate:?} {first:?} {after:?}"
                                    );
                                    assert_eq!(
                                        game.swings_started - swings,
                                        u32::from(item_phase && after == Answer::Success)
                                    );
                                    assert_eq!(
                                        game.adv.progress.has("test:sticks0", "seen"),
                                        item_phase
                                            && matches!(after, Answer::Success | Answer::Consume)
                                    );
                                    let state = observer.0.lock().unwrap();
                                    assert_eq!(
                                        state.steps.contains(&Step::Phase(
                                            UsePhase::ItemAfterBlock,
                                            LogicalSide::Server
                                        )),
                                        item_phase
                                    );
                                    assert_eq!(
                                        state.steps.contains(&Step::Phase(
                                            UsePhase::Block,
                                            LogicalSide::Server
                                        )),
                                        !stopped && !(sneak && held)
                                    );
                                }
                            }
                        }
                    }
                }
            }
        });
    });
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn on_item_use_first_runs_before_the_block_and_ends_the_click() {
    fn run<'g, S: InteractionSide<'g>>(observer: &Observer, game: &mut S) {
        let client = S::IS_CLIENT;
        let side = S::LOGICAL_SIDE;
        let registry = game.pack.assets.registry;
        let stone = registry.default_state("minecraft:stone").unwrap();
        let lever = game
            .with_props(
                registry.default_state("minecraft:lever").unwrap(),
                &[("face", "floor")],
            )
            .unwrap();
        game.world.set(0, 63, 0, stone).unwrap();
        game.world.set(0, 64, 0, lever).unwrap();
        game.set_test_forced_hit(Some(hit()));

        // Every non-PASS Part answer stops before the block, including TRY.
        for (answer, used, swings) in [
            (Answer::Success, true, 1),
            (Answer::SuccessServer, true, u32::from(!client)),
            (Answer::Consume, true, 0),
            (Answer::Fail, true, 0),
            (Answer::TryWithEmptyHand, false, 0),
        ] {
            *observer.0.lock().unwrap() = Controls {
                record_first: true,
                first_part_answer: Some(answer),
                ..Controls::default()
            };
            let before = game.swings_started;
            assert_eq!(use_block(game), used);
            assert_eq!(game.swings_started - before, swings);
            assert_eq!(game.world.block(0, 64, 0), lever);
            let first = Step::Phase(UsePhase::ItemBeforeBlock, side);
            let seen = observer.0.lock().unwrap();
            assert_eq!(seen.steps, [first, Step::FirstPart]);
            assert_eq!(seen.use_item_on_packets, usize::from(client));
        }

        // The broadcast event goes first: its cancel stops the Part.
        *observer.0.lock().unwrap() = Controls {
            record_first: true,
            first_answer: Some(Answer::Success),
            first_part_answer: Some(Answer::Success),
            ..Controls::default()
        };
        assert!(use_block(game));
        assert_eq!(game.world.block(0, 64, 0), lever);
        assert_eq!(
            observer.0.lock().unwrap().steps,
            [Step::Phase(UsePhase::ItemBeforeBlock, side)]
        );

        // A pass goes on: the block's phase and the lever flip.
        *observer.0.lock().unwrap() = Controls {
            record_first: true,
            first_part_answer: Some(Answer::Pass),
            ..Controls::default()
        };
        assert!(use_block(game));
        assert_ne!(game.world.block(0, 64, 0), lever);
        assert_eq!(
            observer.0.lock().unwrap().steps[..3],
            [
                Step::Phase(UsePhase::ItemBeforeBlock, side),
                Step::FirstPart,
                Step::Phase(UsePhase::Block, side),
            ]
        );
    }

    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "first_observer",
        Box::new(observer.clone()),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_server_game(|game| run(&observer, game));
        with_interaction_client_game(|game| run(&observer, game));
    });
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn phase_events_replace_the_behaviour_before_its_part_on_both_sides() {
    fn run<'g, S: InteractionSide<'g>>(observer: &Observer, game: &mut S) {
        let client = S::IS_CLIENT;
        let side = S::LOGICAL_SIDE;
        let registry = game.pack.assets.registry;
        let stone = registry.default_state("minecraft:stone").unwrap();
        let lever = game
            .with_props(
                registry.default_state("minecraft:lever").unwrap(),
                &[("face", "floor")],
            )
            .unwrap();
        game.world.set(0, 63, 0, stone).unwrap();
        game.world.set(0, 64, 0, lever).unwrap();
        game.set_test_forced_hit(Some(hit()));
        *observer.0.lock().unwrap() = Controls::default();
        assert!(use_block(game));
        assert_ne!(game.world.block(0, 64, 0), lever);
        let expected = vec![Step::Phase(UsePhase::Block, side), Step::BlockPart];
        assert_eq!(observer.0.lock().unwrap().steps, expected);

        for answer in [Answer::Success, Answer::SuccessServer, Answer::Consume] {
            game.world.set(0, 64, 0, lever).unwrap();
            *observer.0.lock().unwrap() = Controls {
                block_answer: Some(answer),
                ..Controls::default()
            };
            let swings = game.swings_started;
            assert!(use_block(game));
            assert_eq!(game.world.block(0, 64, 0), lever);
            assert_eq!(
                observer.0.lock().unwrap().steps,
                [Step::Phase(UsePhase::Block, side)]
            );
            let swings = game.swings_started - swings;
            assert_eq!(
                swings,
                u32::from(
                    answer == Answer::Success || (!client && answer == Answer::SuccessServer)
                )
            );
        }

        // TRY skips the item method/Part, but still delegates the
        // block's empty method on the main hand, on both sides.
        *observer.0.lock().unwrap() = Controls {
            block_answer: Some(Answer::TryWithEmptyHand),
            ..Controls::default()
        };
        assert!(use_block(game));
        assert_ne!(game.world.block(0, 64, 0), lever);
        assert_eq!(
            observer.0.lock().unwrap().steps,
            [Step::Phase(UsePhase::Block, side)]
        );

        for answer in [Answer::Pass, Answer::Fail] {
            game.world.set(0, 64, 0, lever).unwrap();
            *observer.0.lock().unwrap() = Controls {
                block_answer: Some(answer),
                item_on_answer: Some(Answer::Consume),
                ..Controls::default()
            };
            assert!(use_block(game));
            assert_eq!(game.world.block(0, 64, 0), lever);
            assert_eq!(
                observer.0.lock().unwrap().steps,
                [
                    Step::Phase(UsePhase::Block, side),
                    Step::Phase(UsePhase::ItemAfterBlock, side),
                ]
            );
        }

        // A passing block gets the item phase, before the item Part.
        game.world.set(0, 64, 0, stone).unwrap();
        *observer.0.lock().unwrap() = Controls::default();
        let _ = use_block(game);
        let mut expected = vec![
            Step::Phase(UsePhase::Block, side),
            Step::BlockPart,
            Step::Phase(UsePhase::ItemAfterBlock, side),
        ];
        if !client {
            expected.push(Step::UseOnPart(LogicalSide::Server));
        }
        assert_eq!(observer.0.lock().unwrap().steps, expected);

        for answer in [
            Answer::Success,
            Answer::SuccessServer,
            Answer::Consume,
            Answer::Pass,
            Answer::Fail,
            Answer::TryWithEmptyHand,
        ] {
            *observer.0.lock().unwrap() = Controls {
                item_on_answer: Some(answer),
                ..Controls::default()
            };
            let swings = game.swings_started;
            assert_eq!(
                use_block(game),
                answer.consumes_action() || answer == Answer::Fail
            );
            let expected = vec![
                Step::Phase(UsePhase::Block, side),
                Step::BlockPart,
                Step::Phase(UsePhase::ItemAfterBlock, side),
            ];
            assert_eq!(observer.0.lock().unwrap().steps, expected);
            assert_eq!(
                game.swings_started - swings,
                u32::from(
                    answer == Answer::Success || (!client && answer == Answer::SuccessServer)
                )
            );
        }

        // The phase event is later than RightClickBlock's choices.
        *observer.0.lock().unwrap() = Controls {
            use_block: TriState::False,
            use_item: TriState::False,
            ..Controls::default()
        };
        assert!(!use_block(game));
        assert!(observer.0.lock().unwrap().steps.is_empty());

        // ItemStack.useOn passes before Item.useOn in adventure mode.
        S::set_interaction_mode(game, sunrise_command::args::GameMode::Adventure);
        *observer.0.lock().unwrap() = Controls {
            use_block: TriState::False,
            ..Controls::default()
        };
        assert!(!use_block(game));
        assert!(observer.0.lock().unwrap().steps.is_empty());
        let selected = game.selected;
        game.inventory[selected]
            .as_mut()
            .unwrap()
            .data_mut()
            .set_other(
                "minecraft:can_place_on",
                Some(Tag::Compound(
                    [("blocks".into(), Tag::String("minecraft:stone".into()))].into(),
                )),
            );
        *observer.0.lock().unwrap() = Controls {
            use_block: TriState::False,
            item_on_answer: Some(Answer::Success),
            ..Controls::default()
        };
        assert!(use_block(game));
        assert_eq!(
            observer.0.lock().unwrap().steps,
            [Step::Phase(UsePhase::ItemAfterBlock, side)]
        );
        S::set_interaction_mode(game, sunrise_command::args::GameMode::Creative);

        // In 26.3 dousing is CampfireBlock.useItemOn, not Item.useOn:
        // a canceled BLOCK cannot predict it or reach ITEM_AFTER_BLOCK.
        let campfire = registry.default_state("minecraft:campfire").unwrap();
        game.world.set(0, 64, 0, campfire).unwrap();
        let common = game.common_mut();
        common.inventory[common.selected] = Some(ItemStack::new("minecraft:iron_shovel", 1));
        *observer.0.lock().unwrap() = Controls {
            block_answer: Some(Answer::Success),
            ..Controls::default()
        };
        assert!(use_block(game));
        assert_eq!(game.world.block(0, 64, 0), campfire);
        assert_eq!(
            observer.0.lock().unwrap().steps,
            [Step::Phase(UsePhase::Block, side)]
        );
        *observer.0.lock().unwrap() = Controls::default();
        assert!(use_block(game));
        assert_ne!(game.world.block(0, 64, 0), campfire);
        let expected = vec![Step::Phase(UsePhase::Block, side), Step::BlockPart];
        assert_eq!(observer.0.lock().unwrap().steps, expected);

        game.offhand = Some(ItemStack::new("minecraft:stick", 1));
        *observer.0.lock().unwrap() = Controls {
            block_answer: Some(Answer::Success),
            expected_hand: Some(Hand::Off),
            ..Controls::default()
        };
        assert!(S::interaction_use_block_hand(game, Hand::Off));
        assert_eq!(
            observer.0.lock().unwrap().steps,
            [Step::Phase(UsePhase::Block, side)]
        );
    }

    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "phase_observer",
        Box::new(observer.clone()),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_server_game(|game| run(&observer, game));
        with_interaction_client_game(|game| run(&observer, game));
    });
}

/// Item.useOn without item behavior or a BLOCK_TRANSFORMER is PASS, and
/// MultiPlayerGameMode.performUseItemOn returns it without a swing. An unknown
/// item must keep that inert fallback even when RightClickBlock forces use.
#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn resource_gated_unknown_held_item_forced_use_predicts_pass_without_a_swing() {
    fn run<'g, S: InteractionSide<'g>>(observer: &Observer, game: &mut S) {
        let client = S::IS_CLIENT;
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        game.world.set(0, 64, 0, stone).unwrap();
        game.set_test_forced_hit(Some(hit()));
        // Keep the API key resolvable so RightClickBlock really runs,
        // but remove its gameplay data to exercise the unknown-item
        // fallback. The suffix also catches premature spawn-egg prediction.
        let stack = ItemStack::new("minecraft:pig_spawn_egg", 2);
        assert!(mod_api::__private::item_key(&stack.item).is_some());
        let mut items = game.items().clone();
        assert!(items.items.remove(&stack.item).is_some());
        game.server_items = Some(Arc::new(items));
        assert!(game.items().get(&stack.item).is_none());
        let common = game.common_mut();
        common.inventory[common.selected] = Some(stack.clone());
        game.offhand = None;
        *observer.0.lock().unwrap() = Controls {
            use_block: TriState::False,
            use_item: TriState::True,
            first_answer: Some(Answer::Pass),
            item_on_answer: Some(Answer::Success),
            ..Controls::default()
        };
        let swings = game.swings_started;
        // Literal vanilla fallback expectations, independently applied
        // to client prediction and authoritative server use (not one
        // result compared with the other).
        assert!(!use_block(game), "client={client}: PASS");
        assert_eq!(game.swings_started - swings, 0, "client={client}");
        assert_eq!(game.held(), Some(&stack));
        assert_eq!(game.world.block(0, 64, 0), stone);
        assert!(game.using.is_none());
        let state = observer.0.lock().unwrap();
        assert_eq!(state.clicks, 1, "the forcing listener must have run");
        assert!(
            !state
                .steps
                .iter()
                .any(|step| matches!(step, Step::Phase(UsePhase::ItemAfterBlock, _))),
            "unknown items cannot reach the forced item's success listener"
        );
    }

    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "unknown_forced_use",
        Box::new(observer.clone()),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_client_game(|game| run(&observer, game));
        with_interaction_server_game(|game| run(&observer, game));
    });
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn right_click_block_explicit_allow_overrides_sneak_skip_on_both_sides() {
    fn run<'g, S: InteractionSide<'g>>(observer: &Observer, game: &mut S) {
        let side = S::LOGICAL_SIDE;
        let registry = game.pack.assets.registry;
        let stone = registry.default_state("minecraft:stone").unwrap();
        let lever = game
            .with_props(
                registry.default_state("minecraft:lever").unwrap(),
                &[("face", "floor")],
            )
            .unwrap();
        game.world.set(0, 63, 0, stone).unwrap();
        game.set_test_forced_hit(Some(hit()));
        S::interaction_set_sneak(game, true);
        // Either hand being full activates the vanilla secondary-use
        // skip. TRUE overrides it; Default and FALSE skip the lever.
        for item_in_main in [true, false] {
            let common = game.common_mut();
            common.inventory[common.selected] =
                item_in_main.then(|| ItemStack::new("minecraft:stick", 1));
            game.offhand = (!item_in_main).then(|| ItemStack::new("minecraft:stick", 1));
            for (switch, used) in [
                (TriState::True, true),
                (TriState::Default, false),
                (TriState::False, false),
            ] {
                game.world.set(0, 64, 0, lever).unwrap();
                *observer.0.lock().unwrap() = Controls {
                    use_block: switch,
                    use_item: TriState::False,
                    ..Controls::default()
                };
                assert_eq!(use_block(game), used);
                assert_eq!(game.world.block(0, 64, 0) != lever, used);
                let expected = if used {
                    vec![Step::Phase(UsePhase::Block, side), Step::BlockPart]
                } else {
                    vec![]
                };
                assert_eq!(observer.0.lock().unwrap().steps, expected);
            }
        }
    }

    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "sneak_observer",
        Box::new(observer.clone()),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_server_game(|game| run(&observer, game));
        with_interaction_client_game(|game| run(&observer, game));
    });
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn right_click_block_forced_item_use_reaches_empty_hand_on_both_sides() {
    fn run<'g, S: InteractionSide<'g>>(observer: &Observer, game: &mut S) {
        let side = S::LOGICAL_SIDE;
        let common = game.common_mut();
        common.inventory[common.selected] = None;
        game.set_test_forced_hit(Some(hit()));
        // RightClickBlock's TRUE enters useOn even for an empty stack;
        // Default and FALSE do not. A phase listener supplies the use.
        for (use_item, used) in [
            (TriState::True, true),
            (TriState::Default, false),
            (TriState::False, false),
        ] {
            *observer.0.lock().unwrap() = Controls {
                use_block: TriState::False,
                use_item,
                item_on_answer: Some(Answer::Consume),
                expect_empty_item_phase: true,
                ..Controls::default()
            };
            assert_eq!(use_block(game), used);
            let expected = if used {
                vec![Step::Phase(UsePhase::ItemAfterBlock, side)]
            } else {
                vec![]
            };
            assert_eq!(observer.0.lock().unwrap().steps, expected);
            assert!(game.inventory[game.selected].is_none());
        }
        // Forcing an empty use without a phase answer still passes:
        // there is no stack-specific Part or vanilla item to invoke.
        *observer.0.lock().unwrap() = Controls {
            use_block: TriState::False,
            use_item: TriState::True,
            expect_empty_item_phase: true,
            ..Controls::default()
        };
        assert!(!use_block(game));
        assert_eq!(
            observer.0.lock().unwrap().steps,
            [Step::Phase(UsePhase::ItemAfterBlock, side)]
        );
        // TRUE bypasses the stack/cooldown gate, not adventure's
        // placement predicate (which an empty hand cannot satisfy).
        S::set_interaction_mode(game, sunrise_command::args::GameMode::Adventure);
        *observer.0.lock().unwrap() = Controls {
            use_block: TriState::False,
            use_item: TriState::True,
            item_on_answer: Some(Answer::Consume),
            ..Controls::default()
        };
        assert!(!use_block(game));
        assert!(observer.0.lock().unwrap().steps.is_empty());
    }

    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "empty_hand_observer",
        Box::new(observer.clone()),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_server_game(|game| run(&observer, game));
        with_interaction_client_game(|game| run(&observer, game));
    });
}

pub(crate) fn setup_interaction_guest(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
) -> std::sync::mpsc::Receiver<sunrise_sim::lan::Out> {
    game.lan_host(
        LanSettings {
            port: 0,
            guest_commands: false,
            force_game_mode: false,
        },
        NetConfig {
            listen: false,
            announce: false,
            lod_distance: 0,
            ..NetConfig::lan(0, String::new(), 2)
        },
        HubConfig {
            default_game_mode: 1,
            guest_commands: false,
            force_game_mode: false,
            hardcore: false,
            view_distance: 2,
            simulation_distance: 2,
            dimension: game.dim.id().to_owned(),
            allow_flight: true,
            seed: 0,
            max_players: 2,
            integrated: true,
            levels: game.server.level_shapes.clone(),
            level_seeds: game.server.levels.0.runtime.seeds(),
        },
    )
    .unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut guest = Guest::new(
        Joined {
            conn: 12,
            uuid: 12,
            name: "interaction_guest".into(),
            properties: Vec::new(),
            info: ClientInformation::new(8),
            brand: String::new(),
            sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:1".parse().unwrap(),
            expected: false,
        },
        12,
        [0.5, 64.0, 3.5],
        10,
    );
    guest.placed = true;
    guest.game_mode = 1;
    game.server.lan.as_mut().unwrap().hub.guests.push(guest);
    rx
}

impl<'g> InteractionSide<'g> for sunrise_gameplay::game::ServerGame<'g> {
    #[cfg(feature = "client")]
    const IS_CLIENT: bool = false;
    #[cfg(feature = "client")]
    const LOGICAL_SIDE: LogicalSide = LogicalSide::Server;
    #[cfg(feature = "client")]
    fn set_interaction_mode(game: &mut Self, mode: sunrise_command::args::GameMode) {
        use sunrise_command::args::GameMode;
        let id = match mode {
            GameMode::Survival => 0,
            GameMode::Creative => 1,
            GameMode::Adventure => 2,
            GameMode::Spectator => 3,
        };
        game.set_mode_id(id);
    }
    fn interaction_use_block_hand(game: &mut Self, hand: Hand) -> bool {
        game.server.guest_use = Some(sunrise_gameplay_common::game::GuestUse::on_block(hand));
        game.with_hand(hand, |g| g.use_hand(Some(&hit()))).unwrap()
    }
    #[cfg(feature = "client")]
    fn interaction_use_air_hand(game: &mut Self, hand: Hand) -> bool {
        game.with_hand(hand, |g| g.use_hand(None)).unwrap()
    }
    #[cfg(feature = "client")]
    fn interaction_use_item_as_player(game: &mut Self) -> bool {
        game.use_item_as_player().unwrap().is_some()
    }
    #[cfg(feature = "client")]
    fn interaction_set_sneak(game: &mut Self, sneak: bool) {
        game.server_set_acting_sneak(sneak);
    }
    #[cfg(feature = "client")]
    fn interaction_clear_guest_use(game: &mut Self) {
        game.server.guest_use = None;
    }
    #[cfg(feature = "client")]
    fn interaction_reset_use_failed(game: &mut Self) {
        game.server.use_failed = false;
    }
}

#[cfg(feature = "client")]
use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;
