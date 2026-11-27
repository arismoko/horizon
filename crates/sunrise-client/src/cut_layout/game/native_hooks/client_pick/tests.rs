use mod_api::{ClientHost, ClientPickEvent, CrosshairTarget, Hooks};
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub(crate) struct Policy {
    pub(in crate::game) override_result: Option<Option<CrosshairTarget>>,
    pub(in crate::game) calls: usize,
}
pub(crate) struct First;
pub(crate) struct Last(pub(crate) Arc<Mutex<Policy>>);
#[mod_api::__subscriptions]
impl Hooks for First {
    fn client_pick(&self, host: &mut dyn ClientHost, event: &mut ClientPickEvent<'_>) {
        assert_eq!(host.crosshair_target(), event.original);
        *event.result = event.block_candidate;
    }
}
#[mod_api::__subscriptions]
impl Hooks for Last {
    fn client_pick(&self, host: &mut dyn ClientHost, event: &mut ClientPickEvent<'_>) {
        assert_eq!(host.crosshair_target(), event.original);
        assert_eq!(
            *event.result, event.block_candidate,
            "the second listener sees the first's write"
        );
        let mut policy = self.0.lock().unwrap();
        policy.calls += 1;
        if let Some(result) = policy.override_result {
            *event.result = result;
        }
    }
}

pub(crate) fn finish_login(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    server: &mut sunrise_protocol::connection::Connection,
) {
    use sunrise_protocol::packets::{Empty, configuration as cfg, login};
    server.recv().unwrap();
    server.recv().unwrap();
    server
        .send(
            &login::Clientbound::LoginFinished(login::LoginFinished {
                profile: login::GameProfile {
                    uuid: 42,
                    name: "ClientPickTest".into(),
                    properties: vec![],
                },
                session: 1,
            })
            .encode(),
        )
        .unwrap();
    server.recv().unwrap();
    server.recv().unwrap();
    server.recv().unwrap();
    server
        .send(&cfg::Clientbound::FinishConfiguration(Empty).encode())
        .unwrap();
    game.acknowledge_test_configuration();
    server.recv().unwrap();
}

pub(crate) struct SelectedPick(pub(crate) Arc<Mutex<Option<CrosshairTarget>>>);
#[mod_api::__subscriptions]
impl Hooks for SelectedPick {
    fn client_pick(&self, _: &mut dyn ClientHost, event: &mut ClientPickEvent<'_>) {
        if let Some(result) = *self.0.lock().unwrap() {
            *event.result = Some(result);
        }
    }
}
pub(crate) struct UnchangedAttack;
#[mod_api::__subscriptions]
impl Hooks for UnchangedAttack {
    fn client_attack_target(
        &self,
        _: &mut dyn ClientHost,
        _: &mut mod_api::ClientAttackTargetEvent<'_>,
    ) -> mod_api::Flow {
        mod_api::Flow::Continue
    }
}

#[allow(unused_imports)]
pub(crate) use crate::game::client_side::native_hooks_client::client_pick::tests::with_pick_game;

pub(crate) fn ranged_stack() -> sunrise_items::items::ItemStack {
    use sunrise_core::nbt::Tag;
    let mut stack = sunrise_items::items::ItemStack::new("minecraft:stick", 1);
    stack.data_mut().set_other(
        "minecraft:attack_range",
        Some(Tag::Compound(
            [
                ("min_reach".into(), Tag::Float(0.0f32.to_bits())),
                ("max_reach".into(), Tag::Float(4.5f32.to_bits())),
                ("hitbox_margin".into(), Tag::Float(0.0f32.to_bits())),
            ]
            .into_iter()
            .collect(),
        )),
    );
    stack
}

pub(crate) fn sent_actions(
    game: &sunrise_client_gameplay::game::ClientGame<'_>,
    server: &mut sunrise_protocol::connection::Connection,
) -> Vec<(&'static str, i32)> {
    let mut actions = sent_packets(game, server);
    actions.retain(|(kind, _)| *kind != "swing");
    actions
}

pub(crate) fn sent_packets(
    game: &sunrise_client_gameplay::game::ClientGame<'_>,
    server: &mut sunrise_protocol::connection::Connection,
) -> Vec<(&'static str, i32)> {
    use sunrise_protocol::packets::{configuration::Ping, play::Serverbound as S};
    // An attack can already have queued Punch. Fence with a distinct packet so
    // that attack's swing cannot terminate observation before the following use.
    game.send(S::Pong(Ping { id: i32::MIN }));
    let mut actions = Vec::new();
    loop {
        let (id, payload) = server.recv().unwrap();
        match S::decode(id, &payload).unwrap() {
            S::Attack(hit) => actions.push(("attack", hit.id)),
            S::Interact(hit) if hit.hand == 0 => actions.push(("interact", hit.id)),
            S::Interact(hit) => actions.push(("interact_off", hit.id)),
            S::UseItem(_) => actions.push(("item", 0)),
            S::UseItemOn(hit) => actions.push(("block", hit.pos[0])),
            S::Punch(_) => actions.push(("swing", 0)),
            S::Pong(ping) if ping.id == i32::MIN => return actions,
            _ => {}
        }
    }
}

#[test]
#[ignore = "daily"]
fn unchanged_attack_listener_preserves_the_committed_point_through_a_wall() {
    for subscribe_attack in [false, true] {
        let selected = Arc::new(Mutex::new(None));
        let mut mods = vec![mod_api::ModEntry::new(
            "pick",
            Box::new(SelectedPick(selected.clone())),
        )];
        if subscribe_attack {
            mods.push(mod_api::ModEntry::new("attack", Box::new(UnchangedAttack)));
        }
        mod_platform::testing::with_mods(mods.into(), || {
            with_pick_game("minecraft:pig", 2.5, |game, server| {
                let entity = game.entity(31).unwrap();
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(1, 2, 0, stone).unwrap();
                let target = CrosshairTarget::Entity {
                    entity,
                    point: [2.25, 2.6, 0.5],
                };
                *selected.lock().unwrap() = Some(target);
                game.refresh_client_pick(1.0);
                assert_eq!(game.crosshair_target(), Some(target));
                assert!(
                    game.targeting_entity_hit(entity).is_none(),
                    "a new occlusion ray would reject it"
                );
                game.remote_start_attack();
                assert_eq!(
                    sent_actions(game, server),
                    [("attack", 31)],
                    "a no-op attack listener must not change the selected attack"
                );
            })
        });
    }
}

#[test]
#[ignore = "daily"]
fn attack_range_indicator_obeys_block_and_miss_replacements() {
    let policy = Arc::new(Mutex::new(Policy::default()));
    let mods = vec![
        mod_api::ModEntry::new("a_pick", Box::new(First)),
        mod_api::ModEntry::new("b_pick", Box::new(Last(policy.clone()))),
    ]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_pick_game("minecraft:pig", 3.5, |game, _| {
            game.inventory[0] = Some(ranged_stack());
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(4, 2, 0, stone).unwrap();
            game.refresh_client_pick(1.0);
            assert!(matches!(
                game.crosshair_target(),
                Some(CrosshairTarget::Block { .. })
            ));
            assert!(
                game.crosshair_living().is_none(),
                "no raw attack-range indicator behind the selected block"
            );
            policy.lock().unwrap().override_result = Some(None);
            game.refresh_client_pick(1.0);
            assert!(
                game.crosshair_living().is_none(),
                "no raw attack-range indicator on a selected miss"
            );
        })
    });
}

#[test]
#[ignore = "daily"]
fn forward_moving_pick_survives_a_noop_listener_and_extended_replacements() {
    let mut vanilla_point = None;
    for subscribe in [false, true] {
        let selected = Arc::new(Mutex::new(None));
        let mods = if subscribe {
            vec![mod_api::ModEntry::new(
                "pick",
                Box::new(SelectedPick(selected.clone())),
            )]
        } else {
            Vec::new()
        };
        mod_platform::testing::with_mods(mods.into(), || {
            with_pick_game("minecraft:pig", 5.5, |game, _| {
                game.inventory[0] = Some(ranged_stack());
                game.previous_position.x -= 1.0;
                game.refresh_client_pick(1.0);
                let Some(CrosshairTarget::Entity { entity, point }) = game.crosshair_target()
                else {
                    panic!("the moving attack-range ray reaches this pig");
                };
                assert_eq!(entity.id(), 31);
                if let Some(vanilla) = vanilla_point {
                    assert_eq!(
                        point, vanilla,
                        "a no-op listener keeps the exact original hit point"
                    );
                } else {
                    vanilla_point = Some(point);
                }
                assert!(game.pick_distance(point) > 4.5);
                if subscribe {
                    let replacement = CrosshairTarget::Entity {
                        entity,
                        point: [5.25, 2.6, 0.5],
                    };
                    *selected.lock().unwrap() = Some(replacement);
                    game.refresh_client_pick(1.0);
                    assert_eq!(
                        game.crosshair_target(),
                        Some(replacement),
                        "replacement uses the movement-extended envelope too"
                    );
                }
            })
        });
    }
}

#[test]
#[ignore = "daily"]
fn attack_range_pick_outside_interact_range_falls_through_to_item_use() {
    for kind in ["minecraft:pig", "minecraft:item_frame"] {
        let selected = Arc::new(Mutex::new(None));
        let mods = vec![mod_api::ModEntry::new(
            "pick",
            Box::new(SelectedPick(selected.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_pick_game(kind, 4.5, |game, server| {
                game.inventory[0] = Some(ranged_stack());
                let entity = game.entity(31).unwrap();
                let target = CrosshairTarget::Entity {
                    entity,
                    point: [4.5, 2.6, 0.5],
                };
                *selected.lock().unwrap() = Some(target);
                game.refresh_client_pick(1.0);
                assert_eq!(game.crosshair_target(), Some(target));
                game.remote_use(mod_api::InteractionAttempt::Press);
                let actions = sent_actions(game, server);
                assert!(!actions.iter().any(|(kind, _)| *kind == "interact"));
                assert!(
                    actions.iter().any(|(kind, _)| *kind == "item"),
                    "the stored entity passes to item use, never a block behind it"
                );
            })
        });
    }
}

/// A cushion the stored pick selected, spawned where `with_pick_game` puts an
/// entity and raised to the player's eye so the look ray crosses it.
pub(crate) fn with_picked_cushion(
    run: impl FnOnce(
        &mut sunrise_client_gameplay::game::ClientGame<'_>,
        &mut sunrise_protocol::connection::Connection,
        CrosshairTarget,
    ),
) {
    let selected = Arc::new(Mutex::new(None));
    let mods = vec![mod_api::ModEntry::new(
        "pick",
        Box::new(SelectedPick(selected.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_pick_game("minecraft:cushion", 2.5, |game, server| {
            game.cushions.update(0, |c| c.position.y = 2.5);
            game.inventory[0] = Some(sunrise_items::items::ItemStack::new("minecraft:stick", 1));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(3, 2, 0, stone).unwrap();
            let target = CrosshairTarget::Entity {
                entity: game.entity(31).unwrap(),
                point: [2.5, 2.6, 0.5],
            };
            *selected.lock().unwrap() = Some(target);
            run(game, server, target)
        })
    });
}

/// The cushion's position comes off the wire as raw doubles. A NaN there
/// leaves it no box: the click is refused, never a panic in the reach check.
#[test]
#[ignore = "daily"]
fn a_click_on_a_cushion_at_a_non_finite_position_is_refused() {
    with_picked_cushion(|game, server, target| {
        game.cushions.update(0, |c| c.position.x = f64::NAN);
        game.refresh_client_pick(1.0);
        assert_eq!(game.crosshair_target(), Some(target));
        game.remote_use(mod_api::InteractionAttempt::Press);
        assert!(
            !sent_actions(game, server)
                .iter()
                .any(|(kind, _)| *kind == "interact"),
            "a cushion without a box is not interacted with"
        );
    });
}

/// `Minecraft.startUseItem` on a free cushion in the crosshair: the click is
/// the cushion's (`Cushion.interact` answers CONSUME on a client), so exactly
/// one main-hand Interact goes out and nothing reaches the block behind it, the
/// held item or the arm.
#[test]
#[ignore = "daily"]
fn a_free_cushion_takes_the_click_and_sends_one_interact() {
    with_picked_cushion(|game, server, target| {
        game.refresh_client_pick(1.0);
        assert_eq!(game.crosshair_target(), Some(target));
        game.remote_use(mod_api::InteractionAttempt::Press);
        assert_eq!(sent_packets(game, server), [("interact", 31)]);
    });
}

/// `Cushion.interact` passes when anything already sits on it, whatever kind
/// of entity the server lists as its passenger: the click goes on to the item.
#[test]
#[ignore = "daily"]
fn a_click_on_a_cushion_with_a_synced_passenger_passes_to_the_item() {
    use sunrise_protocol::packets::play::{self, Clientbound as C};
    with_picked_cushion(|game, server, target| {
        game.refresh_client_pick(1.0);
        assert_eq!(game.crosshair_target(), Some(target));
        game.spawn_entity(77, "minecraft:marker", [0.5, 70.0, 0.5], [0.0; 3]);
        game.handle_packet(C::SetPassengers(play::SetPassengers {
            vehicle: 31,
            passengers: vec![77],
        }))
        .unwrap();
        game.remote_use(mod_api::InteractionAttempt::Press);
        let actions = sent_actions(game, server);
        assert!(
            actions.contains(&("item", 0)),
            "an occupied cushion does not consume the click: {actions:?}"
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_click_on_a_free_cushion_while_sneaking_passes_to_the_item() {
    with_picked_cushion(|game, server, target| {
        game.client.remote.as_mut().unwrap().last_input |=
            sunrise_gameplay_common::game::remote::SHIFT_INPUT;
        game.refresh_client_pick(1.0);
        assert_eq!(game.crosshair_target(), Some(target));
        game.remote_use(mod_api::InteractionAttempt::Press);
        let packets = sent_packets(game, server);
        assert!(packets.contains(&("item", 0)), "{packets:?}");
        assert!(
            !packets
                .iter()
                .any(|(kind, _)| matches!(*kind, "block" | "swing")),
            "{packets:?}"
        );
    });
}

pub(crate) struct CountPicks(pub(crate) Arc<Mutex<Vec<f32>>>);
#[mod_api::__subscriptions]
impl Hooks for CountPicks {
    fn client_pick(&self, _: &mut dyn ClientHost, event: &mut ClientPickEvent<'_>) {
        self.0.lock().unwrap().push(event.partial_tick);
    }
}

#[test]
#[ignore = "daily"]
fn tick_pick_survives_creative_break_then_use_until_the_next_pick() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        with_pick_game("minecraft:pig", 10.0, |game, server| {
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(2, 2, 0, stone).unwrap();
            game.world.set(3, 2, 0, stone).unwrap();
            game.creative = true;
            game.refresh_client_pick(1.0);
            assert_eq!(game.pick().unwrap().pos, [2, 2, 0]);
            game.remote_start_attack();
            assert!(
                game.res.facts().is_air(game.world.block(2, 2, 0)),
                "creative break is predicted"
            );
            assert_eq!(
                game.pick().unwrap().pos,
                [2, 2, 0],
                "the same tick still uses the old hit"
            );
            game.inventory[0] = Some(sunrise_items::items::ItemStack::new("minecraft:stick", 1));
            game.remote_use(mod_api::InteractionAttempt::Press);
            assert!(
                sent_actions(game, server).contains(&("block", 2)),
                "use sends the stored hit, not the block behind"
            );
            game.refresh_client_pick(1.0);
            assert_eq!(
                game.pick().unwrap().pos,
                [3, 2, 0],
                "the next tick picks again"
            );
        })
    });
}

#[test]
#[ignore = "daily"]
fn paused_ticks_pick_and_the_frame_can_overwrite_the_tick_target() {
    use sunrise_core::geometry::Vec3;
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new(
        "count",
        Box::new(CountPicks(calls.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_pick_game("minecraft:pig", 10.0, |game, _| {
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(2, 2, 0, stone).unwrap();
            game.world.set(2, 2, 2, stone).unwrap();
            game.previous_position = Vec3::new(0.5, 1.0, 2.5);
            game.pause = true;
            game.client_tick_steps(2, None).unwrap();
            assert_eq!(
                *calls.lock().unwrap(),
                [1.0, 1.0],
                "paused client ticks still pick exactly once"
            );
            assert_eq!(
                game.pick().unwrap().pos,
                [2, 2, 0],
                "tick uses current eyes"
            );
            game.refresh_client_pick(0.0);
            assert_eq!(
                game.pick().unwrap().pos,
                [2, 2, 2],
                "highlight and crosshair read the interpolated frame pick"
            );
            assert!(
                matches!(game.crosshair_target(), Some(CrosshairTarget::Block { pos, .. }) if pos.z == 2)
            );
            assert_eq!(*calls.lock().unwrap(), [1.0, 1.0, 0.0]);
        })
    });
}

pub(crate) struct OverwriteOrigin;
#[mod_api::__subscriptions]
impl Hooks for OverwriteOrigin {
    fn client_pick(&self, _: &mut dyn ClientHost, event: &mut ClientPickEvent<'_>) {
        event.origin = [0.0; 3];
    }
}

pub(crate) struct RecordOrigins(pub(crate) Arc<Mutex<Vec<[f64; 3]>>>);
#[mod_api::__subscriptions]
impl Hooks for RecordOrigins {
    fn client_pick(&self, _: &mut dyn ClientHost, event: &mut ClientPickEvent<'_>) {
        self.0.lock().unwrap().push(event.origin);
    }
}

#[test]
#[ignore = "daily"]
fn pick_origin_is_the_eye_used_for_each_pick_and_isolated_per_listener() {
    use sunrise_core::geometry::Vec3;
    let origins = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![
        mod_api::ModEntry::new("a_overwrite", Box::new(OverwriteOrigin)),
        mod_api::ModEntry::new("b_record", Box::new(RecordOrigins(origins.clone()))),
    ]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_pick_game("minecraft:pig", 10.0, |game, _| {
            let player_eye = f64::from(game.player.as_ref().unwrap().dimensions.eye_height);
            game.previous_position = Vec3::new(4.5, 5.0, 8.5);
            game.refresh_client_pick(1.0);
            game.refresh_client_pick(0.25);

            let mob = game.mobs.iter_mut().find(|mob| mob.id == 31).unwrap();
            mob.previous = Vec3::new(6.0, 4.0, -1.5);
            let mob_eye = f64::from(mob.eye_height() as f32);
            game.remote_set_camera(31);
            assert_eq!(game.remote_camera_id(), Some(31));
            game.refresh_client_pick(0.25);

            assert_eq!(
                *origins.lock().unwrap(),
                [
                    [0.5, 1.0 + player_eye, 0.5],
                    [3.5, 4.0 + player_eye, 6.5],
                    [7.0, 3.5 + mob_eye, -1.0],
                ],
                "a listener receives the sampled camera eye, not a prior listener's edit"
            );
        });
    });
}

pub(crate) struct RiderPart(pub(crate) Arc<Mutex<usize>>, pub(crate) bool);
#[mod_api::sunrise_part]
impl mod_api::EntityPart for RiderPart {
    fn can_rider_interact(&self, event: &mut mod_api::CanRiderInteractEvent<'_>) {
        let entity = event
            .subject
            .with_view(|view| view.handle())
            .expect("live resident entity");
        assert_eq!(entity.id(), 31);
        *self.0.lock().unwrap() += 1;
        event.allowed = true;
        if self.1 {
            assert!(event.subject.host().set_camera_entity(entity));
        }
    }
}
pub(crate) struct RegisterRiderPart(pub(crate) Arc<Mutex<usize>>, pub(crate) bool);
#[mod_api::__subscriptions]
impl Hooks for RegisterRiderPart {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        r.entity_part("minecraft:pig", RiderPart(self.0.clone(), self.1));
    }
}

#[test]
#[ignore = "daily"]
fn rider_pick_refuses_a_camera_changed_by_its_listener() {
    use sunrise_protocol::packets::play::{self, Clientbound as C};
    let calls = Arc::new(Mutex::new(0));
    let mods = vec![mod_api::ModEntry::new(
        "rider_camera",
        Box::new(RegisterRiderPart(calls.clone(), true)),
    )];
    mod_platform::testing::with_mods(mods.into(), || {
        with_pick_game("minecraft:pig", 1.5, |game, _| {
            let kind = game
                .client
                .remote
                .as_ref()
                .unwrap()
                .builtin
                .get("minecraft:entity_type")
                .unwrap()
                .id("minecraft:oak_boat")
                .unwrap();
            game.handle_packet(C::AddEntity(play::AddEntity {
                id: 32,
                uuid: 32,
                kind,
                position: [0.5, 1.0, 0.5],
                velocity: [0.0; 3],
                pitch: 0,
                yaw: 0,
                head_yaw: 0,
                data: 0,
            }))
            .unwrap();
            game.remote_set_passengers(32, vec![7, 31]);
            assert_eq!(game.pick_camera_id(), Some(7));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(3, 2, 0, stone).unwrap();
            game.refresh_client_pick(1.0);
            assert_eq!(*calls.lock().unwrap(), 1);
            assert_eq!(game.pick_camera_id(), Some(31));
            assert!(game.crosshair_target().is_none());
            assert!(game.mob_hit().is_none());
            assert!(!game.entity_targeted());
        });
    });
}

pub(crate) struct SelectOccludedEntity;
#[mod_api::__subscriptions]
impl Hooks for SelectOccludedEntity {
    fn client_pick(&self, _host: &mut dyn ClientHost, event: &mut ClientPickEvent<'_>) {
        assert!(
            matches!(event.original, Some(CrosshairTarget::Block { .. })),
            "vanilla original keeps the closer block"
        );
        assert!(
            matches!(event.entity_candidate, Some(CrosshairTarget::Entity { .. })),
            "ClientPick supplies the reachable entity candidate independently of the closer block"
        );
        *event.result = event.entity_candidate;
    }
}

#[test]
#[ignore = "daily"]
fn client_pick_listener_can_select_pig_occluded_by_closer_block() {
    let mods = vec![mod_api::ModEntry::new(
        "select_occluded_entity",
        Box::new(SelectOccludedEntity),
    )];
    mod_platform::testing::with_mods(mods.into(), || {
        with_pick_game("minecraft:pig", 2.5, |game, _| {
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(1, 2, 0, stone).unwrap();
            game.refresh_client_pick(1.0);
            assert!(
                matches!(
                    game.crosshair_target(),
                    Some(CrosshairTarget::Entity { entity, .. }) if entity.id() == 31
                ),
                "listener selected the pig behind the closer block"
            );
            assert_eq!(game.mob_hit().map(|(i, _, _)| game.mobs[i].id), Some(31));
        });
    });
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;
