use std::time::Instant;
use sunrise_client::hud::HudBatch;
use sunrise_client_gameplay::game::client_side::advancements_screen::{TabBounds, window};
use sunrise_core::{nbt::Tag, vanilla_bytes::Writer};
use sunrise_protocol::{
    buf::{Codec, Reader},
    ids::play::serverbound::SEEN_ADVANCEMENTS,
    item::Stack,
    packets::play::{
        Advancement, AdvancementDisplay, Clientbound, SeenAdvancements, UpdateAdvancements,
    },
};
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay_common::game::in_own_process,
};

fn displayed(id: &str, parent: Option<&str>, x: f32, y: f32) -> Advancement {
    Advancement {
        id: id.into(),
        parent: parent.map(str::to_owned),
        display: Some(AdvancementDisplay {
            title: Tag::String(id.into()),
            description: Tag::String("Test advancement".into()),
            icon: Stack::new(1, 1),
            frame: 0,
            show_toast: true,
            hidden: false,
            background: parent.is_none().then(|| "test:background".into()),
        }),
        requirements: vec![vec!["c".into()]],
        telemetry: false,
        x,
        y,
    }
}

fn position(game: &sunrise_client_gameplay::game::ClientGame<'_>, a: &Advancement) -> (f32, f32) {
    game.adv.remote_tree.as_ref().unwrap().positions[game.adv.tree.by_id[&a.id]]
}

#[test]
#[ignore = "daily"]
fn remote_tree_rejects_unresolved_nodes_and_retains_removed_progress() {
    in_own_process(
        "cut_layout::game::client_side::advancements_screen::remote_tests::remote_tree_rejects_unresolved_nodes_and_retains_removed_progress",
        || {
            with_client_game(|game| {
                game.init_remote_for_replay().unwrap();
                let root = displayed("test:root", None, 0.0, 0.0);
                let child = displayed("test:child", Some(&root.id), 1.0, 1.5);
                let grand = displayed("test:grand", Some(&child.id), 2.0, 3.0);
                let sibling = displayed("test:sibling", Some(&root.id), 1.0, 2.5);
                let other = displayed("test:other", None, 0.0, 0.0);
                let orphan = displayed("test:orphan", Some("test:missing_parent"), 9.0, 9.0);
                let cyc_a = displayed("test:cyc_a", Some("test:cyc_b"), 4.0, 0.0);
                let cyc_b = displayed("test:cyc_b", Some(&cyc_a.id), 5.0, 0.0);
                game.remote_advancements(UpdateAdvancements {
                    reset: false,
                    added: vec![
                        grand.clone(),
                        child.clone(),
                        orphan.clone(),
                        cyc_a,
                        cyc_b,
                        root.clone(),
                        sibling.clone(),
                        other.clone(),
                    ],
                    removed: vec![],
                    progress: [&root, &child, &grand, &orphan]
                        .into_iter()
                        .map(|a| (a.id.clone(), vec![("c".into(), Some(0))]))
                        .collect(),
                    show: true,
                });
                assert_eq!(
                    game.ui.registries.advancements,
                    [&root, &sibling, &other, &child, &grand].map(|a| a.id.clone()),
                );
                assert_eq!(position(game, &grand), (2.0, 3.0));
                assert_eq!(position(game, &sibling), (1.0, 2.5));
                assert_eq!(game.adv.progress.criteria.len(), 3);
                assert_eq!(game.client.toast_manager.queued.len(), 3);

                game.open_advancements(true);
                game.select_advancement_tab(game.adv.tree.by_id[&other.id]);
                game.client.adv_ui.0.tab_state_mut().unwrap().scroll = Some((12.0, 34.0));
                game.remote_advancements(UpdateAdvancements {
                    reset: false,
                    added: vec![],
                    removed: vec![root.id.clone()],
                    progress: vec![(grand.id.clone(), vec![("c".into(), None)])],
                    show: true,
                });
                assert_eq!(
                    game.ui.registries.advancements,
                    std::slice::from_ref(&other.id)
                );
                assert!(game.adv.progress.has(&root.id, "c"));
                assert!(game.adv.progress.has(&grand.id, "c"));
                let view = game.adv_view();
                assert_eq!(view.tabs, [0]);
                assert_eq!(view.tab, Some(0));
                assert_eq!(
                    game.client.adv_ui.0.tab_state().unwrap().scroll,
                    Some((12.0, 34.0))
                );

                game.remote_advancements(UpdateAdvancements {
                    reset: false,
                    added: vec![child.clone(), root.clone()],
                    removed: vec![other.id],
                    progress: vec![],
                    show: true,
                });
                let view = game.adv_view();
                assert_eq!(view.tabs, [0]);
                assert_eq!(view.tab, Some(0));
                assert_eq!(game.client.adv_ui.0.tab_state().unwrap().scroll, None);
                assert!(game.adv_done(game.adv.tree.by_id[&child.id]));
                assert_eq!(position(game, &child), (1.0, 1.5));
                assert_eq!(game.client.toast_manager.queued.len(), 3);

                game.remote_advancements(UpdateAdvancements {
                    reset: true,
                    added: vec![root.clone()],
                    removed: vec![],
                    progress: vec![],
                    show: true,
                });
                assert_eq!(game.ui.registries.advancements, [root.id]);
                assert!(game.adv.progress.criteria.is_empty());
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn opening_reports_the_first_tab_twice_and_preserves_offscreen_selection() {
    in_own_process(
        "cut_layout::game::client_side::advancements_screen::remote_tests::opening_reports_the_first_tab_twice_and_preserves_offscreen_selection",
        || {
            with_client_game(|game| {
                game.init_remote_for_replay().unwrap();
                let (session, mut peer, _keepalive) =
                    sunrise_client::net::Session::packet_hold_probe();
                game.client.remote.as_mut().unwrap().session = session;
                let mut receive_selection = || {
                    let (id, body) = peer.recv().unwrap();
                    assert_eq!(id, SEEN_ADVANCEMENTS);
                    SeenAdvancements::read(&mut Reader::new(&body)).unwrap()
                };
                let first = displayed("test:first", None, 0.0, 0.0);
                let second = displayed("test:second", None, 0.0, 0.0);
                let reset = UpdateAdvancements {
                    reset: true,
                    added: vec![first.clone(), second.clone()],
                    removed: vec![],
                    progress: vec![],
                    show: false,
                };
                game.remote_advancements(reset.clone());
                game.open_advancements(true);
                assert_eq!(game.adv_view().tab, Some(game.adv.tree.by_id[&first.id]));
                for _ in 0..2 {
                    assert_eq!(
                        receive_selection(),
                        SeenAdvancements {
                            action: 0,
                            tab: Some(first.id.clone())
                        }
                    );
                }
                game.select_advancement_tab(game.adv.tree.by_id[&second.id]);
                assert_eq!(
                    receive_selection(),
                    SeenAdvancements {
                        action: 0,
                        tab: Some(second.id.clone())
                    }
                );
                game.close_advancements();
                assert_eq!(
                    receive_selection(),
                    SeenAdvancements {
                        action: 1,
                        tab: None
                    }
                );

                game.remote_advancements(UpdateAdvancements {
                    reset: false,
                    added: vec![],
                    removed: vec![second.id.clone()],
                    progress: vec![],
                    show: false,
                });
                assert_eq!(game.adv.selected_tab.as_deref(), Some(second.id.as_str()));
                game.remote_advancements(reset);
                game.open_advancements(true);
                assert_eq!(game.adv_view().tab, Some(game.adv.tree.by_id[&first.id]));
                for _ in 0..2 {
                    assert_eq!(
                        receive_selection(),
                        SeenAdvancements {
                            action: 0,
                            tab: Some(first.id.clone())
                        }
                    );
                }
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn remote_tab_updates_restore_pan_and_clear_hover_on_progress_only() {
    in_own_process(
        "cut_layout::game::client_side::advancements_screen::remote_tests::remote_tab_updates_restore_pan_and_clear_hover_on_progress_only",
        || {
            with_client_game(|game| {
                game.init_remote_for_replay().unwrap();
                let first = displayed("test:first", None, 0.0, 0.0);
                let other = displayed("test:other", None, 0.0, 0.0);
                let far = displayed("test:far", Some(&first.id), 20.0, 10.0);
                game.remote_advancements(UpdateAdvancements {
                    reset: true,
                    added: vec![first.clone(), other.clone(), far.clone()],
                    removed: vec![],
                    progress: vec![],
                    show: false,
                });
                game.open_advancements(true);
                let mut batch = HudBatch::new([640, 480], game.client.settings.gui_scale_inputs());
                game.draw_advancements(&mut batch);
                assert_eq!(
                    game.client.adv_ui.0.tab_state().unwrap().scroll,
                    Some((-177.0, -92.0))
                );
                assert!(game.advancements_scroll(10.0, 5.0));
                assert_eq!(
                    game.client.adv_ui.0.tab_state().unwrap().scroll,
                    Some((-17.0, -12.0))
                );
                game.select_advancement_tab(game.adv.tree.by_id[&other.id]);

                game.remote_advancements(UpdateAdvancements {
                    reset: false,
                    added: vec![],
                    removed: vec![other.id, far.id],
                    progress: vec![],
                    show: false,
                });
                let view = game.adv_view();
                let first_index = game.adv.tree.by_id[&first.id];
                assert_eq!(view.tab, Some(first_index));
                assert_eq!(view.widgets, [first_index]);
                let tab = game.client.adv_ui.0.tab_state().unwrap();
                assert_eq!(tab.scroll, Some((-17.0, -12.0)));
                assert_eq!(tab.bounds, TabBounds(0, 0, 588, 297));

                let (left, top) = window(batch.width, batch.height);
                game.client.cursor = (left + 13.0, top + 22.0);
                game.client.adv_ui.0.last_frame =
                    Some(Instant::now() - std::time::Duration::from_secs(1));
                game.adv_tick_hover(&view, -17, -12, left, top);
                let tab = game.client.adv_ui.0.tab_state().unwrap();
                assert_eq!(tab.hovered, Some(first_index));
                assert!(tab.fade > 0.0);
                game.remote_advancements(UpdateAdvancements {
                    reset: false,
                    added: vec![],
                    removed: vec![],
                    progress: vec![(first.id, vec![("c".into(), Some(0))])],
                    show: false,
                });
                let tab = game.client.adv_ui.0.tab_state().unwrap();
                assert_eq!(tab.hovered, None);
                assert_eq!(tab.fade.to_bits(), 0.0f32.to_bits());
                assert_eq!(tab.scroll, Some((-17.0, -12.0)));
                assert!(game.advancements_scroll(-1.0, 0.0));
                assert_eq!(
                    game.client.adv_ui.0.tab_state().unwrap().scroll,
                    Some((-33.0, -12.0))
                );
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn repeated_remote_ids_keep_insertion_edges_and_take_packet_order_positions() {
    in_own_process(
        "cut_layout::game::client_side::advancements_screen::remote_tests::repeated_remote_ids_keep_insertion_edges_and_take_packet_order_positions",
        || {
            with_client_game(|game| {
                game.init_remote_for_replay().unwrap();
                let a = displayed("test:a", None, 0.0, 0.0);
                let b = displayed("test:b", None, 0.0, 0.0);
                let child = displayed("test:child", Some(&a.id), 1.0, 0.5);
                let old_descendant = displayed("test:old_descendant", Some(&child.id), 3.0, 1.0);
                let new_descendant = displayed("test:new_descendant", Some(&child.id), 4.0, 2.0);
                let ordered = displayed("test:ordered", Some(&a.id), 5.0, 0.5);
                game.remote_advancements(UpdateAdvancements {
                    reset: true,
                    added: vec![
                        ordered.clone(), // waits for A, so its definition is admitted last
                        a.clone(),
                        b.clone(),
                        child.clone(),
                        old_descendant,
                        displayed(&child.id, Some(&b.id), 2.0, 0.75),
                        new_descendant.clone(),
                        displayed(&ordered.id, Some(&b.id), 6.0, 1.5),
                        displayed(&child.id, Some("test:missing"), 9.0, 2.5),
                    ],
                    removed: vec![],
                    progress: vec![],
                    show: false,
                });
                assert_eq!(
                    game.adv.tree.get(&child.id).unwrap().parent.as_deref(),
                    Some(b.id.as_str())
                );
                assert_eq!(position(game, &child), (9.0, 2.5));
                assert_eq!(
                    game.adv.tree.get(&ordered.id).unwrap().parent.as_deref(),
                    Some(a.id.as_str())
                );
                assert_eq!(position(game, &ordered), (6.0, 1.5));

                for (root, remaining) in [
                    (a.id, vec![b.id.clone(), new_descendant.id]),
                    (b.id, vec![]),
                ] {
                    game.remote_advancements(UpdateAdvancements {
                        reset: false,
                        added: vec![],
                        removed: vec![root],
                        progress: vec![],
                        show: false,
                    });
                    assert_eq!(game.ui.registries.advancements, remaining);
                }
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn remote_widget_keeps_progress_requirements_until_new_progress() {
    in_own_process(
        "cut_layout::game::client_side::advancements_screen::remote_tests::remote_widget_keeps_progress_requirements_until_new_progress",
        || {
            with_client_game(|game| {
                game.init_remote_for_replay().unwrap();
                let root = displayed("test:root", None, 0.0, 0.0);
                let mut task = displayed("test:task", Some(&root.id), 1.0, 1.0);
                task.requirements =
                    vec![vec!["old_a".into(), "old_b".into()], vec!["old_c".into()]];
                game.remote_advancements(UpdateAdvancements {
                    reset: true,
                    added: vec![root, task.clone()],
                    removed: vec![],
                    progress: vec![(
                        task.id.clone(),
                        vec![("old_b".into(), Some(0)), ("old_c".into(), Some(0))],
                    )],
                    show: false,
                });
                game.open_advancements(true);
                let task_index = game.adv.tree.by_id[&task.id];
                assert!(game.adv_done(task_index));
                assert_eq!(game.adv_counts(task_index), Some((2, 2)));
                game.remote_advancements(UpdateAdvancements {
                    reset: false,
                    added: vec![],
                    removed: vec![task.id.clone()],
                    progress: vec![],
                    show: false,
                });
                task.requirements = ["new_a", "new_b", "new_c"]
                    .map(|name| vec![name.into()])
                    .into();
                game.remote_advancements(UpdateAdvancements {
                    reset: false,
                    added: vec![task.clone()],
                    removed: vec![],
                    progress: vec![],
                    show: false,
                });
                let task_index = game.adv.tree.by_id[&task.id];
                assert!(game.adv_view().widgets.contains(&task_index));
                assert!(game.adv_done(task_index));
                assert_eq!(game.adv_counts(task_index), Some((2, 2)));
                game.remote_advancements(UpdateAdvancements {
                    reset: false,
                    added: vec![],
                    removed: vec![],
                    progress: vec![(
                        task.id.clone(),
                        vec![("new_b".into(), Some(0)), ("old_b".into(), Some(0))],
                    )],
                    show: false,
                });
                assert!(!game.adv_done(task_index));
                assert_eq!(game.adv_counts(task_index), Some((1, 3)));
                assert!(!game.adv.progress.has(&task.id, "old_b"));
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn extreme_remote_positions_use_java_widget_coordinates_and_bounds() {
    in_own_process(
        "cut_layout::game::client_side::advancements_screen::remote_tests::extreme_remote_positions_use_java_widget_coordinates_and_bounds",
        || {
            with_client_game(|game| {
                game.init_remote_for_replay().unwrap();
                game.open_advancements(true);
                let apply = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>, added| {
                    let mut body = Writer::new();
                    UpdateAdvancements::write(
                        &UpdateAdvancements {
                            reset: false,
                            added,
                            removed: vec![],
                            progress: vec![],
                            show: false,
                        },
                        &mut body,
                    );
                    let packet = UpdateAdvancements::read(&mut Reader::new(&body.data)).unwrap();
                    game.apply_packet(Clientbound::UpdateAdvancements(packet))
                        .unwrap();
                };
                let root = displayed("test:extreme", None, f32::MAX, f32::INFINITY);
                apply(game, vec![root.clone()]);
                let view = game.adv_view();
                assert_eq!(view.tab, Some(0));
                assert_eq!(
                    game.client.adv_ui.0.cache.as_ref().unwrap().pos,
                    [Some((i32::MAX, i32::MAX))]
                );
                assert_eq!(
                    game.client.adv_ui.0.tab_state().unwrap().bounds,
                    TabBounds(i32::MAX, i32::MAX, -2_147_483_621, -2_147_483_622)
                );
                let mut batch = HudBatch::new([640, 480], game.client.settings.gui_scale_inputs());
                game.draw_advancements(&mut batch);
                assert_eq!(
                    game.client.adv_ui.0.tab_state().unwrap().scroll,
                    Some((104.0, 44.0))
                );

                apply(
                    game,
                    vec![
                        displayed("test:neg", Some(&root.id), f32::NEG_INFINITY, f32::NAN),
                        displayed("test:fraction", Some(&root.id), -0.03125, -0.0625),
                    ],
                );
                assert_eq!(
                    game.client.adv_ui.0.cache.as_ref().unwrap().pos,
                    [
                        Some((i32::MAX, i32::MAX)),
                        Some((i32::MIN, 0)),
                        Some((-1, -2))
                    ]
                );
                assert_eq!(
                    game.client.adv_ui.0.tab_state().unwrap().bounds,
                    TabBounds(i32::MIN, -2, 27, 27)
                );
                game.draw_advancements(&mut batch);
                assert_eq!(
                    game.client.adv_ui.0.tab_state().unwrap().scroll,
                    Some((104.0, 44.0))
                );
            });
        },
    );
}
