#![cfg(test)]
use sunrise_gameplay_common::game::entity_sections::EntitySectionsGameExt1 as _;

#[cfg(test)]
pub(crate) mod tests {
    use crate::game::native_hooks::farmland_trample::*;
    use mod_api::*;
    use std::sync::{Arc, Mutex};
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    use sunrise_level::chunk::Chunk;
    use sunrise_protocol::packets::{Empty, play, play::Serverbound as S};
    use sunrise_sim::{lan::guest::InboxItem, motion::PlayerMotion};
    use {
        sunrise_gameplay::game::server_side::native_item_places::tests::open_to_lan,
        sunrise_gameplay_common::game::server_side::native_item_places::tests::guest,
    };

    pub(crate) const FIELD: [i32; 3] = [1, 63, 1];
    pub(crate) const LISTENER_WRITE: [i32; 3] = [3, 63, 3];
    pub(crate) const HEALTHY_WRITE: [i32; 3] = [4, 63, 4];
    pub(crate) const LANDING_Y: f64 = 63.9375;
    pub(crate) const LANDING: u32 = 20;
    pub(crate) const OTHER: u32 = 72;
    pub(crate) const OUTSIDE: u32 = 73;
    pub(crate) const WALKING: Body = Body::Guest { sneaking: false };

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum Choice {
        Absent,
        Allow,
        Trap,
        Cancel,
        CancelAndBreakCrop,
        AvoidSneak,
        Replace,
        ReplaceDuringLift,
        MoveBystander,
    }

    #[derive(Clone, Copy)]
    pub(crate) enum Body {
        Guest { sneaking: bool },
        Pig,
        Cat { crouching: bool },
    }

    #[derive(Debug, PartialEq)]
    pub(crate) enum Moment {
        Hook { y: f64, health: f32 },
        CropRemoved(mod_api::BlockState),
        Lift(u32, f64, Option<mod_api::BlockState>),
        Write(mod_api::BlockState),
        GameEvent([f64; 3], Option<u32>, Option<mod_api::BlockState>),
        DestroyGameEvent([f64; 3], Option<u32>, Option<mod_api::BlockState>),
        Fall(f32, f64, mod_api::BlockState),
    }

    pub(crate) type Trace = Arc<Mutex<Vec<Moment>>>;

    // This observer listens to the effects, not to farmland_trample. It can be
    // installed on both sides of the allow/no-listener comparison.
    pub(crate) struct Observe(pub(crate) Trace, pub(crate) Choice);
    #[mod_api::__subscriptions]
    impl Hooks for Observe {
        fn entity_entering_section(
            &self,
            host: HostSide<'_>,
            event: &mut EntityEnteringSectionEvent,
        ) {
            if let HostSide::Server(host) = host
                && event.old_section[1] == 3
                && event.new_section[1] == 4
            {
                let state = event
                    .entity
                    .level(host)
                    .and_then(|level| level.block_state(host, FIELD));
                self.0.lock().unwrap().push(Moment::Lift(
                    event.entity.id(),
                    event.entity.position(host).unwrap()[1],
                    state,
                ));
                if event.entity.id() == LANDING {
                    match self.1 {
                        Choice::ReplaceDuringLift => {
                            let level = event.entity.level(host).unwrap();
                            let key = host.block_key("minecraft:stone").unwrap();
                            let stone =
                                mod_api::BlockState::of(&host.block_states(), key, &[]).unwrap();
                            assert_eq!(
                                level.set_block(host, FIELD, stone, BlockUpdateFlags::KNOWN_SHAPE),
                                Ok(true)
                            );
                        }
                        Choice::MoveBystander => {
                            // This body was already selected. Raise it by 1/32 before
                            // its lift: using its old box would now overshoot the dirt.
                            let result = host.commands().run_command_with_output(
                                "tp @e[type=minecraft:pig,x=1.8,y=63.9375,z=1.5,distance=..0,limit=1] 1.8 63.96875 1.5",
                            );
                            assert!(result.success, "bystander teleport: {:?}", result.feedback);
                        }
                        _ => {}
                    }
                }
            }
        }
        fn block_changed(&self, level: &dyn LevelHost, event: &mut BlockChangedEvent) {
            if [event.pos.x, event.pos.y, event.pos.z] == FIELD {
                assert_eq!(level.block_state(FIELD), Some(event.new));
                self.0.lock().unwrap().push(Moment::Write(event.new));
            }
        }
        fn vanilla_game_event(
            &self,
            _: &mut dyn ServerHost,
            event: &mut VanillaGameEventEvent<'_>,
        ) -> Flow {
            if event.event == "minecraft:block_change" {
                self.0.lock().unwrap().push(Moment::GameEvent(
                    event.pos,
                    event.cause,
                    event.affected_state,
                ));
            } else if event.event == "minecraft:block_destroy"
                && self.1 == Choice::CancelAndBreakCrop
            {
                self.0.lock().unwrap().push(Moment::DestroyGameEvent(
                    event.pos,
                    event.cause,
                    event.affected_state,
                ));
            }
            Flow::Continue
        }
        fn living_incoming_damage(
            &self,
            level: &mut dyn HitHost,
            event: &mut LivingIncomingDamageEvent<'_>,
        ) -> Flow {
            if event.entity as u32 == LANDING && event.source.damage_type == "minecraft:fall" {
                self.0.lock().unwrap().push(Moment::Fall(
                    event.container.new_damage(),
                    event.victim.position[1],
                    level.block_state(FIELD).unwrap(),
                ));
            }
            Flow::Continue
        }
    }

    pub(crate) struct Gardener(pub(crate) Choice, pub(crate) Trace);
    #[mod_api::__subscriptions]
    impl Hooks for Gardener {
        fn farmland_trample(
            &self,
            host: &mut dyn ServerHost,
            event: &mut FarmlandTrampleEvent,
        ) -> Flow {
            let entity = event.entity.entity();
            assert_eq!(entity.id(), LANDING);
            assert_eq!([event.pos.x, event.pos.y, event.pos.z], FIELD);
            let level = entity.level(host).unwrap();
            assert_eq!(level.block_state(host, FIELD), Some(event.state));
            self.1.lock().unwrap().push(Moment::Hook {
                y: entity.position(host).unwrap()[1],
                health: event.entity.health(host).unwrap(),
            });
            if matches!(
                self.0,
                Choice::CancelAndBreakCrop | Choice::Trap | Choice::Replace
            ) {
                let key = host.block_key("minecraft:stone").unwrap();
                let stone = mod_api::BlockState::of(&host.block_states(), key, &[]).unwrap();
                assert_eq!(
                    level.set_block(host, LISTENER_WRITE, stone, BlockUpdateFlags::KNOWN_SHAPE),
                    Ok(true)
                );
                assert_eq!(level.block_state(host, LISTENER_WRITE), Some(stone));
            }
            match self.0 {
                Choice::Cancel => Flow::Cancel,
                Choice::CancelAndBreakCrop => {
                    let crop = [FIELD[0], FIELD[1] + 1, FIELD[2]];
                    assert_eq!(
                        level.destroy_block(host, crop, true, None, Some(entity), false),
                        Ok(Some(Vec::new()))
                    );
                    let state = level.block_state(host, crop).unwrap();
                    // The outer trace assertion distinguishes cancellation from a trapped assertion.
                    self.1.lock().unwrap().push(Moment::CropRemoved(state));
                    Flow::Cancel
                }
                Choice::AvoidSneak => {
                    if entity.is_shift_key_down(host).unwrap() || entity.is_crouching(host).unwrap()
                    {
                        Flow::Cancel
                    } else {
                        Flow::Continue
                    }
                }
                Choice::Replace => {
                    let key = host.block_key("minecraft:stone").unwrap();
                    let stone = mod_api::BlockState::of(&host.block_states(), key, &[]).unwrap();
                    assert_eq!(
                        level.set_block(host, FIELD, stone, BlockUpdateFlags::KNOWN_SHAPE),
                        Ok(true)
                    );
                    Flow::Continue
                }
                Choice::Trap => {
                    event.cancelled = true;
                    panic!("trample listener trapped");
                }
                Choice::Allow | Choice::ReplaceDuringLift => Flow::Continue,
                Choice::Absent | Choice::MoveBystander => unreachable!(),
            }
        }
    }

    pub(crate) struct AfterTrap;
    #[mod_api::__subscriptions]
    impl Hooks for AfterTrap {
        fn farmland_trample(
            &self,
            host: &mut dyn ServerHost,
            event: &mut FarmlandTrampleEvent,
        ) -> Flow {
            assert!(!event.cancelled);
            let level = event.entity.entity().level(host).unwrap();
            let air = host.block_key("minecraft:air").unwrap();
            let air = mod_api::BlockState::of(&host.block_states(), air, &[]).unwrap();
            assert_eq!(level.block_state(host, LISTENER_WRITE), Some(air));
            let stone = host.block_key("minecraft:stone").unwrap();
            let stone = mod_api::BlockState::of(&host.block_states(), stone, &[]).unwrap();
            assert_eq!(
                level.set_block(host, HEALTHY_WRITE, stone, BlockUpdateFlags::KNOWN_SHAPE),
                Ok(true)
            );
            Flow::Continue
        }
    }

    pub(crate) fn land(body: Body, choice: Choice) -> Vec<Moment> {
        land_with_later_packet(body, choice, None)
    }

    pub(crate) fn land_with_later_packet(
        body: Body,
        choice: Choice,
        later: Option<S>,
    ) -> Vec<Moment> {
        let player = matches!(body, Body::Guest { .. });
        let cancelled = matches!(choice, Choice::Cancel | Choice::CancelAndBreakCrop)
            || (choice == Choice::AvoidSneak
                && matches!(
                    body,
                    Body::Guest { sneaking: true } | Body::Cat { crouching: true }
                ));
        let converted =
            !cancelled && !matches!(choice, Choice::Replace | Choice::ReplaceDuringLift);
        let damage = if matches!(body, Body::Cat { .. }) {
            0.0
        } else {
            3.0
        };
        let trace = Trace::default();
        let mut mods = vec![ModEntry::new(
            "trample-observer",
            Box::new(Observe(trace.clone(), choice)),
        )];
        let listens = !matches!(choice, Choice::Absent | Choice::MoveBystander);
        if listens {
            mods.push(ModEntry::new(
                "gardener",
                Box::new(Gardener(choice, trace.clone())),
            ));
        }
        if choice == Choice::Trap {
            mods.push(ModEntry::new("after_trap", Box::new(AfterTrap)));
        }
        mod_platform::testing::with_mods(mods.into(), || {
            with_game(|game| {
                if player || choice == Choice::MoveBystander {
                    // The live-bounds case needs a server console for its teleport.
                    open_to_lan(game);
                } else {
                    game.options.dedicated = Some(Vec::new());
                }
                let (common, server) =
                    sunrise_gameplay_common::game::SideGame::common_and_state_mut(game);
                server
                    .streaming
                    .insert_remote(&mut common.world, Chunk::empty(0, 0));
                let reg = game.pack.assets.registry;
                let [farmland, dirt, wheat, air] = [
                    "minecraft:farmland",
                    "minecraft:dirt",
                    "minecraft:wheat",
                    "minecraft:air",
                ]
                .map(|name| reg.default_state(name).unwrap());
                let [x, y, z] = FIELD;
                game.world.set(x, y, z, farmland).unwrap();
                game.world.set(x, y + 1, z, wheat).unwrap();
                let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                let initial_health = if let Body::Guest { sneaking } = body {
                    let mut player = guest(1);
                    player.loaded = true;
                    player.loading = false;
                    // A real client has reported that it loaded the level.
                    // Otherwise ServerPlayer.isInvulnerableTo refuses fall damage.
                    player.load_timeout = 0;
                    player.sneaking = sneaking;
                    let start = [1.5, LANDING_Y + 0.25, 1.5];
                    player.place(start);
                    player.first_good = start;
                    let mut body =
                        PlayerMotion::new(game.motion, Vec3::new(start[0], start[1], start[2]))
                            .unwrap();
                    body.fall_distance = 6.0;
                    player.motion = Some(Box::new(body));
                    let health = player.health;
                    player.inbox.push_back(InboxItem::Packet(
                        S::MovePlayerPos(play::MovePlayerPos {
                            x: 1.5,
                            y: LANDING_Y,
                            z: 1.5,
                            flags: 1,
                        }),
                        None,
                    ));
                    player
                        .inbox
                        .push_back(InboxItem::Packet(S::ClientTickEnd(Empty), None));
                    if let Some(packet) = later.clone() {
                        player.inbox.push_back(InboxItem::Packet(packet, None));
                        player
                            .inbox
                            .push_back(InboxItem::Packet(S::ClientTickEnd(Empty), None));
                    }
                    game.server.lan.as_mut().unwrap().hub.guests.push(player);
                    game.entity_section_joined(LANDING, start);
                    health
                } else {
                    let mob_kind = if matches!(body, Body::Cat { .. }) {
                        game.res.mob_types().get("minecraft:cat").unwrap().clone()
                    } else {
                        kind.clone()
                    };
                    let mut mob = sunrise_sim::mobs::Mob::new(
                        LANDING,
                        mob_kind,
                        Vec3::new(1.5, 64.125, 1.5),
                        0.0,
                    );
                    // The subject falls vertically; no goal steers it off the field.
                    mob.ai.goals = Default::default();
                    mob.ai.targets = Default::default();
                    if let Body::Cat { crouching } = body {
                        // A normal cat cannot trample; a scaled one can. The
                        // cat's AI selects its stalking pose at move speed 0.6.
                        mob.extra.model_scale = 2.0;
                        if crouching {
                            mob.ai.mov.set_wanted(1.5, 64.125, 1.5, 0.6);
                        }
                    }
                    mob.fall_distance = 6.0;
                    mob.velocity.y = -0.5;
                    let health = mob.health;
                    game.mobs.push(mob);
                    game.entity_section_joined(LANDING, [1.5, 64.125, 1.5]);
                    health
                };
                for (id, x) in [(OTHER, 1.8), (OUTSIDE, 3.5)] {
                    let mut other = sunrise_sim::mobs::Mob::new(
                        id,
                        kind.clone(),
                        Vec3::new(x, LANDING_Y, 1.5),
                        0.0,
                    );
                    other.no_ai = true;
                    game.mobs.push(other);
                    game.entity_section_joined(id, [x, LANDING_Y, 1.5]);
                }
                let drops = game.dropped.len();
                if player {
                    game.tick_lan_packets();
                    let guest = game.server.lan.as_ref().unwrap().hub.guest(1).unwrap();
                    assert_eq!(
                        guest.moves.len(),
                        usize::from(!converted),
                        "only a committed lift discards the saved landing travel"
                    );
                    if matches!(later, Some(S::PlayerInput(_))) {
                        assert!(!guest.sneaking, "the later release still runs");
                    }
                    if matches!(later, Some(S::MovePlayerPos(_))) {
                        // ServerPlayer.teleportRelative lifts the player onto dirt. A
                        // later move without its teleport confirmation cannot move them.
                        assert_eq!(
                            guest.awaiting_teleport.map(|(_, at)| at),
                            Some([1.5, 64.0, 1.5])
                        );
                    }
                } else {
                    game.tick_creature_entities().unwrap();
                }
                if choice == Choice::Cancel {
                    assert_eq!(game.dropped.len(), drops);
                } else if choice == Choice::CancelAndBreakCrop {
                    assert!(
                        game.dropped.len() > drops,
                        "destroying the crop in a cancelling trample listener spawns its drops"
                    );
                    assert!(
                        game.dropped[drops..]
                            .iter()
                            .any(|item| item.stack.item.as_ref() == "minecraft:wheat_seeds")
                    );
                }
                let (landing_y, health) = if player {
                    let g = game.server.lan.as_ref().unwrap().hub.guest(1).unwrap();
                    (g.position[1], g.health)
                } else {
                    let m = game.mob_object(LANDING).unwrap();
                    (m.position.y, m.health)
                };
                let stone = reg.default_state("minecraft:stone").unwrap();
                let floor = if converted {
                    dirt
                } else if choice == Choice::Replace {
                    stone
                } else {
                    farmland
                };
                let y_after = if converted { 64.0 } else { LANDING_Y };
                assert_eq!(game.world.block(x, y, z), floor);
                if choice == Choice::CancelAndBreakCrop {
                    assert_eq!(game.world.block(x, y + 1, z), air);
                } else if !converted {
                    assert_eq!(game.world.block(x, y + 1, z), wheat);
                }
                let [wx, wy, wz] = LISTENER_WRITE;
                let expected_write =
                    if matches!(choice, Choice::CancelAndBreakCrop | Choice::Replace) {
                        stone
                    } else {
                        air
                    };
                assert_eq!(
                    game.world.block(wx, wy, wz),
                    expected_write,
                    "completed listener writes stay; trapped writes roll back"
                );
                if choice == Choice::Trap {
                    let [wx, wy, wz] = HEALTHY_WRITE;
                    assert_eq!(
                        game.world.block(wx, wy, wz),
                        stone,
                        "the later listener's successful write still commits"
                    );
                }
                assert_eq!(landing_y, y_after);
                assert_eq!(game.mob_object(OTHER).unwrap().position.y, y_after);
                assert_eq!(game.mob_object(OUTSIDE).unwrap().position.y, LANDING_Y);
                assert_eq!(
                    health,
                    initial_health - damage,
                    "trace: {:?}",
                    *trace.lock().unwrap()
                );

                let api_state =
                    |state: sunrise_world::registry::BlockStateId| -> mod_api::BlockState {
                        use sunrise_gameplay_common::game::api_values::TryIntoApi;
                        state.try_into_api().unwrap()
                    };
                // With no trample listener, this pins the 26.3 order from
                // FarmlandBlock.turnToBaseBlock and fallOn. The farmland's top is
                // 15/16 high; replacing it with a full cube raises both bodies to 64.
                let mut expected = Vec::new();
                if listens {
                    expected.push(Moment::Hook {
                        y: LANDING_Y,
                        health: initial_health,
                    });
                }
                if choice == Choice::CancelAndBreakCrop {
                    expected.push(Moment::CropRemoved(api_state(air)));
                    expected.push(Moment::DestroyGameEvent(
                        [1.5, 64.5, 1.5],
                        Some(LANDING),
                        Some(api_state(wheat)),
                    ));
                }
                if converted || choice == Choice::ReplaceDuringLift {
                    let following = if choice == Choice::ReplaceDuringLift {
                        stone
                    } else {
                        farmland
                    };
                    expected.extend([
                        Moment::Lift(LANDING, 64.0, Some(api_state(farmland))),
                        Moment::Lift(OTHER, 64.0, Some(api_state(following))),
                    ]);
                }
                if converted || choice == Choice::Replace {
                    expected.push(Moment::Write(api_state(floor)));
                }
                if converted {
                    expected.push(Moment::GameEvent(
                        [1.5, 63.5, 1.5],
                        Some(LANDING),
                        Some(api_state(floor)),
                    ));
                }
                if damage != 0.0 {
                    expected.push(Moment::Fall(damage, y_after, api_state(floor)));
                }
                assert_eq!(*trace.lock().unwrap(), expected);
            });
        });
        std::mem::take(&mut *trace.lock().unwrap())
    }

    #[test]
    #[ignore = "daily"]
    fn player_and_mob_trampling_without_a_listener_follow_vanilla_order() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::native_hooks::farmland_trample::tests::player_and_mob_trampling_without_a_listener_follow_vanilla_order",
            || {
                for body in [WALKING, Body::Pig] {
                    land(body, Choice::Absent);
                }
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn the_lift_reads_a_bystanders_bounds_after_an_earlier_lift_callback() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::native_hooks::farmland_trample::tests::the_lift_reads_a_bystanders_bounds_after_an_earlier_lift_callback",
            || {
                land(Body::Pig, Choice::MoveBystander);
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn an_allowing_listener_keeps_the_player_and_mob_landing_order() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::native_hooks::farmland_trample::tests::an_allowing_listener_keeps_the_player_and_mob_landing_order",
            || {
                for body in [WALKING, Body::Pig] {
                    let baseline = land(body, Choice::Absent);
                    let allowed = land(body, Choice::Allow);
                    assert_eq!(allowed[1..], baseline);
                }
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn a_trapping_listener_rolls_back_without_preventing_fall_damage() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::native_hooks::farmland_trample::tests::a_trapping_listener_rolls_back_without_preventing_fall_damage",
            || {
                for body in [WALKING, Body::Pig] {
                    land(body, Choice::Trap);
                }
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn cancelling_keeps_the_crop_without_preventing_fall_damage() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::native_hooks::farmland_trample::tests::cancelling_keeps_the_crop_without_preventing_fall_damage",
            || {
                for body in [WALKING, Body::Pig] {
                    land(body, Choice::Cancel);
                }
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn cancelling_listener_keeps_writes_and_destroys_crop() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::native_hooks::farmland_trample::tests::cancelling_listener_keeps_writes_and_destroys_crop",
            || {
                for body in [WALKING, Body::Pig] {
                    land(body, Choice::CancelAndBreakCrop);
                }
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn a_sneaking_player_keeps_the_farmland_and_a_walking_one_tramples_it() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::native_hooks::farmland_trample::tests::a_sneaking_player_keeps_the_farmland_and_a_walking_one_tramples_it",
            || {
                land(Body::Guest { sneaking: true }, Choice::AvoidSneak);
                land(WALKING, Choice::AvoidSneak);
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn a_guest_landing_uses_its_own_move_in_a_batch() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::native_hooks::farmland_trample::tests::a_guest_landing_uses_its_own_move_in_a_batch",
            || {
                land_with_later_packet(
                    Body::Guest { sneaking: true },
                    Choice::AvoidSneak,
                    Some(S::PlayerInput(play::PlayerInput { flags: 0 })),
                );
                land_with_later_packet(
                    WALKING,
                    Choice::Allow,
                    Some(S::MovePlayerPos(play::MovePlayerPos {
                        x: 0.5,
                        y: LANDING_Y,
                        z: 1.5,
                        flags: 0,
                    })),
                );
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn a_crouching_mob_keeps_the_farmland_and_a_standing_one_tramples_it() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::native_hooks::farmland_trample::tests::a_crouching_mob_keeps_the_farmland_and_a_standing_one_tramples_it",
            || {
                for crouching in [true, false] {
                    land(Body::Cat { crouching }, Choice::AvoidSneak);
                }
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn a_listener_that_changes_the_block_refuses_the_trample() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::native_hooks::farmland_trample::tests::a_listener_that_changes_the_block_refuses_the_trample",
            || {
                for body in [WALKING, Body::Pig] {
                    land(body, Choice::Replace);
                    land(body, Choice::ReplaceDuringLift);
                }
            },
        );
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn a_panicking_creative_listener_during_trample_does_not_abort_the_trample() {
        use mod_api::prelude::ItemsHost;
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

        struct PanickingCreative(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl Hooks for PanickingCreative {
            fn build_creative_mode_tab_contents(
                &self,
                _host: HostSide<'_>,
                _event: &mut BuildCreativeModeTabContentsEvent<'_>,
            ) {
                self.0.fetch_add(1, Ordering::Relaxed);
                panic!("creative listener boom");
            }
        }

        struct HealthyCreative(Arc<Mutex<Vec<String>>>);
        #[mod_api::__subscriptions]
        impl Hooks for HealthyCreative {
            fn build_creative_mode_tab_contents(
                &self,
                _host: HostSide<'_>,
                event: &mut BuildCreativeModeTabContentsEvent<'_>,
            ) {
                self.0.lock().unwrap().push(event.tab.to_owned());
                if event.tab == "test:first" {
                    let torch = mod_api::ItemStack::new(
                        mod_api::__private::item_key("minecraft:torch").unwrap(),
                        1,
                    )
                    .unwrap();
                    event
                        .output
                        .insert(
                            CreativeTabPlacement::Last,
                            torch,
                            CreativeTabVisibility::Both,
                        )
                        .unwrap();
                }
            }
        }

        struct LazyCreativeGardener(Arc<AtomicBool>);
        #[mod_api::__subscriptions]
        impl Hooks for LazyCreativeGardener {
            fn farmland_trample(
                &self,
                host: &mut dyn ServerHost,
                event: &mut FarmlandTrampleEvent,
            ) -> Flow {
                let level = event.entity.entity().level(host).unwrap();
                let key = host.block_key("minecraft:stone").unwrap();
                let stone = mod_api::BlockState::of(&host.block_states(), key, &[]).unwrap();
                assert_eq!(
                    level.set_block(host, LISTENER_WRITE, stone, BlockUpdateFlags::KNOWN_SHAPE),
                    Ok(true)
                );
                let tabs = ItemsHost::creative_tabs(host, CreativeTabPermissions::Normal).unwrap();
                let torch = mod_api::ItemStack::new(
                    mod_api::__private::item_key("minecraft:torch").unwrap(),
                    1,
                )
                .unwrap();
                assert_eq!(tabs.tab("test:first").unwrap().display_items, [torch]);
                assert!(tabs.tab("test:second").is_some());
                self.0.store(true, Ordering::Relaxed);
                Flow::Continue
            }
        }

        let catalog = serde_json::json!({
            "schema": 4,
            "item_features": {},
            "potion_features": {},
            "tabs": [
                {"id": "test:first", "title": "test:first", "type": "CATEGORY", "build": []},
                {"id": "test:second", "title": "test:second", "type": "CATEGORY", "build": []},
            ]
        });
        let broken_calls = Arc::new(AtomicUsize::new(0));
        let healthy_tabs = Arc::new(Mutex::new(Vec::new()));
        let built = Arc::new(AtomicBool::new(false));
        let mods = vec![
            ModEntry::new(
                "broken-creative",
                Box::new(PanickingCreative(broken_calls.clone())),
            ),
            ModEntry::new(
                "healthy-creative",
                Box::new(HealthyCreative(healthy_tabs.clone())),
            ),
            ModEntry::new("gardener", Box::new(LazyCreativeGardener(built.clone()))),
        ];
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::native_hooks::farmland_trample::tests::a_panicking_creative_listener_during_trample_does_not_abort_the_trample",
            || {
                mod_platform::testing::with_mods(mods.into(), || {
                    with_game(|game| {
                        game.options.dedicated = Some(Vec::new());
                        let check_catalog = sunrise_client::creative::Creative::from_json(&catalog)
                            .unwrap()
                            .catalog()
                            .clone();
                        game.test_set_creative_catalog(check_catalog);
                        assert_eq!(ItemsHost::built_creative_tabs(game), Ok(None));

                        let (common, server) =
                            sunrise_gameplay_common::game::SideGame::common_and_state_mut(game);
                        server
                            .streaming
                            .insert_remote(&mut common.world, Chunk::empty(0, 0));
                        let reg = game.pack.assets.registry;
                        let [farmland, dirt, stone, wheat] = [
                            "minecraft:farmland",
                            "minecraft:dirt",
                            "minecraft:stone",
                            "minecraft:wheat",
                        ]
                        .map(|name| reg.default_state(name).unwrap());
                        let [x, y, z] = FIELD;
                        game.world.set(x, y, z, farmland).unwrap();
                        game.world.set(x, y + 1, z, wheat).unwrap();
                        let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                        let mut mob = sunrise_sim::mobs::Mob::new(
                            LANDING,
                            kind,
                            Vec3::new(1.5, 64.125, 1.5),
                            0.0,
                        );
                        mob.ai.goals = Default::default();
                        mob.ai.targets = Default::default();
                        mob.fall_distance = 6.0;
                        mob.velocity.y = -0.5;
                        let initial_health = mob.health;
                        game.mobs.push(mob);
                        game.entity_section_joined(LANDING, [1.5, 64.125, 1.5]);

                        game.tick_creature_entities().unwrap();

                        assert!(built.load(Ordering::Relaxed));
                        assert_eq!(broken_calls.load(Ordering::Relaxed), 1);
                        assert_eq!(*healthy_tabs.lock().unwrap(), ["test:first", "test:second"]);
                        assert_eq!(
                            mod_api::__private::poisoned_listeners(
                                mod_api::HookId::BuildCreativeModeTabContents
                            ),
                            1
                        );
                        assert_eq!(
                            mod_api::__private::poisoned_listeners(
                                mod_api::HookId::FarmlandTrample
                            ),
                            0
                        );
                        assert!(ItemsHost::built_creative_tabs(game).unwrap().is_some());
                        assert_eq!(game.world.block(x, y, z), dirt);
                        let [wx, wy, wz] = LISTENER_WRITE;
                        assert_eq!(game.world.block(wx, wy, wz), stone);
                        let landed = game.mob_object(LANDING).unwrap();
                        assert_eq!(landed.position.y, 64.0);
                        assert_eq!(landed.health, initial_health - 3.0);
                    })
                })
            },
        );
    }
}
