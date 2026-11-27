#![cfg(test)]
use sunrise_core::geometry::Vec3;
use sunrise_core::position::BlockPos;
use sunrise_gameplay_common::game::Dim;
use sunrise_gameplay_common::game::entity_memory::OffLevel;
use sunrise_gameplay_common::game::minecarts;
use sunrise_sim::mobs::Mob;
#[cfg(test)]
pub(crate) mod tests {

    use super::*;
    use sunrise_gameplay_common::game::GameGameExt1;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;

    use mod_api::ServerHost;
    use sunrise_gameplay::game::native_entity_refs::tests::fixture_resident_column;

    use std::sync::Arc;
    use sunrise_sim::nether_portal::PLAYER_DELAY;

    fn in_own_process(name: &str, case: &str) -> bool {
        !sunrise_gameplay_common::game::test_fixtures::run_fresh_cases(
            &format!("cut_layout::game::server_side::server_travel::tests::{name}"),
            "SUNRISE_TEST_SERVER_PORTAL_CASE",
            &[case],
        )
    }

    struct CooldownWitness(Arc<std::sync::Mutex<Vec<Option<i32>>>>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for CooldownWitness {
        fn entity_travel_to_dimension(
            &self,
            host: &mut dyn ServerHost,
            e: &mut mod_api::EntityTravelToDimensionEvent,
        ) -> mod_api::Flow {
            let host: &dyn mod_api::Host = host;
            self.0.lock().unwrap().push(e.entity.portal_cooldown(host));
            mod_api::Flow::Cancel
        }
    }

    #[test]
    #[ignore = "daily"]
    fn a_joining_player_keeps_its_saved_portal_cooldown() {
        if sunrise_gameplay_common::game::test_fixtures::run_fresh_cases(
            "cut_layout::game::server_side::server_travel::tests::a_joining_player_keeps_its_saved_portal_cooldown",
            "SUNRISE_TEST_SERVER_PORTAL_CASE",
            &["join"],
        ) {
            return;
        }
        super::super::native_guest_turn_tests::with_guests(|game| {
            // A player whose file was saved 7 ticks into its cooldown, before its first body.
            let body =
                sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5))
                    .unwrap();
            game.server.lan.as_mut().unwrap().hub.update_guest(12, |g| {
                g.motion = None;
                g.loaded_body = Some(sunrise_sim::lan::guest::SavedBody::of(&body));
                g.saved = Some(sunrise_core::nbt::Tag::compound([(
                    "PortalCooldown",
                    sunrise_core::nbt::Tag::Int(7),
                )]));
            });
            let tag = game.as_guest(12, |g| g.player_nbt()).unwrap().unwrap();
            assert_eq!(
                tag.get("PortalCooldown"),
                Some(&sunrise_core::nbt::Tag::Int(7))
            );
            // The countdown goes on in the player's record between its turns.
            assert_eq!(
                game.server.lan.as_ref().unwrap().play[&12].portal.cooldown,
                7
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn guest_portal_entry_uses_body_sleep_and_vehicle() {
        if !in_own_process("guest_portal_entry_uses_body_sleep_and_vehicle", "entry") {
            return;
        }
        super::super::native_guest_turn_tests::with_guests(|game| {
            let mut chunk = sunrise_level::chunk::Chunk::empty(0, 0);
            chunk.lit = true;
            game.world.chunks.insert((0, 0), Arc::new(chunk));
            let portal = game
                .pack
                .assets
                .registry
                .default_state("minecraft:nether_portal")
                .unwrap();
            game.world.set(3, 64, 5, portal).unwrap();
            // handlePortal decays the wait first; only a player that may use
            // the portal is then marked inside it, for the next tick.
            for (body, dead, sleeping, riding, expected) in [
                (true, false, false, false, true),
                (true, false, true, false, false),
                (true, false, false, true, false),
                (true, true, false, false, false),
                (false, false, false, false, false),
            ] {
                game.server.lan.as_mut().unwrap().hub.update_guest(12, |g| {
                    g.position = [3.5, 64.0, 5.5];
                    g.sleeping = sleeping;
                    g.vehicle = riding.then_some(99);
                    g.motion = body.then(|| {
                        Box::new(
                            sunrise_sim::motion::PlayerMotion::new(
                                game.common.motion,
                                Vec3::new(3.5, 64.0, 5.5),
                            )
                            .unwrap(),
                        )
                    });
                    if dead {
                        g.motion.as_mut().unwrap().set_health(0.0, false).unwrap();
                    }
                });
                game.guest_portal(12).unwrap().process =
                    Some(sunrise_sim::nether_portal::Process {
                        entry: BlockPos::new(3, 64, 5),
                        time: 8,
                        inside: false,
                    });
                game.handle_server_portal(12);
                let process = game.guest_portal(12).unwrap().process.unwrap();
                assert_eq!(
                    (process.time, process.inside),
                    (4, expected),
                    "body={body} dead={dead} sleeping={sleeping} riding={riding}"
                );
            }
            // Marked inside last tick with the wait at its threshold, then mounted: handlePortal
            // asks canUsePortal again, so the player stays and the wait does not grow.
            game.server.lan.as_mut().unwrap().hub.update_guest(12, |g| {
                g.sleeping = false;
                g.vehicle = Some(99);
                g.motion = Some(Box::new(
                    sunrise_sim::motion::PlayerMotion::new(
                        game.common.motion,
                        Vec3::new(3.5, 64.0, 5.5),
                    )
                    .unwrap(),
                ));
            });
            let threshold = game
                .ui
                .rules
                .int_at(sunrise_game_data::gamerules::vanilla::PLAYERS_NETHER_PORTAL_DEFAULT_DELAY);
            game.guest_portal(12).unwrap().process = Some(sunrise_sim::nether_portal::Process {
                entry: BlockPos::new(3, 64, 5),
                time: threshold,
                inside: true,
            });
            game.handle_server_portal(12);
            assert!(!game.has_pending_travel(12));
            assert_eq!(game.guest_portal_cooldown(12), 0);
            let process = game.guest_portal(12).unwrap().process.unwrap();
            assert_eq!((process.time, process.inside), (threshold, false));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_travel_listener_reads_the_cooldown_the_portal_just_set() {
        if !in_own_process(
            "a_travel_listener_reads_the_cooldown_the_portal_just_set",
            "listener",
        ) {
            return;
        }
        let heard = Arc::default();
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "cooldown-witness",
            Box::new(CooldownWitness(Arc::clone(&heard))),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_guest_turn_tests::with_guests(|game| {
                game.common.options.dedicated = Some(Vec::new());
                let mut chunk = sunrise_level::chunk::Chunk::empty(0, 0);
                chunk.lit = true;
                game.world.chunks.insert((0, 0), Arc::new(chunk));
                let portal = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:nether_portal")
                    .unwrap();
                game.world.set(3, 64, 5, portal).unwrap();
                game.server.lan.as_mut().unwrap().hub.update_guest(12, |g| {
                    g.position = [3.5, 64.0, 5.5];
                    g.motion = Some(Box::new(
                        sunrise_sim::motion::PlayerMotion::new(
                            game.common.motion,
                            Vec3::new(3.5, 64.0, 5.5),
                        )
                        .unwrap(),
                    ));
                });
                // A survival player whose wait reaches the portal's transition time this tick.
                let threshold = game.ui.rules.int_at(
                    sunrise_game_data::gamerules::vanilla::PLAYERS_NETHER_PORTAL_DEFAULT_DELAY,
                );
                game.guest_portal(12).unwrap().process =
                    Some(sunrise_sim::nether_portal::Process {
                        entry: BlockPos::new(3, 64, 5),
                        time: threshold,
                        inside: true,
                    });
                game.handle_server_portal(12);
                // Entity.handlePortal: setPortalCooldown comes before the travel is asked.
                assert_eq!(*heard.lock().unwrap(), [Some(PLAYER_DELAY)]);
                // The listener canceled the trip; a fresh wait must come first.
                assert!(!game.has_pending_travel(12));
                assert!(game.guest_portal(12).unwrap().process.is_none());
            });
        });
    }
    #[test]
    #[ignore = "daily"]
    fn arriving_vehicle_is_announced_once_after_destination_entities() {
        use mod_api::{self, EntityJoinLevelEvent, Flow, Hooks};
        use std::sync::Mutex;
        struct Observer(Arc<Mutex<Vec<u32>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Observer {
            fn entity_join_level(
                &self,
                _: &mut dyn ServerHost,
                event: &mut EntityJoinLevelEvent<'_>,
            ) -> Flow {
                self.0.lock().unwrap().push(event.subject.id());
                Flow::Continue
            }
        }
        let joins = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "arrival-observer",
            Box::new(Observer(joins.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.native_joins_skip();
                fixture_resident_column(&mut game.world, (0, 0));
                let base = game.server.next_entity;
                game.server.next_entity += 2;
                game.carts.push(minecarts::Minecart::new(
                    base + 1,
                    minecarts::CartKind::Rideable,
                    Vec3::ZERO,
                ));
                let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                let mut child = Mob::new(base + 10, kind, Vec3::ZERO, 0.0);
                child.extra.mount = Some(base + 1);
                game.persist
                    .off_level
                    .insert(base + 10, OffLevel::Mob(child));
                // It left with 7; the source level has run it down to 3 since.
                game.server
                    .portals
                    .entities
                    .entry(base + 1)
                    .or_default()
                    .cooldown = 3;
                game.server.portals.carried_cooldown.insert(base + 1, 7);
                let source = game.dim;
                game.enter_level(Dim::Nether).unwrap();
                game.carts.push(minecarts::Minecart::new(
                    base + 2,
                    minecarts::CartKind::Rideable,
                    Vec3::ZERO,
                ));
                game.native_joins_flush(false, 0);
                game.arrive_vehicle(source, base + 1, Vec3::ZERO, 0.0);
                assert!(
                    game.mob_object(base + 10).is_some(),
                    "natural carrier arrival must transfer retained passenger"
                );
                assert_eq!(game.server.portals.entities[&(base + 1)].cooldown, 7);
                assert!(
                    !game
                        .server
                        .levels
                        .0
                        .parked_mut(source)
                        .unwrap()
                        .portal_states
                        .contains_key(&(base + 1))
                );
                game.native_joins_flush(false, 0);
                game.native_joins_flush(false, 0);
                assert_eq!(*joins.lock().unwrap(), vec![base + 2, base + 10, base + 1]);
            });
        });
    }
    #[test]
    #[ignore = "daily"]
    fn natural_carrier_moves_full_objects_and_readmits_the_tree_in_order() {
        use mod_api::{
            AttachmentDescriptor, AttachmentKey, EntityConstructingEvent, Holder, Hooks, HostSide,
        };
        use std::sync::Mutex;
        struct Constructed(
            Arc<Mutex<Vec<Holder>>>,
            Arc<Mutex<Option<AttachmentKey<i32>>>>,
        );
        #[mod_api::__subscriptions]
        impl Hooks for Constructed {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                let state = r
                    .register_attachment_type(AttachmentDescriptor::new(
                        "natural_tree:state",
                        || 0i32,
                    ))
                    .unwrap();
                *self.1.lock().unwrap() = Some(state);
            }
            fn entity_constructing(&self, host: HostSide<'_>, event: &mut EntityConstructingEvent) {
                self.0.lock().unwrap().push(event.holder);
                if let HostSide::Server(host) = host {
                    let state = self
                        .1
                        .lock()
                        .unwrap()
                        .clone()
                        .expect("Constructed registered");
                    state.set(host, event.holder, 41).unwrap();
                }
            }
        }
        for conflict in [false, true] {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let state_slot = Arc::new(Mutex::new(None));
            let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                "natural_tree",
                Box::new(Constructed(calls.clone(), state_slot.clone())),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                super::super::native_entity_refs::tests::with_game(|game| {
                    let state = state_slot
                        .lock()
                        .unwrap()
                        .clone()
                        .expect("Constructed registered");
                    game.options.dedicated = Some(Vec::new());
                    let tag = sunrise_core::nbt::parse_snbt(r#"{
                    id:"minecraft:pig",UUID:[I;703,0,0,1],Pos:[1.0d,80.0d,2.0d],NoAI:1b,
                    Passengers:[
                        {id:"minecraft:cow",UUID:[I;703,0,0,1],Pos:[1.0d,81.0d,2.0d],NoAI:1b,
                         Passengers:[{id:"minecraft:chicken",UUID:[I;703,0,0,2],Pos:[1.0d,82.0d,2.0d],NoAI:1b}]},
                        {id:"minecraft:sheep",UUID:[I;703,0,0,2],Pos:[1.0d,81.0d,2.0d],NoAI:1b}
                    ]
                }"#).unwrap().0;
                    let root = game.load_mob(tag, None).unwrap();
                    let children = game.mob_passenger_ids(root);
                    let (held, sibling) = (children[0], children[1]);
                    let leaf = game.mob_passenger_ids(held)[0];
                    let ids = [root, held, leaf, sibling];
                    assert!(game.persist.off_level.contains_key(&held));
                    assert!(game.persist.off_level.contains_key(&sibling));
                    state.set(game, Holder::Entity(leaf), 77).unwrap();
                    game.persist.forced_meta.insert(leaf, [5, 6, 7, 8]);
                    game.server
                        .portals
                        .entities
                        .entry(leaf)
                        .or_default()
                        .cooldown = 19;
                    let source = game.dim;
                    game.enter_level(Dim::Nether).unwrap();
                    fixture_resident_column(&mut game.world, (0, 0));
                    fixture_resident_column(&mut game.world, (2, 3));
                    let owner = conflict.then(|| {
                    let tag = sunrise_core::nbt::parse_snbt(r#"{id:"minecraft:zombie",UUID:[I;703,0,0,1],Pos:[0.0d,70.0d,0.0d],NoAI:1b}"#).unwrap().0;
                    game.load_mob(tag, None).unwrap()
                });
                    let before = calls.lock().unwrap().clone();
                    // A player's direct mount may be nested below a retained object;
                    // arrival must still carry the actual source root and every sibling.
                    let destination = Vec3::new(32.0, 90.0, 48.0);
                    game.arrive_vehicle(source, leaf, destination, 90.0);
                    assert!(ids.iter().all(|id| game.mob_object(*id).is_some()));
                    assert!(game.persist.off_level.contains_key(&root));
                    assert!(game.persist.off_level.contains_key(&sibling));
                    assert_eq!(game.persist.off_level.contains_key(&held), conflict);
                    assert!(game.entity(root).is_none());
                    assert!(game.entity(sibling).is_none());
                    assert!(game.entity(leaf).is_some());
                    assert_eq!(game.entity(held).is_some(), !conflict);
                    if let Some(owner) = owner {
                        assert!(game.entity(owner).is_some());
                    }
                    assert_eq!(game.mob_object(held).unwrap().extra.mount, Some(root));
                    assert_eq!(game.mob_object(leaf).unwrap().extra.mount, Some(held));
                    assert_eq!(game.mob_object(sibling).unwrap().extra.mount, Some(root));
                    assert_eq!(game.mob_object(root).unwrap().position, destination);
                    assert_eq!(
                        game.mob_object(held).unwrap().position,
                        Vec3::new(32.0, 91.0, 48.0)
                    );
                    assert_eq!(
                        game.mob_object(leaf).unwrap().position,
                        Vec3::new(32.0, 92.0, 48.0)
                    );
                    assert_eq!(
                        game.mob_object(sibling).unwrap().position,
                        Vec3::new(32.0, 91.0, 48.0)
                    );
                    assert_eq!(game.persist.forced_meta.get(&leaf), Some(&[5, 6, 7, 8]));
                    assert_eq!(game.server.portals.entities[&leaf].cooldown, 19);
                    assert_eq!(state.get(game, Holder::Entity(leaf), Clone::clone), Ok(77));
                    if !conflict {
                        assert_eq!(state.get(game, Holder::Entity(held), Clone::clone), Ok(41));
                    }
                    assert!(game.mobs_below_off_level().contains(&leaf));
                    game.native_joins_flush(false, 0);
                    assert_eq!(
                        *calls.lock().unwrap(),
                        before,
                        "arrival must not construct again"
                    );
                    // Another player from the same stack arrives without duplicating it.
                    game.arrive_vehicle(source, leaf, destination, 90.0);
                    assert_eq!(
                        game.mob_objects().filter(|m| ids.contains(&m.id)).count(),
                        4
                    );
                    game.enter_level(source).unwrap();
                    for id in ids {
                        assert!(game.mob_object(id).is_none());
                        assert!(!game.persist.uuids.contains_key(&id));
                        assert!(!game.persist.raw.contains_key(&id));
                        assert!(!game.persist.forced_meta.contains_key(&id));
                    }
                    game.enter_level(Dim::Nether).unwrap();
                    assert_eq!(state.get(game, Holder::Entity(leaf), Clone::clone), Ok(77));
                    game.native_joins_flush(false, 0);
                    assert_eq!(*calls.lock().unwrap(), before);
                })
            });
        }
    }
    /// `Entity.teleportCrossDimension` admits the passengers before the copied
    /// vehicle, so a pig that shares its boat's or minecart's UUID takes it in
    /// the destination and the vehicle is retained off-level below it.
    #[test]
    #[ignore = "daily"]
    fn natural_carrier_retains_a_vehicle_root_refused_by_its_passenger() {
        for (kind, uuid) in [("minecraft:oak_boat", 705), ("minecraft:minecart", 706)] {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                let tag = sunrise_core::nbt::parse_snbt(&format!(
                    r#"{{id:"{kind}",UUID:[I;{uuid},0,0,1],Pos:[1.0d,80.0d,2.0d],
                    Passengers:[{{id:"minecraft:pig",UUID:[I;{uuid},0,0,1],Pos:[1.0d,80.0d,2.0d],NoAI:1b,
                     Passengers:[{{id:"minecraft:chicken",UUID:[I;{uuid},0,0,2],Pos:[1.0d,80.0d,2.0d],NoAI:1b}}]}}]}}"#
                ))
                .unwrap()
                .0;
                let root = if kind.ends_with("boat") {
                    game.load_boat_transferred(tag, Some([uuid, 0, 0, 1]), kind, None)
                } else {
                    game.load_cart_transferred(tag, Some([uuid, 0, 0, 1]), kind, None)
                }
                .unwrap();
                let pig = game.mob_passenger_ids(root)[0];
                let chicken = game.mob_passenger_ids(pig)[0];
                assert!(game.persist.off_level.contains_key(&pig));
                let source = game.dim;
                game.enter_level(Dim::Nether).unwrap();
                fixture_resident_column(&mut game.world, (2, 3));
                game.arrive_vehicle(source, chicken, Vec3::new(32.0, 90.0, 48.0), 0.0);
                assert!(game.entity(pig).is_some());
                assert!(game.entity(chicken).is_some());
                assert!(game.entity(root).is_none());
                assert!(game.boats.iter().all(|b| b.id != root));
                assert!(game.carts.iter().all(|c| c.id != root));
                assert!(game.boat_object(root).is_some() || game.cart_object(root).is_some());
                assert_eq!(game.mob_object(pig).unwrap().extra.mount, Some(root));
                assert_eq!(game.mob_object(chicken).unwrap().extra.mount, Some(pig));
                assert!(game.mobs_below_off_level().contains(&pig));
                assert!(game.mobs_below_off_level().contains(&chicken));
                game.enter_level(source).unwrap();
                for id in [root, pig, chicken] {
                    assert!(!game.persist.off_level.contains_key(&id));
                    assert!(game.mob_object(id).is_none());
                }
                assert!(game.boats.iter().all(|b| b.id != root));
                assert!(game.carts.iter().all(|c| c.id != root));
            });
        }
    }
    #[test]
    #[ignore = "daily"]
    fn natural_portal_schedules_a_player_on_a_retained_indirect_vehicle() {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            sunrise_gameplay::game::native_player_death::tests::add_guest(game);
            let source = game.dim;
            let tag = sunrise_core::nbt::parse_snbt(
                r#"{
                id:"minecraft:pig",UUID:[I;704,0,0,1],Pos:[1.0d,80.0d,2.0d],NoAI:1b,
                Passengers:[{id:"minecraft:cow",UUID:[I;704,0,0,1],Pos:[1.0d,81.0d,2.0d],NoAI:1b}]
            }"#,
            )
            .unwrap()
            .0;
            let root = game.load_mob(tag, None).unwrap();
            let held = game.mob_passenger_ids(root)[0];
            assert!(game.persist.off_level.contains_key(&held));
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |guest| {
                    guest.vehicle = Some(held);
                });
            super::super::native_entity_refs::tests::prepare_entity_ticking_neighborhood(
                game,
                sunrise_core::position::ChunkPos::new(0, 0),
            );
            game.server.tickets.0.add_portal((0, 0));
            game.update_chunk_tickets();
            assert!(game.entity_ticking_area()(1, 2));
            // Supply the block's real inside notification, then exercise the actual
            // entity portal processor, carrier routing and server trip scheduler.
            let delay = sunrise_sim::nether_portal::ENTITY_DELAY;
            game.server
                .portals
                .entities
                .entry(root)
                .or_default()
                .set_inside(BlockPos::new(1, 80, 2), delay);
            game.handle_entity_portals();
            assert!(
                game.mob_object(root).is_some(),
                "route through the player carrier path"
            );
            game.carry_riders_through_portals();
            assert_eq!(game.server.server_trips.len(), 1);
            let trip = &game.server.server_trips[0];
            assert_eq!(trip.source, source);
            assert_eq!(trip.dim, Dim::Nether);
            assert_eq!(
                trip.vehicle,
                Some(held),
                "keep the direct mount in the ticket"
            );
            assert_eq!(game.dim, source);
            game.enter_level(Dim::Nether).unwrap();
            game.arrive_vehicle(source, held, Vec3::new(32.0, 90.0, 48.0), 0.0);
            assert!(game.mob_object(root).is_some());
            assert!(game.mob_object(held).is_some());
            assert_eq!(game.mob_object(held).unwrap().extra.mount, Some(root));
            assert_eq!(
                game.server.lan.as_ref().unwrap().hub.guests[0].vehicle,
                Some(held)
            );
        });
    }
}

#[cfg(test)]
pub(crate) mod transfer_lifetime_tests {

    use sunrise_client_lib::join_seq::Joined;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::server_side::server_travel::transfer_lifetime_tests::VALUE;

    use super::*;

    use sunrise_mod_runtime::attachments::{
        AttachmentLifetime, Attachments, HasAttachmentLifetime, Holder,
    };

    #[test]
    #[ignore = "daily"]
    fn transfer_arrival_rebinds_only_bound_owners_with_the_same_id() {
        for kind in ["minecraft:minecart", "minecraft:oak_boat", "minecraft:pig"] {
            for bound in [true, false] {
                super::super::native_entity_refs::tests::with_game(|game| {
                    sunrise_gameplay::game::native_entity_refs::tests::fixture_resident_column(
                        &mut game.world,
                        (0, 0),
                    );
                    let target = game
                        .summon_with_nbt(kind, [0.5, 70.0, 0.5], "{}")
                        .unwrap()
                        .unwrap();
                    let id = match target {
                        sunrise_gameplay_common::game::commands_common::Target::Cart(id)
                        | sunrise_gameplay_common::game::commands_common::Target::Boat(id)
                        | sunrise_gameplay_common::game::commands_common::Target::Mob(id) => id,
                        _ => panic!("unexpected vehicle target"),
                    };
                    let old_join = match kind {
                        "minecraft:minecart" => game.carts[0].join_seq(),
                        "minecraft:oak_boat" => game.boats[0].join_seq(),
                        _ => game.mobs[0].join_seq(),
                    };
                    assert!(old_join.is_joined());
                    let uuid = game.uuid_for(id);
                    let source = game.dim;
                    let source_ref = game.entity(id).unwrap();
                    let source_index = game.attachment_indexes.entry(source).or_default().clone();
                    let mut map = Attachments::default();
                    VALUE.set(&mut map, Holder::Entity(id), 17).unwrap();
                    let old = if bound {
                        Some(map.bind_index(
                            &source_index,
                            mod_api::AttachmentHolder::Entity(source_ref),
                        ))
                    } else {
                        None
                    };
                    macro_rules! slot {
                        ($list:ident) => {
                            game.$list.update(0, |entity| {
                                if let Some(old) = &old {
                                    *entity.attachment_lifetime() =
                                        AttachmentLifetime::new(old.clone());
                                }
                            })
                        };
                    }
                    match kind {
                        "minecraft:minecart" => slot!(carts),
                        "minecraft:oak_boat" => slot!(boats),
                        _ => slot!(mobs),
                    }
                    let key = ("test".into(), "transfer".into());
                    let retained = source_index.lock().unwrap().snapshot(&key);
                    let mut stale_map = map.clone();
                    game.entity_attachments.insert(id, map);
                    game.enter_level(Dim::Nether).unwrap();
                    sunrise_gameplay::game::native_entity_refs::tests::fixture_resident_column(
                        &mut game.world,
                        (0, 0),
                    );
                    let destination_ref =
                        mod_api::EntityRef::new(game.level(game.dim.id()).unwrap().epoch(), id);
                    assert_eq!(source_ref, destination_ref);
                    game.arrive_vehicle(source, id, Vec3::ZERO, 0.0);
                    macro_rules! transferred {
                        ($list:ident) => {{
                            let parked = game.server.levels.0.parked_mut(source).unwrap();
                            assert!(
                                !parked
                                    .$list
                                    .iter()
                                    .any(|e| e.id == id || e.join_seq() == old_join)
                            );
                            let arrived = game.$list.iter().find(|e| e.id == id).unwrap();
                            assert!(arrived.join_seq().is_joined());
                            assert_ne!(arrived.join_seq(), old_join);
                            assert!(!game.$list.iter().any(|e| e.join_seq() == old_join));
                        }};
                    }
                    match kind {
                        "minecraft:minecart" => transferred!(carts),
                        "minecraft:oak_boat" => transferred!(boats),
                        _ => transferred!(mobs),
                    }
                    assert!(source_index.lock().unwrap().snapshot(&key).is_empty());
                    let dim = game.dim;
                    let destination_index = game.attachment_indexes.entry(dim).or_default().clone();
                    let rows = destination_index.lock().unwrap().snapshot(&key);
                    assert_eq!(rows.len(), usize::from(bound));
                    if let Some(old) = old {
                        let current = match kind {
                            "minecraft:minecart" => game
                                .carts
                                .update(0, |e| e.attachment_lifetime().registration().unwrap()),
                            "minecraft:oak_boat" => game
                                .boats
                                .update(0, |e| e.attachment_lifetime().registration().unwrap()),
                            _ => game.mobs[0].attachment_lifetime.registration().unwrap(),
                        };
                        assert!(!old.same_registration(&current));
                        assert_eq!(
                            rows[0].holder,
                            mod_api::AttachmentHolder::Entity(destination_ref)
                        );
                        assert_eq!(rows[0].entry.read(&VALUE), Ok(&17));
                        assert!(!old.unregister());
                        drop(AttachmentLifetime::new(old));
                        VALUE.set(&mut stale_map, Holder::Entity(id), 99).unwrap();
                        assert_eq!(
                            destination_index.lock().unwrap().snapshot(&key)[0]
                                .entry
                                .read(&VALUE),
                            Ok(&17)
                        );
                        assert_eq!(retained[0].entry.read(&VALUE), Ok(&17));
                        for index in [&source_index, &destination_index] {
                            assert_eq!(index.lock().unwrap().validate_row(&retained[0]),
                                Err(sunrise_mod_runtime::attachments::AttachmentRowError::StaleHolder));
                        }
                        assert_eq!(
                            destination_index.lock().unwrap().validate_row(&rows[0]),
                            Ok(())
                        );
                    }
                    assert_eq!(game.persist.uuids.get(&id), Some(&uuid));
                });
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod credits_respawn_tests {

    use sunrise_gameplay_common::game::GameGameExt1;
    use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2;
    use sunrise_gameplay_common::game::server_side::server_travel::credits_respawn_tests::ANCHOR;

    use sunrise_gameplay_common::game::server_side::server_travel::credits_respawn_tests::placed_guest;

    use sunrise_gameplay_common::game::server_side::server_travel::credits_respawn_tests::received_packets;

    use sunrise_gameplay::game::server_side::server_travel::credits_respawn_tests::charge_anchor;

    use sunrise_gameplay::game::server_side::server_travel::credits_respawn_tests::light_spawn_chunks;

    use sunrise_gameplay_common::game::Dim;

    use std::sync::{Arc, mpsc};

    use sunrise_level::chunk::Chunk;

    use sunrise_protocol::packets::play::{self, Clientbound as C, Serverbound as S};

    use sunrise_sim::lan::{
        NetConfig, Out,
        guest::{InboxItem, SPECTATOR},
        hub::HubConfig,
    };

    /// `ServerGamePacketListenerImpl.handleClientCommand(PERFORM_RESPAWN)` for a
    /// player who won the game: `restartClientLoadTimerAfterRespawn`, so the
    /// respawned player is not loaded (its actions wait, `isInvulnerableTo`
    /// spares it) until the client says so or 60 ticks pass.
    #[test]
    #[ignore = "daily"]
    fn the_credits_respawn_waits_for_the_client_to_load() {
        let mut resources = None;
        let (_scratch, mut game, rx) = server_with_a_nether_anchor(&mut resources);
        let host = game.server.lan.as_mut().unwrap();
        host.hub.guests.update(0, |g| {
            g.spawn = Some([0.5, 100.0, 0.5]);
            g.respawn_elsewhere = None;
            g.saved = Some(sunrise_core::nbt::Tag::compound([(
                "NeoForgeData",
                sunrise_core::nbt::Tag::compound([
                    ("outside", sunrise_core::nbt::Tag::Int(1)),
                    (
                        "PlayerPersisted",
                        sunrise_core::nbt::Tag::compound([(
                            "inside",
                            sunrise_core::nbt::Tag::Int(2),
                        )]),
                    ),
                ]),
            )]));
            g.health = 13.0;
            g.food = 9;
            g.saturation = 2.0;
        });
        host.hub.sync_guest_after_tick(2);
        assert!(host.hub.guests[0].sent_health.is_some());
        assert!(host.hub.guests[0].client_loaded());
        rx.try_iter().for_each(drop);

        game.remove_old_player_for_clone(2, false);
        let host = game.server.lan.as_mut().unwrap();
        host.hub.guests.update(0, |g| {
            g.won_game = true;
            g.inbox.push_back(InboxItem::Packet(
                S::ClientCommand(play::ClientCommand { action: 0 }),
                None,
            ));
        });
        game.tick_lan_packets();
        let host = game.server.lan.as_mut().unwrap();
        host.hub.sync_guest_after_tick(2);
        let delivered: Vec<_> = received_packets(&rx).collect();
        assert!(delivered.iter().any(|packet| matches!(packet,
            C::SetHealth(health) if (health.health, health.food, health.saturation) == (13.0, 9, 2.0)
        )));
        assert!(
            delivered
                .iter()
                .any(|packet| matches!(packet, C::SetEntityData(data) if data.id == 2))
        );
        let guest = &host.hub.guests[0];
        assert!(!guest.won_game, "the credits are over");
        assert_eq!(guest.load_timeout, 60);
        assert!(guest.loading);
        assert!(!guest.client_loaded());
        assert_eq!(
            guest.saved.as_ref().and_then(|tag| tag.get("NeoForgeData")),
            Some(&sunrise_core::nbt::Tag::compound([(
                "PlayerPersisted",
                sunrise_core::nbt::Tag::compound([("inside", sunrise_core::nbt::Tag::Int(2))]),
            )])),
            "the real first-End-exit path drops only data outside PlayerPersisted"
        );
    }
    /// A scratch server whose Overworld guest has a Nether respawn point.
    pub(super) fn server_with_a_nether_anchor<'a>(
        resources: &'a mut Option<Arc<sunrise_gameplay_common::game::Resources>>,
    ) -> (
        tempfile::TempDir,
        sunrise_gameplay::game::ServerGame<'a>,
        mpsc::Receiver<Out>,
    ) {
        let scratch = tempfile::tempdir().unwrap();
        let mut game = sunrise_gameplay::game::game_tests::tests::server_game(resources);
        game.server.world_root = scratch.path().to_owned();
        // The world spawn a player without a point of its own comes back to.
        game.level.world_spawn = Some([0, 100, 0]);
        game.lan_host(
            sunrise_gameplay::game::server_side::lan::LanSettings {
                port: 0,
                guest_commands: true,
                force_game_mode: false,
            },
            NetConfig {
                listen: false,
                announce: false,
                ..NetConfig::loopback(0, "credits-anchor-test".into(), 2)
            },
            HubConfig {
                default_game_mode: 0,
                guest_commands: true,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: "minecraft:overworld".into(),
                allow_flight: false,
                seed: 0,
                max_players: 2,
                integrated: false,
                levels: Default::default(),
                level_seeds: Default::default(),
            },
        )
        .unwrap();
        let (mut guest, rx) = placed_guest(2, Dim::Overworld, [0.5, 100.0, 0.5]);
        guest.respawn_elsewhere = Some((Dim::Nether.id().into(), ANCHOR, 0.0, 0.0, false));
        game.server.lan.as_mut().unwrap().hub.guests.push(guest);
        (scratch, game, rx)
    }
    /// A player whose own point is a charged Nether anchor and who has seen the
    /// credits comes back at that anchor, in the Nether, and the anchor keeps
    /// its charge: `PlayerList.respawn(keepAllPlayerData = true)` resolves the
    /// point in the config's own level and does not use the block up
    /// (`ServerPlayer.findRespawnPositionAndUseSpawnBlock(false, ..)`).
    #[test]
    #[ignore = "daily"]
    fn the_credits_respawn_returns_to_a_nether_anchor_and_keeps_its_charge() {
        let mut resources = None;
        let (_scratch, mut game, _rx) = server_with_a_nether_anchor(&mut resources);
        game.remove_old_player_for_clone(2, false);
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(2, |guest| guest.won_game = true);
        game.guest_respawn(2);
        assert_eq!(
            game.server.lan.as_ref().unwrap().hub.guests[0].dimension,
            "minecraft:overworld",
            "the credits respawn doesn't place a replacement before checking the anchor"
        );

        // The Nether loads around the anchor, which holds one charge.
        game.enter_level(Dim::Nether).unwrap();
        light_spawn_chunks(&mut game);
        let charged = charge_anchor(&mut game);
        game.tick_server_travel();

        // RespawnAnchorBlock.findStandUpPosition: the block's top is the only
        // spot with a floor under it here (`DismountHelper.findSafeDismountLocation`).
        let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
        assert_eq!(
            guest.position,
            [10.5, 101.0, 10.5],
            "the player stands on its anchor"
        );
        assert_eq!(guest.dimension, "minecraft:the_nether");
        // PlayerList.respawn copies the point when its block resolved.
        assert_eq!(
            guest.respawn_elsewhere.as_ref().map(|r| r.1),
            Some(ANCHOR),
            "the point stays the player's own"
        );
        assert_eq!(
            game.world.block(ANCHOR[0], ANCHOR[1], ANCHOR[2]),
            charged,
            "the anchor keeps its charge"
        );
        assert!(!game.on_server_trip(2), "the trip has arrived");
    }
    /// A spectator that right-clicks the exit portal in the End leaves for the
    /// level of its own point, not the Overworld: a spectator is a
    /// `ServerPlayer`, so `EndPortalBlock.getPortalDestination` resolves its
    /// point too (`findRespawnPositionAndUseSpawnBlock(false, ..)`).
    #[test]
    #[ignore = "daily"]
    fn a_spectator_leaves_the_end_for_its_nether_point() {
        let mut resources = None;
        let (_scratch, mut game, _rx) = server_with_a_nether_anchor(&mut resources);
        game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
            g.game_mode = SPECTATOR;
            g.seen_credits = true;
            g.dimension = "minecraft:the_end".into();
        });
        game.enter_level(Dim::End).unwrap();
        game.server
            .streaming
            .insert_remote(&mut game.common.world, Chunk::empty(0, 0));
        let portal = [0, 100, 0];
        assert!(
            game.set_state_named(portal, "minecraft:end_portal", &[])
                .unwrap()
        );
        assert!(game.spectator_use_portal(2, 0, portal));
        assert!(
            game.has_pending_travel(2),
            "still searching the saved point"
        );
        assert!(
            !game.on_server_trip(2),
            "the spectator stays active in the End"
        );
        assert_eq!(game.command_guest(2).unwrap().dimension, Dim::End.id());
        game.enter_level(Dim::Nether).unwrap();
        light_spawn_chunks(&mut game);
        charge_anchor(&mut game);
        game.tick_server_travel();
        assert_eq!(
            game.command_guest(2).unwrap().dimension,
            Dim::Nether.id(),
            "a spectator's exit from the End goes to its resolved point's level"
        );
        assert!(!game.has_pending_travel(2));
    }
}

#[cfg(test)]
#[path = "server_travel/hold_tests.rs"]
mod hold_tests;

#[cfg(test)]
#[path = "server_travel/changed_dimension_tests.rs"]
mod changed_dimension_tests;

#[cfg(test)]
#[path = "server_travel/respawn_tests.rs"]
mod respawn_tests;

#[cfg(test)]
#[path = "server_travel/respawn_target_tests.rs"]
mod respawn_target_tests;
