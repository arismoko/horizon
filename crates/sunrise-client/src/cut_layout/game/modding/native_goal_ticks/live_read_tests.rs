use mod_api::{ActiveEffect, AttributeOperation, EntityReadError, EntityRef, LivingRef, MobRef};
use std::ops::ControlFlow;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay::game::native_entity_refs::tests::{fixture_resident_column, with_game};
use sunrise_gameplay::game::native_goal_ticks::GoalCx;
use sunrise_sim::mobs;
use sunrise_sim::mobs::Mob;

struct OneMob<'a>(&'a Mob);
impl mobs::LiveMobs for OneMob<'_> {
    fn find(&self, id: u32) -> Option<&Mob> {
        (self.0.id == id).then_some(self.0)
    }

    fn for_each(&self, visitor: &mut dyn FnMut(&Mob)) {
        visitor(self.0);
    }
}

/// Run the public target getter with the actor and the other mob outside `Game`.
pub(crate) fn read_borrowed_target(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    warden: MobRef,
    other: u32,
    tick_conn: u32,
) -> Option<LivingRef> {
    let (_, view) = game
        .lan_guest_views()
        .into_iter()
        .find(|(conn, _)| *conn == tick_conn)
        .unwrap();
    let identity = game.guest_identity(tick_conn).unwrap();
    let mut loan = game.mobs.lift();
    let index = loan
        .iter()
        .position(|mob| mob.id == warden.entity().id())
        .unwrap();
    let result = loan
        .with_one_and_rest(index, |actor, before, after| {
            let live = OneMob(
                before
                    .iter()
                    .chain(after)
                    .find(|mob| mob.id == other)
                    .unwrap(),
            );
            let mut world = game.mob_world_base(&[]);
            world.goal_host = Some(game);
            world.live_mobs = Some(&live);
            world.player = Some(view);
            world.player_identity = Some(identity);
            warden.target(&GoalCx {
                m: actor,
                w: &world,
                host: game,
            })
        })
        .unwrap();
    game.mobs
        .restore(loan, sunrise_sim::join_seq::RestoreOrder::SurvivorsFirst);
    result
}

#[test]
#[ignore = "daily"]
fn scoped_host_reads_live_other_mob_before_stale_snapshot() {
    with_game(|game| {
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let mut actor = Mob::new(9001, kind.clone(), Vec3::ZERO, 0.0);
        let mut other = Mob::new(9002, kind.clone(), Vec3::ZERO, 0.0);
        let stale = Mob::new(9003, kind, Vec3::ZERO, 0.0);
        let snapshot = [
            sunrise_gameplay_common::game::creatures::seen(
                &other,
                game.level.player_uuid,
                game.items(),
            ),
            sunrise_gameplay_common::game::creatures::seen(
                &stale,
                game.level.player_uuid,
                game.items(),
            ),
        ];
        other.yaw = 37.0;
        other.head_pitch = -12.0;
        other.attrs.max_health = 73.0;
        other.add_effect("minecraft:night_vision", 80, 1);
        sunrise_sim::mobs::finalize::add_modifier(
            &mut other,
            sunrise_sim::mobs::finalize::Attr::MovementSpeed,
            "test:loaned_speed",
            0.25,
            sunrise_sim::mobs::finalize::Op::AddMultipliedTotal,
        );
        let live = OneMob(&other);
        let mut world = game.mob_world_base(&snapshot);
        world.goal_host = Some(game);
        world.live_mobs = Some(&live);
        let epoch = game.native_lifecycle.epoch().unwrap();
        let entity = EntityRef::new(epoch, other.id);
        let own = LivingRef::new(EntityRef::new(epoch, actor.id));
        let context = GoalCx {
            m: &mut actor,
            w: &world,
            host: game,
        };
        assert_eq!(entity.rotation(&context), Some([37.0, -12.0]));
        let living = LivingRef::new(entity);
        assert_eq!(living.max_health(&context), Some(73.0));
        assert_eq!(
            own.visit_effects(&context, &mut |_| panic!("ticking mob has no effects")),
            Ok(ControlFlow::Continue(()))
        );
        let mut effects = Vec::new();
        assert_eq!(
            living.visit_effects(&context, &mut |effect| {
                effects.push(effect);
                ControlFlow::Continue(())
            }),
            Ok(ControlFlow::Continue(()))
        );
        let night_vision =
            mod_platform::__private::part_tests::effect_key("minecraft:night_vision");
        assert_eq!(
            effects,
            [ActiveEffect::new(night_vision, 80, 1, false, true, true)]
        );
        let speed = "minecraft:movement_speed";
        assert_eq!(
            own.visit_attributes(&context, &mut |id| {
                if id == speed {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            }),
            Ok(ControlFlow::Break(()))
        );
        let mut modifiers = Vec::new();
        assert_eq!(
            living.visit_attribute_modifiers(&context, speed, &mut |m| {
                modifiers.push((m.id.to_owned(), m.amount, m.operation));
                ControlFlow::Break(())
            }),
            Ok(ControlFlow::Break(()))
        );
        assert_eq!(
            modifiers,
            [(
                "test:loaned_speed".to_owned(),
                0.25,
                AttributeOperation::AddMultipliedTotal
            )]
        );
        let stale_living = LivingRef::new(EntityRef::new(epoch, stale.id));
        assert_eq!(
            stale_living.visit_effects(&context, &mut |_| panic!("unloaned effects visited")),
            Err(EntityReadError::Stale)
        );
        assert!(context.lookup_entity(other.id).is_some());
        assert!(context.lookup_entity(stale.id).is_none());
        assert!(context.lookup_entity(9999).is_none());
    });
}

#[test]
#[ignore = "daily"]
fn scoped_host_resolves_other_real_players_from_the_game() {
    use std::sync::{
        Arc,
        atomic::{AtomicI32, AtomicUsize},
    };
    use sunrise_gameplay_common::game::test_fixtures::collect_entity_visit;
    use sunrise_protocol::packets::configuration::ClientInformation;
    use sunrise_sim::lan::{Joined, PlayerSender, guest::Guest};

    with_game(|game| {
        sunrise_gameplay::game::server_side::native_item_places::tests::open_to_lan(game);
        for id in [12, 13] {
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut guest = Guest::new(
                Joined {
                    conn: id,
                    uuid: u128::from(id),
                    name: format!("guest{id}"),
                    properties: Vec::new(),
                    info: ClientInformation::new(8),
                    brand: String::new(),
                    sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                    latency: Arc::new(AtomicI32::new(0)),
                    address: "127.0.0.1:1".parse().unwrap(),
                    expected: false,
                },
                id as i32,
                [f64::from(id), 64.0, 5.0],
                10,
            );
            guest.placed = true;
            if id == 13 {
                guest.vehicle = Some(9002);
                guest.vehicle_order = 0;
                let mut body = sunrise_sim::motion::PlayerMotion::new(
                    game.motion,
                    Vec3::new(f64::from(id), 64.0, 5.0),
                )
                .unwrap();
                body.entity_id = id as i32;
                body.set_modifier(
                    "minecraft:max_health",
                    "test:guest_health",
                    2.0,
                    "add_value",
                    true,
                )
                .unwrap();
                guest.motion = Some(Box::new(body));
            }
            game.server.lan.as_mut().unwrap().hub.guests.push(guest);
        }
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let mut actor = Mob::new(9001, kind.clone(), Vec3::ZERO, 0.0);
        actor.extra.mount = Some(9002);
        actor.extra.vehicle_order = 0;
        let vehicle = Mob::new(9002, kind.clone(), Vec3::ZERO, 0.0);
        let live = OneMob(&vehicle);
        // The player's child remains in Game, so backtracking must cross from Game
        // to the player and then the loaned vehicle before visiting the actor.
        let mut carried = Mob::new(9003, kind, Vec3::ZERO, 0.0);
        carried.extra.mount = Some(13);
        fixture_resident_column(&mut game.world, (0, 0));
        game.mobs.push(carried);
        let mut world = game.mob_world_base(&[]);
        world.goal_host = Some(game);
        world.live_mobs = Some(&live);
        // Deliberately select only one guest: the other remains a real live
        // player and must resolve through the underlying game host.
        world.player_identity = Some(([0, 0, 0, 12], 12));
        let context = GoalCx {
            m: &mut actor,
            w: &world,
            host: game,
        };
        let player = context.lookup_player(13).expect("other guest is online");
        assert_eq!(player.entity().id(), 13);
        assert_eq!(context.lookup_entity(13), Some(player.entity()));
        let living = context.lookup_living(player.entity()).unwrap();
        let mut modifiers = Vec::new();
        assert_eq!(
            living.visit_attribute_modifiers(&context, "minecraft:max_health", &mut |m| {
                modifiers.push((m.id.to_owned(), m.amount, m.operation));
                ControlFlow::Continue(())
            }),
            Ok(ControlFlow::Continue(()))
        );
        assert_eq!(
            modifiers,
            [(
                "test:guest_health".to_owned(),
                2.0,
                AttributeOperation::AddValue
            )]
        );
        assert_eq!(
            player.entity().position(&context),
            player.entity().position(game)
        );
        assert!(context.lookup_player(999).is_none());

        let vehicle = EntityRef::new(player.entity().epoch(), 9002);
        let active = EntityRef::new(vehicle.epoch(), 9001);
        let carried = EntityRef::new(vehicle.epoch(), 9003);
        assert_eq!(
            collect_entity_visit(|visit| vehicle.visit_passengers(&context, visit)),
            Ok(vec![player.entity(), active]),
            "player seats precede loaned mobs at equal boarding places"
        );
        assert_eq!(
            collect_entity_visit(|visit| vehicle.visit_indirect_passengers(&context, visit)),
            Ok(vec![player.entity(), carried, active]),
            "a player lends Game's riders before the loaned vehicle's next seat"
        );

        // A temporarily hidden player still owns its seat and the link back to the root.
        game.server.held_out.push(player.entity().id());
        let mut world = game.mob_world_base(&[]);
        world.goal_host = Some(game);
        world.live_mobs = Some(&live);
        let context = GoalCx {
            m: &mut actor,
            w: &world,
            host: game,
        };
        assert_eq!(
            collect_entity_visit(|visit| vehicle.visit_indirect_passengers(&context, visit)),
            Ok(vec![carried, active])
        );
        assert_eq!(game.server.held_out.pop(), Some(player.entity().id()));
    });
}

/// Authoritative living body yaw, including subclass overrides and the scoped goal read.
#[test]
#[ignore = "daily"]
fn body_yaw_targets_live_living_owners_and_respects_overrides() {
    use sunrise_gameplay::game::{
        native_entity_refs::tests::run_fresh_cases, native_guest_turn_tests::with_guests,
    };
    if run_fresh_cases(
        "cut_layout::game::server_side::native_goal_ticks::live_read_tests::body_yaw_targets_live_living_owners_and_respects_overrides",
        "SUNRISE_SERVER_BODY_YAW_CASE",
        &["owners"],
    ) {
        return;
    }

    with_guests(|game| {
        fixture_resident_column(&mut game.world, (0, 0));
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let mut mob = Mob::new(40, kind.clone(), Vec3::ZERO, 5.0);
        mob.head_yaw = 45.0;
        game.mobs.push(mob);
        let entity = game.entity(40).expect("the mob's handle");
        assert_eq!(entity.set_body_yaw(game, -725.5), Ok(true));
        assert_eq!(entity.body_yaw(game), Some(-725.5));
        assert_eq!(entity.head_yaw(game), Some(45.0));
        assert_eq!(entity.rotation(game), Some([5.0, 0.0]));
        assert_eq!(game.mobs[0].previous_body_yaw, 5.0);
        assert!(!game.mobs[0].needs_sync);

        for (id, name) in [(48, "minecraft:silverfish"), (49, "minecraft:endermite")] {
            let kind = game.res.mob_types().get(name).unwrap().clone();
            game.mobs.push(Mob::new(id, kind, Vec3::ZERO, 5.0));
            let insect = game.entity(id).unwrap();
            assert_eq!(insect.set_body_yaw(game, 405.5), Ok(true));
            assert_eq!(insect.rotation(game), Some([405.5, 0.0]));
            assert_eq!(insect.head_yaw(game), Some(5.0));
            assert_eq!(insect.set_body_yaw(game, f32::NAN), Ok(true));
            assert!(insect.body_yaw(game).unwrap().is_nan());
            assert_eq!(insect.rotation(game), Some([405.5, 0.0]));
        }
        use sunrise_sim::entity_nbt::{armor_stand_from_nbt, armor_stand_to_nbt};
        let stand = sunrise_sim::entities::ArmorStand::new(50, Vec3::ZERO, 12.0);
        let saved = armor_stand_to_nbt(&stand, [0; 4], None, game.items());
        game.armor_stands
            .push(armor_stand_from_nbt(&saved, 50).unwrap());
        let stand = game.entity(50).unwrap();
        assert_eq!(stand.body_yaw(game), Some(0.0));
        assert_eq!(stand.head_yaw(game), Some(12.0));
        assert_eq!(game.armor_stands[0].previous_yaw, 12.0);
        assert_eq!(stand.set_body_yaw(game, -405.5), Ok(true));
        assert_eq!(stand.body_yaw(game), Some(0.0));
        assert_eq!(stand.rotation(game), Some([12.0, 0.0]));
        assert_eq!(stand.head_yaw(game), Some(-405.5));
        assert_eq!(game.armor_stands[0].previous_yaw, -405.5);
        assert_eq!(game.armor_stands[0].previous_body_yaw, -405.5);
        assert_eq!(game.armor_stands[0].previous_head_yaw, -405.5);
        // EntityDataAccessor reloads the existing body rather than resetting it.
        game.armor_stands[0].body_yaw = 50.0;
        game.apply_entity_nbt(
            sunrise_gameplay_common::game::commands_common::Target::Stand(50),
            saved,
        )
        .unwrap();
        assert_eq!(stand.body_yaw(game), Some(50.0));
        assert_eq!(stand.head_yaw(game), Some(12.0));

        let stale = EntityRef::new(
            mod_api::WorldEpoch::new(entity.epoch().get() + 1),
            entity.id(),
        );
        assert_eq!(stale.body_yaw(game), None);
        assert_eq!(stale.set_body_yaw(game, 10.0), Ok(false));

        let [guest, other_guest] = [12, 13].map(|conn| game.player(conn).unwrap().entity());
        assert_eq!(guest.body_yaw(game), None);
        game.as_guest(12, |game| {
            assert_eq!(guest.body_yaw(game), Some(0.0));
            assert_eq!(guest.set_body_yaw(game, 85.0), Ok(true));
            assert_eq!(other_guest.body_yaw(game), None);
        })
        .unwrap();
        assert_eq!(guest.body_yaw(game), Some(85.0));
        game.as_guest(13, |game| {
            assert_eq!(other_guest.body_yaw(game), Some(0.0));
            assert_eq!(guest.body_yaw(game), Some(85.0));
        })
        .unwrap();
        game.guest_server_tick(12, true);
        assert_eq!(guest.body_yaw(game), Some(50.0));
        game.boats
            .push(sunrise_gameplay_common::game::boats_common::Boat::new(
                51,
                "oak".into(),
                Vec3::ZERO,
                385.0,
            ));
        let hub = &mut game.server.lan.as_mut().unwrap().hub;
        hub.update_guest(12, |g| g.vehicle = Some(51)).unwrap();
        game.guest_vehicle_tick(12);
        assert_eq!(guest.body_yaw(game), Some(385.0));

        game.server.others.0.push(
            sunrise_gameplay_common::game::commands_common::OtherEntity {
                size_answer: None,
                attachment_lifetime: Default::default(),
                knot_check: None,
                eye: None,
                join: Default::default(),
                display: None,
                interaction: None,
                id: 41,
                kind: "minecraft:marker".into(),
                position: Vec3::ZERO,
                yaw: 30.0,
                pitch: 0.0,
            },
        );
        let marker = game.entity(41).expect("the marker's handle");
        assert_eq!(marker.body_yaw(game), None);
        assert_eq!(marker.set_body_yaw(game, 15.0), Ok(false));

        let mut dragon =
            sunrise_gameplay_common::game::dragon_common::Dragon::new(Vec3::ZERO, 540.75, 0);
        dragon.id = 100;
        game.dragon.replace(dragon);
        let dragon = game.entity(100).expect("the dragon's handle");
        assert!(game.mob(dragon).is_some());
        assert_eq!(dragon.body_yaw(game), Some(540.75));
        assert_eq!(dragon.set_body_yaw(game, 71.0), Ok(true));
        assert_eq!(game.dragon.as_ref().unwrap().yaw, 540.75);
        let stale_dragon = EntityRef::new(stale.epoch(), dragon.id());
        assert_eq!(stale_dragon.body_yaw(game), None);
        let part = game.entity(101).expect("the dragon head's handle");
        assert_eq!(part.body_yaw(game), None);

        let other = game
            .mobs
            .iter()
            .find(|mob| mob.id == 40)
            .expect("the mob row");
        let snapshot = [sunrise_gameplay_common::game::creatures::seen(
            other,
            game.level.player_uuid,
            game.items(),
        )];
        let live = OneMob(other);
        let mut actor = Mob::new(9001, kind, Vec3::ZERO, 0.0);
        actor.body_yaw = 815.25;
        let own = EntityRef::new(entity.epoch(), actor.id);
        let mut world = game.mob_world_base(&snapshot);
        world.goal_host = Some(game);
        world.live_mobs = Some(&live);
        let context = GoalCx {
            m: &mut actor,
            w: &world,
            host: game,
        };
        assert_eq!(own.body_yaw(&context), Some(815.25));
        assert_eq!(entity.body_yaw(&context), Some(-725.5));
        assert_eq!(dragon.body_yaw(&context), Some(71.0));
        assert_eq!(stale_dragon.body_yaw(&context), None);
        assert_eq!(part.body_yaw(&context), None);
        assert_eq!(stale.body_yaw(&context), None);
        assert_eq!(guest.body_yaw(&context), Some(385.0));
        assert_eq!(other_guest.body_yaw(&context), Some(0.0));
        assert_eq!(marker.body_yaw(&context), None);
        context.m.removed = true;
        assert_eq!(own.body_yaw(&context), None);

        let role = game.options.execution_role;
        game.options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
        assert_eq!(entity.body_yaw(game), None);
        assert_eq!(dragon.body_yaw(game), None);
        game.options.execution_role = role;

        game.mobs
            .iter_mut()
            .find(|mob| mob.id == 40)
            .unwrap()
            .removed = true;
        assert_eq!(entity.body_yaw(game), None);
        game.dragon.discard();
        assert_eq!(dragon.body_yaw(game), None);
    });
}

/// A player's `getLastHurtMob` read in a goal tick of the very mob it names: the game alone
/// cannot see the mob while it is lent to its tick, and the tick's view resolves it.
#[test]
#[ignore = "daily"]
fn a_players_last_hurt_mob_resolves_the_mob_lent_to_its_tick() {
    use sunrise_level::chunk::Chunk;
    use sunrise_sim::motion::PlayerMotion;
    use {
        sunrise_gameplay::game::native_entity_refs::tests::bind_local_player,
        sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    };

    // Loading resources replaces process-wide tables: the case runs in its own process.
    if run_fresh_cases(
        "cut_layout::game::server_side::native_goal_ticks::live_read_tests::a_players_last_hurt_mob_resolves_the_mob_lent_to_its_tick",
        "SUNRISE_GOAL_LAST_HURT_MOB_CASE",
        &["lent"],
    ) {
        return;
    }
    with_game(|game| {
        game.enter_level(sunrise_gameplay_common::game::Dim::Overworld)
            .unwrap();
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), std::sync::Arc::new(Chunk::empty(0, 0)));
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 80.0, 0.5)).unwrap());
        bind_local_player(game, 20, 123);
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        game.mobs
            .push(Mob::new(72, kind, Vec3::new(3.0, 80.0, 0.5), 0.0));
        game.player.as_mut().unwrap().health.last_hurt_mob = Some((72, 39));
        let player = game.living(game.entity(20).unwrap()).unwrap();
        let zombie = game.entity(72).unwrap();
        assert_eq!(player.last_hurt_mob(game), Some((zombie, 39)));

        let mut loan = game.mobs.lift();
        let index = loan.iter().position(|mob| mob.id == 72).unwrap();
        let reads = loan
            .with_one_and_rest(index, |actor, _, _| {
                let mut world = game.mob_world_base(&[]);
                world.goal_host = Some(game);
                let context = GoalCx {
                    m: actor,
                    w: &world,
                    host: game,
                };
                (player.last_hurt_mob(game), player.last_hurt_mob(&context))
            })
            .unwrap();
        game.mobs
            .restore(loan, sunrise_sim::join_seq::RestoreOrder::SurvivorsFirst);
        assert_eq!(reads, (None, Some((zombie, 39))));
    });
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt6 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
