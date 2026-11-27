#[test]
#[ignore = "daily"]
fn dimension_transition_screen_is_selected_at_entry_drawn_and_dropped() {
    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    use mod_api::__private::game::RegistrationContextNew;
    use mod_api::{
        ClientHost, DimensionTransition, DimensionTransitionReason as ApiReason,
        DimensionTransitionScreen, DrawContext, content::crash::current_native_mod,
    };
    use std::sync::Mutex;
    use sunrise_client::{hud::HudBatch, level_loading::Reason};
    use sunrise_client_gameplay::game::client_side::remote_client::session_state::RemoteDimensionEntry;
    use sunrise_gameplay_common::game::Screen;
    use sunrise_protocol::packets::play::{self, Clientbound as C};

    const OWNER: &str = "transition_test";
    const A: &str = "transition_test:a";
    const B: &str = "transition_test:b";
    const END: &str = "minecraft:the_end";

    #[derive(Debug, PartialEq)]
    enum Call {
        Create(&'static str, ApiReason),
        Draw(ApiReason, ApiReason, Option<f32>),
    }
    static CALLS: Mutex<Vec<Call>> = Mutex::new(Vec::new());

    struct Arrival;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Arrival {
        fn register_dimension_transition_screen(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut mod_api::RegisterDimensionTransitionScreenEvent<'_>,
        ) {
            for (id, transition) in [
                ("marker:login", DimensionTransition::Incoming(A.into())),
                ("marker:death", DimensionTransition::Incoming(B.into())),
                ("marker:end", DimensionTransition::Incoming(END.into())),
                (
                    "marker:travel",
                    DimensionTransition::Conditional {
                        to: B.into(),
                        from: A.into(),
                    },
                ),
            ] {
                event.screens.push(DimensionTransitionScreen::new(
                    transition,
                    move |_, initial_reason| {
                        assert_eq!(current_native_mod(), Some(OWNER));
                        CALLS.lock().unwrap().push(Call::Create(id, initial_reason));
                        Box::new(
                            move |_: &dyn ClientHost,
                                  reason,
                                  progress,
                                  draw: &mut dyn DrawContext| {
                                assert_eq!(current_native_mod(), Some(OWNER));
                                CALLS.lock().unwrap().push(Call::Draw(
                                    initial_reason,
                                    reason,
                                    progress,
                                ));
                                draw.fill([0.0, 0.0, 80.0, 10.0], 0xFF102030);
                            },
                        )
                    },
                ));
            }
        }
    }
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::remote::tests::dimension_transition_tests::dimension_transition_screen_is_selected_at_entry_drawn_and_dropped",
        || {
            mod_platform::testing::with_mods(
                vec![mod_api::ModEntry::new(OWNER, Box::new(Arrival))].into(),
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
                        let respawn = |dimension, keep| {
                            C::Respawn(play::Respawn {
                                spawn: spawn(dimension),
                                keep,
                            })
                        };
                        game.enter_remote_dimension(
                            &spawn(A),
                            RemoteDimensionEntry::Login,
                            Reason::Other,
                        );
                        let tracker = &mut game.client.remote.as_mut().unwrap().level_load;
                        assert!(
                            tracker.transition_screen().is_none(),
                            "login skips incoming A"
                        );
                        tracker.finish();

                        game.set_screen(Screen::Dead);
                        game.handle_packet(respawn(B, 0)).unwrap();
                        let tracker = &game.client.remote.as_ref().unwrap().level_load;
                        assert_eq!(tracker.reason, Reason::Other);
                        assert!(
                            tracker.transition_screen().is_none(),
                            "death skips incoming B and conditional A to B"
                        );

                        game.handle_packet(respawn(A, 3)).unwrap();
                        let tracker = &mut game.client.remote.as_mut().unwrap().level_load;
                        assert!(
                            tracker.transition_screen().is_none(),
                            "an open vanilla screen also stays in place"
                        );
                        assert!(CALLS.lock().unwrap().is_empty());
                        tracker.finish();

                        game.handle_packet(respawn(B, 3)).unwrap();
                        let tracker = &game.client.remote.as_ref().unwrap().level_load;
                        let mut batch = HudBatch::scratch([320, 240]);
                        assert!(game.draw_dimension_transition_screen(
                            &mut batch,
                            tracker,
                            Some(0.25)
                        ));
                        assert!(!batch.vertices.is_empty());

                        game.handle_packet(respawn(END, 3)).unwrap();
                        let tracker = &game.client.remote.as_ref().unwrap().level_load;
                        let mut updated = HudBatch::scratch([320, 240]);
                        assert!(game.draw_dimension_transition_screen(
                            &mut updated,
                            tracker,
                            Some(0.75)
                        ));
                        assert!(!updated.vertices.is_empty());
                        assert_eq!(
                            *CALLS.lock().unwrap(),
                            [
                                Call::Create("marker:travel", ApiReason::Other),
                                Call::Draw(ApiReason::Other, ApiReason::Other, Some(0.25)),
                                Call::Draw(ApiReason::Other, ApiReason::EndPortal, Some(0.75)),
                            ]
                        );

                        game.client.remote.as_mut().unwrap().level_load.finish();
                        let tracker = &game.client.remote.as_ref().unwrap().level_load;
                        assert!(!game.draw_dimension_transition_screen(
                            &mut updated,
                            tracker,
                            None
                        ));
                        game.handle_packet(respawn("minecraft:overworld", 3))
                            .unwrap();
                        let tracker = &game.client.remote.as_ref().unwrap().level_load;
                        let mut vanilla = HudBatch::scratch([320, 240]);
                        assert!(!game.draw_dimension_transition_screen(
                            &mut vanilla,
                            tracker,
                            None
                        ));
                        game.draw_level_loading_background(&mut vanilla, tracker.reason);
                        assert!(!vanilla.vertices.is_empty());
                    });
                },
            );
        },
    );
}
