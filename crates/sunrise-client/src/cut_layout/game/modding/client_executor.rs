#![cfg(test)]
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::ClientExecutor;
    use mod_api::{
        __private::{HookPanicReport, take_hook_panics},
        AudioStream, AudioStreamRef, AudioStreamState, ClientHost, ClientPlayerLoggingOutEvent,
        Hooks, ModEntry, SoundSource, StreamSpec,
    };
    use std::{
        sync::{Arc, Mutex, mpsc},
        thread,
    };
    use sunrise_client::net::Event;
    use sunrise_level::crash;
    use sunrise_level::crash::ModCall;
    use {
        crate::game::server_side::native_entity_refs::tests::with_client_game,
        sunrise_client_gameplay::game::EventLoopCtl,
        sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    };

    struct PanickingPayload;
    impl Drop for PanickingPayload {
        fn drop(&mut self) {
            panic!("panic payload's destructor ran");
        }
    }

    fn executor(host: &dyn ClientHost, id: &'static str) -> ClientExecutor {
        let _call = crash::enter_native_mod(ModCall {
            id,
            hook: "client_setup",
            part: None,
        });
        host.client_executor().unwrap()
    }

    #[test]
    #[ignore = "daily"]
    fn worker_tasks_run_in_order_on_the_client_with_its_live_host() {
        with_client_game(|game| {
            let client = thread::current().id();
            let executor = executor(game, "producer");
            let calls = Arc::new(Mutex::new(Vec::new()));
            let worker_calls = calls.clone();
            let worker = executor.clone();
            thread::spawn(move || {
                for ordinal in [2, 0, 1] {
                    let calls = worker_calls.clone();
                    worker.execute(Box::new(move |host| {
                        calls.lock().unwrap().push((
                            ordinal,
                            thread::current().id(),
                            host.window_focused(),
                            crash::current_native_mod(),
                        ));
                    }));
                }
            })
            .join()
            .unwrap();
            assert!(calls.lock().unwrap().is_empty());
            assert!(game.client.remote.is_none());
            game.client.focused = true;
            game.poll_remote().unwrap();
            assert_eq!(
                *calls.lock().unwrap(),
                [
                    (2, client, true, Some("producer")),
                    (0, client, true, Some("producer")),
                    (1, client, true, Some("producer")),
                ]
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn closing_window_during_pending_save_discards_pending_and_late_client_tasks() {
        use sunrise_client_gameplay::input_trace::InputOrigin;
        use winit::{event::WindowEvent, event_loop::ControlFlow, window::WindowId};
        struct TestLoop;
        impl EventLoopCtl for TestLoop {
            fn exit(&self) {
                panic!("the quit must wait for the pending save");
            }
            fn set_control_flow(&self, _: ControlFlow) {}
        }
        struct PanicOnDiscard;
        impl Drop for PanicOnDiscard {
            fn drop(&mut self) {
                std::panic::panic_any(PanickingPayload);
            }
        }

        if run_fresh_cases(
            "cut_layout::game::client_side::client_executor::tests::closing_window_during_pending_save_discards_pending_and_late_client_tasks",
            "SUNRISE_CLIENT_EXECUTOR_CLOSE_DURING_SAVE_CHILD",
            &["close"],
        ) {
            return;
        }

        with_client_game(|game| {
            let executor = executor(game, "producer");
            let captured = Arc::new(());
            let pending = captured.clone();
            executor.execute(Box::new(move |_| {
                drop(pending);
                panic!("pending client task ran after shutdown");
            }));
            assert_eq!(Arc::strong_count(&captured), 2);

            let (reply, completion) = std::sync::mpsc::channel();
            game.test_seed_pending_server_save(reply);
            assert!(!game.client.pending_save.0.as_ref().unwrap().quit);
            game.options.headless = true;
            game.handle_window_event(
                &TestLoop,
                WindowId::dummy(),
                WindowEvent::CloseRequested,
                InputOrigin::User,
            );
            assert!(game.client.pending_save.0.as_ref().unwrap().quit);
            assert!(matches!(
                completion.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Empty)
            ));
            assert_eq!(Arc::strong_count(&captured), 1);
            let late = captured.clone();
            thread::spawn(move || {
                let panicking_capture = PanicOnDiscard;
                executor.execute(Box::new(move |_| {
                    drop(late);
                    drop(panicking_capture);
                }));
            })
            .join()
            .expect("dropping a late client task must not unwind from execute");
            assert_eq!(Arc::strong_count(&captured), 1);
            game.poll_remote().unwrap();
        });
    }

    #[test]
    #[ignore = "daily"]
    fn title_quit_discards_pending_and_late_client_tasks() {
        with_client_game(|game| {
            let saves = tempfile::tempdir().unwrap();
            game.client.title.0 = Some(game.new_title(saves.path().to_path_buf()));
            let executor = executor(game, "producer");
            let (pending, pending_calls) = mpsc::channel();
            executor.execute(Box::new(move |_| pending.send(()).unwrap()));

            game.title_action(crate::title::Action::Quit).unwrap();
            assert_eq!(
                pending_calls.try_recv(),
                Err(mpsc::TryRecvError::Disconnected)
            );
            assert!(game.client.pending_save.0.as_ref().unwrap().quit);

            let (late, late_calls) = mpsc::channel();
            thread::spawn(move || executor.execute(Box::new(move |_| late.send(()).unwrap())))
                .join()
                .unwrap();
            assert_eq!(late_calls.try_recv(), Err(mpsc::TryRecvError::Disconnected));
            game.poll_remote().unwrap();
            assert!(game.poll_closing_save().unwrap());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn executor_panic_reports_and_disables_only_the_failed_owner_without_undoing_live_placement() {
        struct Silent;
        impl AudioStream for Silent {
            fn read(&mut self, pcm: &mut [i16]) -> usize {
                pcm.fill(0);
                pcm.len()
            }
        }

        mod_platform::testing::with_mods(Default::default(), || {
            with_client_game(|game| {
                game.init_remote_for_replay().unwrap();
                game.client.remote.as_mut().unwrap().logged_in = true;
                game.spawn_entity(20, "minecraft:pig", [2.0, 64.0, 0.0], [0.0; 3]);
                let entity = game.entity(20).unwrap();
                game.client.audio.0.backend_live_for_bench();
                let stream = |owner| {
                    Arc::new(AudioStreamRef::new(
                        game.client
                            .audio
                            .0
                            .open_stream(
                                owner,
                                &StreamSpec::new(SoundSource::Music, 1, 48_000),
                                None,
                                Box::new(Silent),
                            )
                            .unwrap(),
                    ))
                };
                let failed_stream = stream("failed");
                let other_stream = stream("other");

                let failed = executor(game, "failed");
                let second_handle = executor(game, "failed");
                let other = executor(game, "other");
                let (record, calls) = mpsc::channel();
                let retarget = failed_stream.clone();
                let report = record.clone();
                failed.execute(Box::new(move |host| {
                    report
                        .send(("failed", retarget.set_entity(host, entity)))
                        .unwrap();
                    std::panic::panic_any(PanickingPayload);
                }));
                let queued = record.clone();
                second_handle.execute(Box::new(move |_| queued.send(("queued", false)).unwrap()));
                let retarget = other_stream.clone();
                let report = record.clone();
                thread::spawn(move || {
                    other.execute(Box::new(move |host| {
                        report
                            .send(("other", retarget.set_entity(host, entity)))
                            .unwrap();
                    }));
                })
                .join()
                .unwrap();

                game.poll_remote().unwrap();
                assert_eq!(
                    calls.try_iter().collect::<Vec<_>>(),
                    [("failed", true), ("other", true)]
                );
                // Refuse a new handle, and discard work submitted on a worker through a retained one.
                {
                    let _call = crash::enter_native_mod(ModCall {
                        id: "failed",
                        hook: "client_setup",
                        part: None,
                    });
                    assert!(game.client_executor().is_none());
                }
                thread::spawn(move || {
                    failed.execute(Box::new(move |_| record.send(("late", false)).unwrap()));
                })
                .join()
                .unwrap();
                assert_eq!(calls.try_recv(), Err(mpsc::TryRecvError::Disconnected));

                // A live placement survives a later panic: both streams now follow the entity.
                use sunrise_protocol::packets::play::{Clientbound, RemoveEntities};
                game.apply_packet(Clientbound::RemoveEntities(RemoveEntities {
                    ids: vec![20],
                }))
                .unwrap();
                game.follow_audio_stream_entities();
                assert_eq!(failed_stream.state(), AudioStreamState::Stopped);
                assert_eq!(other_stream.state(), AudioStreamState::Stopped);
                assert_eq!(
                    take_hook_panics(),
                    [HookPanicReport {
                        mod_id: "failed",
                        hook: "client_executor",
                    }]
                );
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn executor_drains_work_published_while_a_packet_disconnects() {
        struct Listener(mpsc::Sender<i32>);
        #[mod_api::__subscriptions]
        impl Hooks for Listener {
            fn client_player_logging_out(
                &self,
                host: &mut dyn ClientHost,
                _: &mut ClientPlayerLoggingOutEvent,
            ) {
                let record = self.0.clone();
                host.client_executor()
                    .unwrap()
                    .execute(Box::new(move |_| record.send(3).unwrap()));
            }
        }

        let (record, calls) = mpsc::channel();
        let mods = vec![ModEntry::new(
            "during_disconnect",
            Box::new(Listener(record.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_client_game(|game| {
                game.init_remote_for_replay().unwrap();
                game.client
                    .remote
                    .as_ref()
                    .unwrap()
                    .session
                    .received_event_probe()(Event::Disconnected {
                    reason: "fixture disconnected".into(),
                    key: None,
                    from_server: true,
                    after_login: true,
                });
                let executor = executor(game, "producer");
                let first = record.clone();
                executor.execute(Box::new(move |_| first.send(2).unwrap()));
                executor.execute(Box::new(move |_| record.send(1).unwrap()));

                game.poll_remote().unwrap();
                assert!(game.client.remote.is_none());
                assert_eq!(calls.try_iter().collect::<Vec<_>>(), [2, 1, 3]);
            });
        });
    }
}
