//! `start_riding`'s named refusals (#4456 part 1): one test per cause, each
//! with the refusal it must name, and the engine's bool adapter answering the same.
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use mod_api::{EntityMountEvent, EntityRef, Flow, Hooks, ServerHost, StartRidingRefusal};
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::test_fixtures::fixture_resident_column;
use sunrise_sim::mobs::Mob;

use {
    sunrise_gameplay::game::native_entity_refs::tests::{with_game, with_resident_game},
    sunrise_gameplay_common::game::Dim,
    sunrise_gameplay_common::game::commands_common::Target,
};

fn add_mob(game: &mut sunrise_gameplay::game::ServerGame<'_>, id: u32, name: &str) -> EntityRef {
    let kind = game.res.mob_types().get(name).unwrap().clone();
    game.mobs.push(Mob::new(id, kind, Vec3::ZERO, 0.0));
    game.entity(id).unwrap()
}

/// Both calls on the same mount: the named answer, and the bool that is its `is_ok()`.
fn try_both(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    rider: Target,
    vehicle: Target,
    force: bool,
) -> Result<(), StartRidingRefusal> {
    let named = game.try_start_riding_target(rider, vehicle, force);
    assert!(
        named.is_err(),
        "a refusal case must not mount, so the bool call can be asked again"
    );
    assert!(!game.start_riding_target(rider, vehicle, force));
    named
}

/// Counts `entity_mount` calls, and cancels them while `cancel` is set.
struct MountCounter {
    pub heard: Arc<Mutex<u32>>,
    cancel: Arc<AtomicBool>,
}
#[mod_api::__subscriptions]
impl Hooks for MountCounter {
    fn entity_mount(&self, _: &mut dyn ServerHost, _: &mut EntityMountEvent<'_>) -> Flow {
        *self.heard.lock().unwrap() += 1;
        if self.cancel.load(Ordering::SeqCst) {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

fn with_counter(
    cancel: bool,
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, &Mutex<u32>),
) {
    let heard = Arc::new(Mutex::new(0));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "start_riding_refusal_counter",
        Box::new(MountCounter {
            heard: heard.clone(),
            cancel: Arc::new(AtomicBool::new(cancel)),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_resident_game(|game| {
            game.options.dedicated = Some(Vec::new());
            test(game, &heard);
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_player_vehicle_is_refused_by_name_forced_or_not_before_entity_mount() {
    with_counter(false, |game, heard| {
        add_mob(game, 40, "minecraft:zombie");
        for force in [false, true] {
            assert_eq!(
                try_both(game, Target::Mob(40), Target::Player, force),
                Err(StartRidingRefusal::PlayerVehicle)
            );
        }
        assert_eq!(game.mobs[0].extra.mount, None);
        assert_eq!(*heard.lock().unwrap(), 0, "refused before the hook");
    });
}

#[test]
#[ignore = "daily"]
fn a_cycle_is_refused_by_name() {
    with_resident_game(|game| {
        add_mob(game, 30, "minecraft:pig");
        add_mob(game, 31, "minecraft:pig");
        assert_eq!(
            game.try_start_riding_target(Target::Mob(30), Target::Mob(31), true),
            Ok(())
        );
        assert_eq!(
            try_both(game, Target::Mob(31), Target::Mob(30), true),
            Err(StartRidingRefusal::Cycle)
        );
        assert_eq!(game.mobs[1].extra.mount, None);
    });
}

#[test]
#[ignore = "daily"]
fn a_removed_vehicle_is_refused_as_no_entity() {
    with_resident_game(|game| {
        let rider = add_mob(game, 50, "minecraft:zombie");
        let vehicle = add_mob(game, 51, "minecraft:pig");
        game.mobs.retain(|m| m.id != 51);
        game.detach_entity(51);
        assert_eq!(
            game.entity_start_riding(rider, vehicle, true),
            Err(StartRidingRefusal::NoEntity)
        );
        assert_eq!(
            try_both(game, Target::Mob(50), Target::Mob(51), true),
            Err(StartRidingRefusal::NoEntity)
        );
        assert_eq!(game.mobs[0].extra.mount, None);
    });
}

#[test]
#[ignore = "daily"]
fn a_vehicle_in_another_level_is_refused_as_no_entity() {
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.native_level_opened(Dim::Overworld);
        fixture_resident_column(&mut game.world, (0, 0));
        game.native_joins_skip();
        let vehicle = add_mob(game, 60, "minecraft:pig");
        game.enter_level(Dim::Nether).unwrap();
        fixture_resident_column(&mut game.world, (0, 0));
        let rider = add_mob(game, 61, "minecraft:zombie");
        for force in [false, true] {
            assert_eq!(
                game.entity_start_riding(rider, vehicle, force),
                Err(StartRidingRefusal::NoEntity)
            );
        }
        game.enter_level(Dim::Overworld).unwrap();
    });
}

#[test]
#[ignore = "daily"]
fn a_full_vehicle_is_refused_by_name_unless_forced() {
    with_resident_game(|game| {
        add_mob(game, 70, "minecraft:zombie");
        add_mob(game, 71, "minecraft:pig");
        add_mob(game, 72, "minecraft:skeleton");
        assert_eq!(
            game.try_start_riding_target(Target::Mob(70), Target::Mob(71), true),
            Ok(())
        );
        // A pig carries one passenger (`Entity.canAddPassenger`); `force` skips that check.
        assert_eq!(
            try_both(game, Target::Mob(72), Target::Mob(71), false),
            Err(StartRidingRefusal::Full)
        );
        assert_eq!(game.mobs[2].extra.mount, None);
        assert_eq!(
            game.try_start_riding_target(Target::Mob(72), Target::Mob(71), true),
            Ok(())
        );
    });
}

#[test]
#[ignore = "daily"]
fn the_same_entity_and_a_second_mount_on_the_same_vehicle_are_refused_by_name() {
    with_resident_game(|game| {
        add_mob(game, 80, "minecraft:zombie");
        add_mob(game, 81, "minecraft:pig");
        assert_eq!(
            try_both(game, Target::Mob(80), Target::Mob(80), true),
            Err(StartRidingRefusal::SameEntity)
        );
        assert_eq!(
            game.try_start_riding_target(Target::Mob(80), Target::Mob(81), true),
            Ok(())
        );
        assert_eq!(
            try_both(game, Target::Mob(80), Target::Mob(81), true),
            Err(StartRidingRefusal::AlreadyRiding)
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_boarding_cooldown_is_refused_as_cannot_ride_unless_forced() {
    with_resident_game(|game| {
        add_mob(game, 90, "minecraft:zombie");
        add_mob(game, 91, "minecraft:pig");
        game.mobs[0].extra.boarding_cooldown = 10;
        assert_eq!(
            try_both(game, Target::Mob(90), Target::Mob(91), false),
            Err(StartRidingRefusal::CannotRide)
        );
        assert_eq!(
            game.try_start_riding_target(Target::Mob(90), Target::Mob(91), true),
            Ok(())
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_canceled_entity_mount_is_refused_as_canceled_after_the_checks() {
    with_counter(true, |game, heard| {
        add_mob(game, 100, "minecraft:zombie");
        add_mob(game, 101, "minecraft:pig");
        assert_eq!(
            game.try_start_riding_target(Target::Mob(100), Target::Mob(101), true),
            Err(StartRidingRefusal::Canceled)
        );
        assert_eq!(*heard.lock().unwrap(), 1);
        assert_eq!(game.mobs[0].extra.mount, None);
    });
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
