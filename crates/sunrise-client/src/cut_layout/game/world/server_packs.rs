#![cfg(test)]
use sunrise_client::hud::HudBatch;
use sunrise_client::toasts::SystemToast;
use sunrise_client_gameplay::game::client_side::server_packs::PackNotifier;
use sunrise_game_data::text::Text;
#[cfg(test)]
mod tests {
    use super::*;
    use crate::server_packs::download::Progress::*;
    use std::{path::Path, time::Duration};
    use sunrise_client::multiplayer;
    use sunrise_client::server_packs::{Prompt, action, download};
    use sunrise_client_gameplay::game::toast_manager::{SystemToastId, Toast};
    use sunrise_protocol::packets::configuration::ResourcePackPush;
    use sunrise_protocol::servers;
    use sunrise_protocol::{
        connection::Connection,
        packets::{Empty, play::Serverbound},
    };
    use {
        crate::game::server_side::native_entity_refs::tests::with_client_game,
        sunrise_client_gameplay::game::client_side::native_client_session::tests::finish_login,
        sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client,
        sunrise_gameplay_common::game::in_own_process,
    };

    const PACK_IDS: [u128; 2] = [31, 32];

    fn with_pack_prompt(
        required: bool,
        test: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>, &mut Connection, &Path),
    ) {
        const ADDRESS: &str = "pack-test.invalid";
        let dir = tempfile::tempdir().unwrap();
        sunrise_level::logging::set_player_dir(dir.path().to_owned());
        let file = dir.path().join("servers.dat");
        if required {
            // Start the joined shown row at Disabled (`0`) and include an unrelated entry
            // whose Disabled policy is encoded as an NBT Int `0` (valid under vanilla's
            // NbtOps.getBooleanValue) so a lossy decoder or policy reset would be caught.
            let root = sunrise_core::nbt::NamedTag {
                name: sunrise_core::nbt::JavaString::from(""),
                tag: sunrise_core::nbt::Tag::compound([(
                    "servers",
                    sunrise_core::nbt::Tag::List(vec![
                        sunrise_core::nbt::Tag::compound([
                            ("name", sunrise_core::nbt::Tag::String("Shown".into())),
                            ("ip", sunrise_core::nbt::Tag::String(ADDRESS.into())),
                            ("acceptTextures", sunrise_core::nbt::Tag::Byte(0)),
                            ("hidden", sunrise_core::nbt::Tag::Byte(0)),
                        ]),
                        sunrise_core::nbt::Tag::compound([
                            ("name", sunrise_core::nbt::Tag::String("Numeric".into())),
                            ("ip", sunrise_core::nbt::Tag::String("other.invalid".into())),
                            ("acceptTextures", sunrise_core::nbt::Tag::Int(0)),
                            ("hidden", sunrise_core::nbt::Tag::Byte(0)),
                        ]),
                        sunrise_core::nbt::Tag::compound([
                            ("name", sunrise_core::nbt::Tag::String("Hidden".into())),
                            ("ip", sunrise_core::nbt::Tag::String(ADDRESS.into())),
                            ("acceptTextures", sunrise_core::nbt::Tag::Byte(1)),
                            ("hidden", sunrise_core::nbt::Tag::Byte(1)),
                        ]),
                    ]),
                )]),
            };
            let bytes = sunrise_core::nbt::write_named(&root, sunrise_core::nbt::Limits::default())
                .unwrap();
            std::fs::write(&file, bytes).unwrap();
        } else {
            let shown = servers::ServerEntry::new("Shown", ADDRESS);
            let mut hidden = servers::ServerEntry::new("Hidden", ADDRESS);
            hidden.accept_textures = Some(true);
            servers::save_all(&file, &[shown], &[hidden]).unwrap();
        }

        with_client_game(|game| {
            let mut peer = Connection::local(logged_client(game));
            peer.reader
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            finish_login(game, &mut peer);
            game.server_packs_connect(Some(ADDRESS), None, ("SessionReadTest".into(), 42));
            for id in PACK_IDS {
                game.handle_pack_push(ResourcePackPush {
                    id,
                    url: "https://pack-test.invalid/server.zip".into(),
                    hash: String::new(),
                    required,
                    prompt: None,
                });
            }
            test(game, &mut peer, &file);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn required_decline_closes_the_wire_but_reports_locally_and_saves() {
        in_own_process(
            "cut_layout::game::client_side::server_packs::tests::required_decline_closes_the_wire_but_reports_locally_and_saves",
            || {
                with_pack_prompt(true, |game, peer, file| {
                    let original_bytes = std::fs::read(file).unwrap();
                    let backup = file.with_file_name("servers.dat_old");
                    // Mark an old mtime to detect an actual rewrite.
                    std::fs::OpenOptions::new()
                        .write(true)
                        .open(file)
                        .unwrap()
                        .set_modified(std::time::UNIX_EPOCH)
                        .unwrap();
                    let old_modified = file.metadata().unwrap().modified().unwrap();
                    game.client.server_packs.0.packs.record_feedback();
                    game.answer_pack_prompt(false).unwrap();

                    assert!(game.client.remote.is_none());
                    assert!(matches!(
                        &game.client.title.0.as_ref().unwrap().mode,
                        sunrise_client::title::Mode::Multiplayer(multiplayer::Screen::Disconnected {
                            reason, retry: None, ..
                        }) if reason.translation_key() == Some("multiplayer.requiredTexturePrompt.disconnect")
                    ));
                    loop {
                        match peer.recv() {
                            Ok((id, body)) => assert!(!matches!(
                                Serverbound::decode(id, &body).unwrap(),
                                Serverbound::ResourcePack(_)
                            )),
                            Err(e) if e.is_end_of_stream() => break,
                            Err(e) => panic!("expected transport EOF: {e}"),
                        }
                    }
                    assert_eq!(
                        game.client.server_packs.0.packs.take_recorded_feedback(),
                        PACK_IDS.map(|id| (id, action::DECLINED))
                    );
                    assert_eq!(game.client.server_packs.0.packs.prompt(), Prompt::Pending);
                    assert!(game.client.server_packs.0.packs.take_replies().is_empty());
                    assert_ne!(file.metadata().unwrap().modified().unwrap(), old_modified);
                    assert_eq!(std::fs::read(&backup).unwrap(), original_bytes);
                    // Read the saved raw NBT directly to prove Disabled policies were not cleared.
                    let (saved, _) = sunrise_core::nbt::read_named(
                        &std::fs::read(file).unwrap(),
                        sunrise_core::nbt::Limits::default(),
                    )
                    .unwrap();
                    let Some(sunrise_core::nbt::Tag::List(entries)) = saved.tag.get("servers")
                    else {
                        panic!("expected servers list");
                    };
                    assert_eq!(entries.len(), 3);
                    assert_eq!(
                        entries[0].get("acceptTextures"),
                        Some(&sunrise_core::nbt::Tag::Byte(0))
                    );
                    assert_eq!(
                        entries[1].get("acceptTextures"),
                        Some(&sunrise_core::nbt::Tag::Byte(0))
                    );
                    assert_eq!(
                        entries[2].get("acceptTextures"),
                        Some(&sunrise_core::nbt::Tag::Byte(1))
                    );
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn optional_decline_replies_on_the_live_wire_and_saves_disabled_policy() {
        in_own_process(
            "cut_layout::game::client_side::server_packs::tests::optional_decline_replies_on_the_live_wire_and_saves_disabled_policy",
            || {
                with_pack_prompt(false, |game, peer, file| {
                    let (mut shown, hidden) = servers::load_all(file);
                    shown[0].accept_textures = Some(false);
                    game.answer_pack_prompt(false).unwrap();
                    game.tick_server_packs();
                    game.send(Serverbound::Punch(Empty)); // Fence the outgoing replies.
                    let mut replies = Vec::new();
                    loop {
                        let (id, body) = peer.recv().unwrap();
                        match Serverbound::decode(id, &body).unwrap() {
                            Serverbound::ResourcePack(response) => {
                                replies.push((response.id, response.action));
                            }
                            Serverbound::Punch(_) => break,
                            _ => {}
                        }
                    }
                    assert_eq!(replies, PACK_IDS.map(|id| (id, 1))); // DECLINED
                    assert_eq!(servers::load_all(file), (shown, hidden));
                });
            },
        );
    }

    /// `ServerPackManager.acceptPack` reports from `handleResourcePackPush`, so a pack
    /// the server list allows is ACCEPTED in configuration, before the FinishConfiguration
    /// that the same poll acknowledges.
    #[test]
    #[ignore = "daily"]
    fn an_allowed_pack_is_accepted_before_finish_configuration() {
        in_own_process(
            "cut_layout::game::client_side::server_packs::tests::an_allowed_pack_is_accepted_before_finish_configuration",
            || {
                use sunrise_protocol::packets::{configuration as cfg, login};
                const ADDRESS: &str = "pack-order.invalid";
                let dir = tempfile::tempdir().unwrap();
                sunrise_client::logging::set_player_dir(dir.path().to_owned());
                let mut entry = servers::ServerEntry::new("Packs", ADDRESS);
                entry.accept_textures = Some(true);
                servers::save_all(&dir.path().join("servers.dat"), &[entry], &[]).unwrap();
                with_client_game(|game| {
                    let mut peer = Connection::local(logged_client(game));
                    peer.reader
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    game.server_packs_connect(Some(ADDRESS), None, ("SessionReadTest".into(), 42));
                    peer.recv().unwrap(); // Intention.
                    peer.recv().unwrap(); // Hello.
                    let finished = login::Clientbound::LoginFinished(login::LoginFinished {
                        profile: login::GameProfile {
                            uuid: 42,
                            name: "SessionReadTest".into(),
                            properties: vec![],
                        },
                        session: 1,
                    });
                    peer.send(&finished.encode()).unwrap();
                    peer.recv().unwrap(); // Login acknowledged.
                    peer.recv().unwrap(); // Brand.
                    peer.recv().unwrap(); // Client information.
                    let push = cfg::ResourcePackPush {
                        id: PACK_IDS[0],
                        url: "https://pack-order.invalid/server.zip".into(),
                        hash: String::new(),
                        required: false,
                        prompt: None,
                    };
                    for packet in [
                        cfg::Clientbound::ResourcePackPush(push),
                        cfg::Clientbound::FinishConfiguration(Empty),
                    ] {
                        peer.send(&packet.encode()).unwrap();
                    }
                    // Both reach main and are handled in one poll, with no pack tick between.
                    game.acknowledge_test_configuration();
                    let mut sent = Vec::new();
                    loop {
                        let (id, body) = peer.recv().unwrap();
                        match cfg::Serverbound::decode(id, &body).unwrap() {
                            cfg::Serverbound::ResourcePack(r) => sent.push(Some((r.id, r.action))),
                            cfg::Serverbound::FinishConfiguration(_) => {
                                sent.push(None);
                                break;
                            }
                            _ => {}
                        }
                    }
                    assert_eq!(sent, [Some((PACK_IDS[0], action::ACCEPTED)), None]);
                    assert!(game.client.server_packs.0.packs.take_replies().is_empty());
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn an_audio_panic_toast_survives_its_mods_hud_veto_but_not_f1() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::client_side::server_packs::tests::an_audio_panic_toast_survives_its_mods_hud_veto_but_not_f1",
            || {
                use mod_api::{
                    AudioStream, ClientHost, Flow, Hooks, ModEntry, RenderGuiPreEvent, SoundSource,
                    StreamSpec,
                };
                use std::{
                    sync::mpsc,
                    time::{Duration, Instant},
                };
                use winit::keyboard::KeyCode;

                const OWNER: &str = "audio_toast_fixture";
                struct HideHud;
                #[mod_api::__subscriptions]
                impl Hooks for HideHud {
                    fn render_gui_pre(
                        &self,
                        _: &dyn ClientHost,
                        _: &mut RenderGuiPreEvent,
                    ) -> Flow {
                        Flow::Cancel
                    }
                }

                struct BrokenSource(mpsc::Sender<()>);
                impl AudioStream for BrokenSource {
                    fn read(&mut self, _: &mut [i16]) -> usize {
                        panic!("broken pull callback");
                    }
                }
                impl Drop for BrokenSource {
                    fn drop(&mut self) {
                        let _ = self.0.send(());
                    }
                }

                mod_platform::testing::with_mods(
                    vec![ModEntry::new(OWNER, Box::new(HideHud))].into(),
                    || {
                        crate::game::server_side::native_entity_refs::tests::with_client_game(
                            |game| {
                                game.client.title.0 = None;
                                game.client.screen = sunrise_gameplay_common::game::Screen::Playing;
                                game.client.audio.0.backend_live_for_bench();
                                let spec = StreamSpec::new(SoundSource::Voice, 2, 48_000);
                                let (dropped, retired) = mpsc::channel();
                                let _handle = game
                                    .client
                                    .audio
                                    .0
                                    .open_stream(
                                        OWNER,
                                        &spec,
                                        None,
                                        Box::new(BrokenSource(dropped)),
                                    )
                                    .unwrap();
                                game.client.audio.0.mix_buffer(&mut [0.0; 2], 2, 48_000);
                                // The worker queues the notice before dropping the failed source.
                                retired.recv_timeout(Duration::from_secs(1)).unwrap();

                                game.add_system_toast(
                                    Some(SystemToastId::PackLoadFailure),
                                    SystemToast::new(Text::tr("resourcePack.load_fail", &[]), None),
                                );
                                let size = [640, 480];
                                let _ = game.hud(size);
                                let _ = game.hud(size);
                                let [regular, error] =
                                    game.client.toast_manager.visible.as_mut_slice()
                                else {
                                    panic!("expected two visible notices")
                                };
                                let Toast::System(
                                    Some(SystemToastId::PackLoadFailure),
                                    regular_toast,
                                ) = &mut regular.toast
                                else {
                                    panic!("expected ordinary toast")
                                };
                                let Toast::Diagnostic {
                                    toast: error_toast, ..
                                } = &mut error.toast
                                else {
                                    panic!("expected diagnostic toast")
                                };
                                assert_eq!(regular.slot, 0);
                                assert_ne!(error.slot, 0, "the ordinary toast keeps its slot");
                                let error_y = 32.0 * error.slot as f32;
                                assert_eq!(
                                    error_toast.message.as_ref().unwrap().component()["with"][0]["text"],
                                    OWNER
                                );
                                let past = Instant::now() - Duration::from_millis(700);
                                regular_toast.slide.origin = Some(past);
                                error_toast.slide.origin = Some(past);

                                let rect = *game
                                    .client
                                    .gui
                                    .atlas
                                    .sprites
                                    .entry("toast/system".into())
                                    .or_insert(sunrise_client::hud::Rect {
                                        u0: 0.25,
                                        v0: 0.25,
                                        u1: 0.5,
                                        v1: 0.5,
                                        w: 160,
                                        h: 32,
                                    });
                                let has_toast = |batch: &HudBatch, y| {
                                    batch.vertices.iter().any(|vertex| {
                                        vertex.uv == [rect.u0, rect.v0]
                                            && vertex.pos[1] == y
                                            && vertex.pos[0] < batch.width
                                    })
                                };
                                let canceled = game.hud(size);
                                assert!(
                                    has_toast(&canceled, 0.0),
                                    "the HUD veto does not hide toasts"
                                );
                                assert!(has_toast(&canceled, error_y), "audio error still draws");
                                game.key_pressed(KeyCode::F1, KeyCode::F1).unwrap();
                                game.handle_play_keybinds();
                                assert!(
                                    !has_toast(&game.hud(size), error_y),
                                    "F1 hides the notice"
                                );
                            },
                        );
                    },
                );
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    #[cfg(any())]
    fn closing_save_reports_reach_the_title_in_the_owning_window() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::client_side::server_packs::tests::closing_save_reports_reach_the_title_in_the_owning_window",
            || {
                use mod_api::{ClientHost, Flow, Hooks, ModEntry, ToastAddEvent, ToastKind};
                use {
                    crate::game::server_side::native_entity_refs::tests::with_client_game,
                    sunrise_client_gameplay::game::client_side::integrated_client::Integrated,
                    sunrise_gameplay::game::server_side::integrated::Serving,
                };

                struct Listener;
                #[mod_api::__subscriptions]
                impl Hooks for Listener {
                    fn toast_add(&self, _: &dyn ClientHost, event: &mut ToastAddEvent<'_>) -> Flow {
                        use mod_api::text::TextContent::Translatable;

                        if let Translatable { key, .. } = &event.title.content
                            && key == "chunk.toast.saveFailure"
                        {
                            assert_eq!(event.kind, ToastKind::System);
                            *event.description = Some(mod_api::Text::literal("Reported on title"));
                        }
                        Flow::Continue
                    }
                }

                let mods = vec![ModEntry::new("chunk_report_probe", Box::new(Listener))];
                mod_platform::testing::with_mods(mods.into(), || {
                    with_client_game(|window| {
                        let serving = Serving::fixture_with_reports(
                            0,
                            window.client.client_reports.0.clone(),
                        );
                        window.client.integrated = Some(Integrated::fixture_for(&serving));
                        window.begin_closing_save(false).unwrap();
                        assert!(window.client.pending_save.0.is_some());
                        let dir = tempfile::tempdir().unwrap();
                        let reporter = sunrise_sim::chunk_io::SaveFailureReporter::new(
                            true,
                            dir.path(),
                            "test",
                        );
                        serving.reports().report_save_failure(
                            &reporter,
                            dir.path(),
                            "minecraft:overworld",
                            "chunk",
                            (10, -7),
                            &std::io::Error::other("region write failed"),
                        );
                        drop(reporter);
                        drop(serving);

                        assert!(!window.poll_closing_save().unwrap());
                        assert!(window.client.integrated.is_none());
                        assert!(window.client.title.0.is_some());
                        assert!(
                            window
                                .client
                                .toast_manager
                                .system_mut(SystemToastId::ChunkIo(2))
                                .is_none()
                        );
                        let mut batch = HudBatch::scratch([640, 480]);
                        window.draw_every_toast(&mut batch);
                        let [entry] = window.client.toast_manager.visible.as_slice() else {
                            panic!("expected a visible toast")
                        };
                        let Toast::System(Some(SystemToastId::ChunkIo(2)), toast) = &entry.toast
                        else {
                            panic!("expected a save failure toast")
                        };
                        assert_eq!(
                            toast.title.translation_key(),
                            Some("chunk.toast.saveFailure")
                        );
                        assert_eq!(toast.title.component()["with"][0]["text"], "[10, -7]");
                        assert_eq!(toast.message.as_ref().unwrap().plain(), "Reported on title");
                    });
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    #[cfg(any())]
    fn scheduled_task_error_toasts_are_admitted_by_the_owning_client() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::client_side::server_packs::tests::scheduled_task_error_toasts_are_admitted_by_the_owning_client",
            || {
                use mod_api::{
                    ClientHost, Flow, Hooks, ModEntry, ServerHost, ToastAddEvent, ToastKind,
                };
                use std::sync::{Arc, Mutex};
                use {
                    crate::game::server_side::native_entity_refs::tests::with_client_game,
                    sunrise_client_gameplay::game::client_side::integrated_client::Integrated,
                    sunrise_gameplay::game::native_entity_refs::tests::with_game,
                    sunrise_gameplay::game::server_side::integrated::Serving,
                };

                const ACCEPTED: &str = "toast_accept";
                const CANCELLED: &str = "toast_cancel";
                struct Listener(Arc<Mutex<Vec<String>>>);
                #[mod_api::__subscriptions]
                impl Hooks for Listener {
                    fn toast_add(&self, _: &dyn ClientHost, event: &mut ToastAddEvent<'_>) -> Flow {
                        use mod_api::text::TextContent::{Literal, Translatable};

                        let Some(Translatable { key, arguments, .. }) =
                            event.description.as_ref().map(|text| &text.content)
                        else {
                            return Flow::Continue;
                        };
                        if key != "sunrise.mods.scheduled_task_error.message" {
                            return Flow::Continue;
                        }
                        assert_eq!(event.kind, ToastKind::System);
                        let [argument] = arguments.as_slice() else {
                            panic!("the notice must name one mod");
                        };
                        let Literal(id) = &argument.content else {
                            panic!("the mod ID must be a literal component");
                        };
                        self.0.lock().unwrap().push(id.clone());
                        if id == CANCELLED {
                            Flow::Cancel
                        } else {
                            *event.title = mod_api::Text::literal("Edited task error");
                            Flow::Continue
                        }
                    }
                }
                struct NoHooks;
                #[mod_api::__subscriptions]
                impl Hooks for NoHooks {}

                let observed = Arc::new(Mutex::new(Vec::new()));
                let mods = vec![
                    ModEntry::new(ACCEPTED, Box::new(Listener(observed.clone()))),
                    ModEntry::new(CANCELLED, Box::new(NoHooks)),
                ];
                mod_platform::testing::with_mods(mods.into(), || {
                    with_client_game(|window| {
                        let serving = Serving::fixture_with_reports(
                            0,
                            window.client.client_reports.0.clone(),
                        );
                        window.client.integrated = Some(Integrated::fixture_for(&serving));
                        with_game(|server| {
                            server.server.serving = Some(serving);
                            server.native_lifecycle.world_open_ready().unwrap();
                            for id in [ACCEPTED, CANCELLED] {
                                let _owner = mod_api::__private::enter(id, "schedule fixture");
                                let host: &mut dyn ServerHost = &mut *server;
                                host.schedule_task(
                                    0,
                                    Box::new(|_| panic!("scheduled callback panicked")),
                                )
                                .unwrap();
                            }
                            server.native_lifecycle_server_tick_start(true);
                            server.native_lifecycle_server_tick_end(false);
                        });
                        assert!(
                            observed.lock().unwrap().is_empty(),
                            "not admitted on the server"
                        );

                        let mut batch = HudBatch::scratch([640, 480]);
                        with_client_game(|other| {
                            other.draw_every_toast(&mut batch);
                            assert!(other.client.toast_manager.visible.is_empty());
                            assert!(
                                observed.lock().unwrap().is_empty(),
                                "not sent to another window"
                            );
                        });

                        window.draw_every_toast(&mut batch);
                        let [entry] = window.client.toast_manager.visible.as_slice() else {
                            panic!("only the accepted notice should reach the queue");
                        };
                        let Toast::System(None, toast) = &entry.toast else {
                            panic!("expected a scheduled task error toast")
                        };
                        assert_eq!(toast.title.plain(), "Edited task error");
                        assert_eq!(
                            toast.message.as_ref().unwrap().component()["with"][0]["text"],
                            ACCEPTED,
                        );
                        window.draw_every_toast(&mut batch);
                        assert_eq!(observed.lock().unwrap().as_slice(), [ACCEPTED, CANCELLED]);
                    });
                });
            },
        );
    }

    fn notifier(total: usize, events: &[download::Progress]) -> PackNotifier {
        let (send, receive) = std::sync::mpsc::channel();
        for event in events {
            send.send(*event).unwrap();
        }
        let mut notifier = PackNotifier::new(receive, total, 0);
        assert!(notifier.poll(|_| true).0);
        notifier
    }
    fn texts(n: &PackNotifier) -> (serde_json::Value, Option<serde_json::Value>) {
        let (title, message) = n.text();
        (title.component(), message.map(|m| m.component()))
    }

    #[test]
    fn the_toast_counts_requests_and_shows_progress() {
        let n = notifier(2, &[RequestStart]);
        let (title, message) = texts(&n);
        assert_eq!(title["translate"], "download.pack.title");
        assert_eq!(title["with"], serde_json::json!([1, 2]));
        assert_eq!(message, None);
        let n = notifier(2, &[RequestStart, DownloadStart(Some(400)), Bytes(100)]);
        assert_eq!(
            texts(&n).1.unwrap(),
            serde_json::json!({"translate": "download.pack.progress.percent", "with": [25]})
        );
        let n = notifier(1, &[RequestStart, DownloadStart(None), Bytes(1280)]);
        assert_eq!(
            texts(&n).1.unwrap()["with"],
            serde_json::json!([{"text": "1.3 KB"}])
        );
    }

    #[test]
    fn a_failed_pack_turns_the_toast_into_the_failure_count() {
        let events = [
            RequestStart,
            RequestFinished(false),
            RequestStart,
            DownloadStart(Some(3)),
            Bytes(3),
        ];
        let (send, receive) = std::sync::mpsc::channel();
        for event in events {
            send.send(event).unwrap();
        }
        let mut n = PackNotifier::new(receive, 2, 0);
        assert!(n.poll(|_| true).0);
        // Not the last request yet: still downloading.
        assert_eq!(texts(&n).0["translate"], "download.pack.title");
        send.send(RequestFinished(true)).unwrap();
        drop(send);
        assert_eq!(
            n.poll(|_| true),
            (false, false),
            "a failure does not force-hide the toast"
        );
        let (title, message) = texts(&n);
        assert_eq!(
            title,
            serde_json::json!({"translate": "download.pack.failed", "with": [1, 2]})
        );
        assert_eq!(message, None);
    }

    #[test]
    fn a_cancelled_download_toast_is_not_retried_on_progress() {
        let (send, receive) = std::sync::mpsc::channel();
        let mut n = PackNotifier::new(receive, 1, 0);
        send.send(RequestStart).unwrap();
        assert_eq!(n.poll(|_| false), (true, false));
        send.send(DownloadStart(Some(100))).unwrap();
        send.send(Bytes(50)).unwrap();
        assert_eq!(
            n.poll(|_| panic!("cancelled toast must not be added again")),
            (true, false)
        );
        send.send(RequestFinished(true)).unwrap();
        drop(send);
        assert_eq!(n.poll(|_| panic!("no toast on completion")), (false, true));
    }

    #[test]
    fn a_finished_batch_without_a_toast_is_dropped() {
        let (send, receive) = std::sync::mpsc::channel::<download::Progress>();
        drop(send);
        assert_eq!(
            PackNotifier::new(receive, 1, 0).poll(|_| panic!("no toast without progress")),
            (false, false)
        );
    }

    #[test]
    #[ignore = "daily"]
    fn hook_panic_toast_cannot_be_vetoed_or_rewritten_by_toast_add() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::client_side::server_packs::tests::hook_panic_toast_cannot_be_vetoed_or_rewritten_by_toast_add",
            || {
                use mod_api::{
                    ClientHost, ClientTickPreEvent, Flow, Hooks, ModEntry, RenderGuiPreEvent,
                    ToastAddEvent,
                };
                use sunrise_game_data::lang;

                struct VetoingPanicker;
                #[mod_api::__subscriptions]
                impl Hooks for VetoingPanicker {
                    fn client_tick_pre(&self, _: &mut dyn ClientHost, _: &mut ClientTickPreEvent) {
                        panic!("boom from vetoing mod");
                    }
                    fn toast_add(&self, _: &dyn ClientHost, event: &mut ToastAddEvent<'_>) -> Flow {
                        *event.title = mod_api::Text::literal("rewritten");
                        *event.description = Some(mod_api::Text::literal("hidden"));
                        Flow::Cancel
                    }
                    fn render_gui_pre(
                        &self,
                        _: &dyn ClientHost,
                        _: &mut RenderGuiPreEvent,
                    ) -> Flow {
                        Flow::Cancel
                    }
                }
                let mods: mod_api::Mods =
                    vec![ModEntry::new("veto_mod", Box::new(VetoingPanicker))].into();
                mod_platform::testing::with_mods(mods, || {
                    super::super::native_entity_refs::tests::with_client_game(|game| {
                        // Ordinary system toasts are still vetoed by toast_add.
                        assert!(!game.add_system_toast(
                            None,
                            SystemToast::new(Text::literal("ordinary"), None)
                        ));
                        game.native_client_tick_pre();
                        // Dedicated/server instances do not drain the client toast queue.
                        game.options.dedicated = Some(Vec::new());
                        game.add_hook_panic_toasts();
                        assert!(game.client.toast_manager.queued.is_empty());
                        // The client drains and queues the toast directly without firing toast_add.
                        game.options.dedicated = None;
                        let _ = lang::scoped(lang::Language::load(&[], lang::DEFAULT), || {
                            game.add_hook_panic_toasts();
                        });
                        assert_eq!(game.client.toast_manager.queued.len(), 1);
                        let Some(Toast::Diagnostic { toast, .. }) =
                            game.client.toast_manager.queued.front()
                        else {
                            panic!("expected a queued diagnostic")
                        };
                        assert_eq!(
                            toast.title.component()["translate"],
                            "sunrise.mods.hook_panic.title"
                        );
                        let message = toast.message.as_ref().unwrap();
                        assert_eq!(
                            message.component(),
                            serde_json::json!({
                                "translate": "sunrise.mods.hook_panic.description",
                                "with": [{"text": "veto_mod"}, {"text": "client_tick_pre"}],
                            })
                        );
                        assert!(message.plain().contains("veto_mod"));
                        let mut gui_canceled = false;
                        game.native_render_gui_pre(&mut gui_canceled);
                        assert!(gui_canceled);
                        // Advance the existing slide past its entrance without sleeping.
                        let mut first = HudBatch::scratch([640, 480]);
                        game.draw_every_toast(&mut first);
                        game.draw_every_toast(&mut first);
                        let entry = &mut game.client.toast_manager.visible[0];
                        assert_eq!(entry.slot, 0);
                        let Toast::Diagnostic { toast, .. } = &mut entry.toast else {
                            panic!("expected a visible diagnostic")
                        };
                        toast.slide.origin =
                            Some(std::time::Instant::now() - std::time::Duration::from_secs(1));
                        let mut visible = HudBatch::scratch([640, 480]);
                        game.draw_every_toast(&mut visible);
                        let mut drawn_text = visible.vertices.iter().filter(|v| {
                            v.text == 1
                                && (0.0..visible.width).contains(&v.pos[0])
                                && (0.0..32.0).contains(&v.pos[1])
                        });
                        assert!(
                            drawn_text.clone().any(|v| {
                                v.color == sunrise_client::hud::rgba(255, 85, 85, 255)
                            }),
                            "the error title draws inside its slot despite the HUD veto"
                        );
                        assert!(
                            drawn_text.any(|v| v.color == u32::MAX),
                            "the message draws too"
                        );

                        game.set_hud_mode(mod_api::HudMode::NoHud);
                        assert!(game.client.chat_ui.hide_gui);
                        let mut mod_hidden = HudBatch::scratch([640, 480]);
                        game.draw_every_toast(&mut mod_hidden);
                        assert!(
                            !mod_hidden.vertices.is_empty(),
                            "NoHud does not hide diagnostics"
                        );

                        game.toggle_hide_gui();
                        game.set_hud_mode(mod_api::HudMode::AllVisible);
                        assert!(game.client.chat_ui.player_hides_gui);
                        let mut player_hidden = HudBatch::scratch([640, 480]);
                        game.draw_every_toast(&mut player_hidden);
                        assert!(
                            player_hidden.vertices.iter().any(|v| {
                                v.text == 1
                                    && v.color == sunrise_client::hud::rgba(255, 85, 85, 255)
                            }),
                            "the hook failure diagnostic still draws with the vanilla HUD hidden"
                        );
                    });
                });
            },
        );
    }
}
