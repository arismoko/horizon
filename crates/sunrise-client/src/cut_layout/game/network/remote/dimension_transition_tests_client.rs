#[test]
#[ignore = "daily"]
fn a_failed_presentation_discards_its_draws_without_disabling_its_factory() {
    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    use mod_api::{
        __private::game::RegistrationContextNew, CameraType, ClientHost, DimensionTransition,
        DimensionTransitionScreen, DrawContext, SoundPlay, SoundRef, SoundState,
    };
    use std::sync::{
        OnceLock,
        atomic::{AtomicUsize, Ordering},
    };
    use sunrise_client::{
        hud::HudBatch,
        level_loading::{self, Reason},
    };
    use sunrise_client_gameplay::game::client_side::remote_client::session_state::RemoteDimensionEntry;
    use sunrise_protocol::packets::play;

    const NETHER: &str = "minecraft:the_nether";
    static CREATES: AtomicUsize = AtomicUsize::new(0);
    static DRAWS: AtomicUsize = AtomicUsize::new(0);
    static FIRST_SOUND: OnceLock<SoundRef> = OnceLock::new();

    struct Arrival;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Arrival {
        fn register_dimension_transition_screen(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut mod_api::RegisterDimensionTransitionScreenEvent<'_>,
        ) {
            event.screens.push(DimensionTransitionScreen::new(
                DimensionTransition::Incoming(NETHER.into()),
                |_, _| {
                    CREATES.fetch_add(1, Ordering::Relaxed);
                    Box::new(|host: &dyn ClientHost, _, _, draw: &mut dyn DrawContext| {
                        draw.fill([0.0, 0.0, 80.0, 10.0], 0xFFFF_0000);
                        host.apply_screen_blur();
                        assert!(host.clear_particles());
                        host.set_camera_type(CameraType::ThirdPersonFront);
                        let frame = DRAWS.fetch_add(1, Ordering::Relaxed);
                        if frame == 0 {
                            let sound = host
                                .play_local_sound(&SoundPlay::ui("minecraft:ui.button.click"), 0)
                                .unwrap();
                            assert!(FIRST_SOUND.set(sound).is_ok());
                            return;
                        }
                        assert!(FIRST_SOUND.get().unwrap().stop(host));
                        assert!(
                            host.play_local_sound(
                                &SoundPlay::ui("minecraft:entity.player.levelup"),
                                0
                            )
                            .is_some()
                        );
                        panic!("test draw panic");
                    })
                },
            ));
        }
    }

    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::client_side::remote_client::tests::dimension_transition_tests::a_failed_presentation_discards_its_draws_without_disabling_its_factory",
        || {
            mod_platform::testing::with_mods(
                vec![mod_api::ModEntry::new(
                    "failed_transition",
                    Box::new(Arrival),
                )]
                .into(),
                || {
                    let mut registrations = Vec::new();
                    sunrise_gameplay_common::game::native_hooks::native_register_dimension_transition_screen(
                        &mod_api::RegistrationContext::new(),
                        &mut registrations,
                    );
                    sunrise_client_render::dimension_transition::register(registrations);
                    with_client_game(|game| {
                        let (client, _server) = sunrise_protocol::connection::Pipe::pair();
                        game.connect_local(client, "TransitionTest").unwrap();
                        let spawn = |dimension: &str| play::SpawnInfo {
                            dimension_type: 0,
                            dimension: dimension.into(),
                            seed: 0,
                            game_type: 0,
                            previous_game_type: None,
                            debug: false,
                            flat: true,
                            last_death: None,
                            portal_cooldown: 0,
                            sea_level: 63,
                        };
                        let overworld = spawn("minecraft:overworld");
                        let nether = spawn(NETHER);
                        game.client.settings.menu_blur = 5;
                        game.enter_remote_dimension(
                            &overworld,
                            RemoteDimensionEntry::Login,
                            Reason::Other,
                        );
                        for trip in [1, 2] {
                            game.client.remote.as_mut().unwrap().level_load.finish();
                            game.enter_remote_dimension(
                                &nether,
                                RemoteDimensionEntry::Respawn,
                                Reason::NetherPortal,
                            );
                            assert_eq!(CREATES.load(Ordering::Relaxed), trip);

                            let mut vanilla = HudBatch::scratch([320, 240]);
                            game.draw_level_loading_background(&mut vanilla, Reason::NetherPortal);
                            level_loading::draw(&mut vanilla, &game.client.gui.atlas, None, None);
                            assert!(!vanilla.vertices.is_empty());
                            if trip == 1 {
                                assert_eq!(game.menu_blur(), None);
                                let mut frame = HudBatch::scratch([320, 240]);
                                assert!(game.remote_overlay(&mut frame));
                                assert_eq!(game.menu_blur(), Some(5.0));
                                assert!(game.client.mod_clear_particles.get());
                                assert_eq!(
                                    game.client.requested_camera_type.get(),
                                    Some(sunrise_level::camera_math::CameraType::ThirdPersonFront)
                                );
                            }
                            let sound = FIRST_SOUND.get().unwrap();
                            // Keep requests already pending on entry, including the sound from
                            // the successful draw. Discard the failed draw's requests only.
                            for incoming in [false, true, false] {
                                let camera = incoming.then_some(
                                    sunrise_level::camera_math::CameraType::ThirdPersonBack,
                                );
                                game.client.screen_blur_requested.set(incoming);
                                game.client.mod_clear_particles.set(incoming);
                                game.client.requested_camera_type.set(camera);
                                let mut frame = HudBatch::scratch([320, 240]);
                                assert!(game.remote_overlay(&mut frame));
                                assert_eq!(
                                    bytemuck::cast_slice::<_, u8>(&frame.vertices),
                                    bytemuck::cast_slice::<_, u8>(&vanilla.vertices)
                                );
                                assert_eq!(frame.blends, vanilla.blends);
                                assert_eq!(game.menu_blur(), incoming.then_some(5.0));
                                assert_eq!(game.client.mod_clear_particles.get(), incoming);
                                assert_eq!(game.client.requested_camera_type.get(), camera);
                                let pending = game.client.mod_sounds.queued.borrow();
                                if trip == 1 {
                                    assert_eq!(pending.len(), 1);
                                    assert_eq!(
                                        pending[0].1.identifier,
                                        "minecraft:ui.button.click"
                                    );
                                    assert_eq!(sound.state(game), Some(SoundState::Waiting));
                                } else {
                                    assert!(pending.is_empty());
                                }
                            }
                            if trip == 1 {
                                game.client.audio.0.heard.clear();
                                game.tick_sound_engine();
                                assert_eq!(
                                    game.client.audio.0.heard.get("minecraft:ui.button.click"),
                                    Some(&1)
                                );
                                assert!(
                                    !game
                                        .client
                                        .audio
                                        .0
                                        .heard
                                        .contains_key("minecraft:entity.player.levelup")
                                );
                            }
                            assert_eq!(DRAWS.load(Ordering::Relaxed), 2);
                            game.client.remote.as_mut().unwrap().level_load.finish();
                            game.enter_remote_dimension(
                                &overworld,
                                RemoteDimensionEntry::Respawn,
                                Reason::NetherPortal,
                            );
                        }
                    });
                },
            );
        },
    );
}
