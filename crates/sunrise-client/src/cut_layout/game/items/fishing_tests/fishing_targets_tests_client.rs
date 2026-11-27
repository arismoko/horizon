use crate::game::server_side::fishing::fishing_targets_tests::FishingCase;
use crate::game::server_side::fishing::fishing_targets_tests::FishingExtraCase;
use crate::game::server_side::fishing::fishing_targets_tests::TrackedTarget;
use crate::game::server_side::fishing::fishing_targets_tests::align_next_entity_mod4;
use crate::game::server_side::fishing::fishing_targets_tests::check_hook_tick;
use crate::game::server_side::fishing::fishing_targets_tests::clear_entities;
use crate::game::server_side::fishing::fishing_targets_tests::spawn_test_hook;
use crate::game::server_side::fishing::fishing_targets_tests::summon_target;
use crate::game::server_side::fishing::fishing_targets_tests::target_kinematics;
use crate::game::server_side::fishing::fishing_targets_tests::vec_bits;
use sunrise_core::geometry::Vec3;
use sunrise_core::random::RandomSource;
use sunrise_gameplay_common::game::commands_common::Target;
use sunrise_gameplay_common::game::fishing_common::State;
use sunrise_items::items::ItemStack;

pub(crate) fn reset_arena(game: &mut sunrise_gameplay::game::ServerGame<'_>, target: &str) {
    clear_entities(game);
    let _ = game.run_command("fill 0 63 0 11 63 9 minecraft:stone");
    let _ = game.run_command("fill 0 64 0 11 70 9 minecraft:air");
    let _ = game.run_command("fill 10 64 2 10 67 6 minecraft:stone");
    let wall = target == "painting"
        || target.starts_with("item_frame")
        || target.starts_with("glow_item_frame");
    if wall {
        let _ = game.run_command("fill 4 64 2 4 66 6 minecraft:stone");
    }
    if target.starts_with("leash_knot") {
        let _ = game.run_command("setblock 3 64 4 minecraft:oak_fence");
    }
    if game.riding.is_some() {
        game.stop_riding();
    }
    game.set_mode(sunrise_command::args::GameMode::Survival);
    game.yaw = -90.0;
    game.pitch = 0.0;
    if let Some(p) = game.player.as_mut() {
        p.position = Vec3::new(0.5, 64.0, 4.5);
        p.velocity = Vec3::ZERO;
    }
    game.selected = 0;
    game.inventory[0] = Some(ItemStack::new("minecraft:fishing_rod", 1));
    game.tick_server(false).unwrap();
    game.start_audio_capture();
}

pub(crate) fn run_case(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    case: &FishingCase,
    hook_seed: i64,
    level_seed: i64,
) -> Vec<String> {
    let mut errors = Vec::new();
    let label = &case.target;
    reset_arena(game, &case.target);
    game.world.random.set_seed(level_seed);

    if case.target == "dropped_item" {
        align_next_entity_mod4(game, case.target_id_mod4);
    }
    let tracked = summon_target(game, &case.target);
    if let TrackedTarget::Item(id) = tracked {
        assert_eq!(id & 3, case.target_id_mod4, "ItemEntity tick phase");
    }
    spawn_test_hook(
        game,
        Vec3::new(1.5, case.spawn_y, 4.5),
        Vec3::new(1.0, 0.0, 0.0),
        hook_seed,
    );
    game.world.random.set_seed(level_seed);

    let is_dragon = matches!(tracked, TrackedTarget::Dragon);
    for expected in &case.ticks {
        game.start_audio_capture();
        sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS.with(|e| {
            *e.borrow_mut() = Some(Vec::new());
        });
        game.tick_server(false).unwrap();
        let posted = sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS
            .with(|e| e.borrow_mut().take().unwrap_or_default());
        check_hook_tick(
            game,
            label,
            expected,
            tracked.id(),
            is_dragon,
            &posted,
            &mut errors,
        );
        if !errors.is_empty() {
            break;
        }
    }

    // Retrieve rod and verify exact durability damage and bit-exact pull velocity.
    let rod = ItemStack::new("minecraft:fishing_rod", 1);
    let damage = game.retrieve_hook(&rod) as i32;
    if damage != case.retrieve_damage {
        errors.push(format!(
            "{label}: retrieve damage expected {} got {damage}",
            case.retrieve_damage
        ));
    }
    if let Some((_, vel)) = target_kinematics(game, tracked) {
        let got_vel = vec_bits(vel);
        if got_vel != case.target_vel_after_retrieve_bits {
            errors.push(format!(
                "{label}: vel after retrieve expected {:?} got {got_vel:?} ({vel:?})",
                case.target_vel_after_retrieve_bits
            ));
        }
    }
    game.tick_server(false).unwrap();
    if let Some((pos, _)) = target_kinematics(game, tracked) {
        let got_pos = vec_bits(pos);
        if got_pos != case.target_pos_after_retrieve_tick_bits {
            errors.push(format!(
                "{label}: pos after retrieve tick expected {:?} got {got_pos:?} ({pos:?})",
                case.target_pos_after_retrieve_tick_bits
            ));
        }
    }
    errors
}

pub(crate) fn run_extra_case(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    case: &FishingExtraCase,
    hook_seed: i64,
    level_seed: i64,
) -> Vec<String> {
    let mut errors = Vec::new();
    let label = format!("extra {}", case.case);
    reset_arena(game, "extra");
    game.world.random.set_seed(level_seed);

    let (expected_hooked_id, tracked_item): (Option<u32>, Option<u32>) = match case.case.as_str() {
        "owner_vehicle_passenger" => {
            assert!(game.run_command("summon minecraft:oak_boat 2.5 64.0 4.5 {}"));
            let boat_id = game.boats.last().unwrap().id;
            assert!(game.run_command("summon minecraft:pig 2.5 64.0 4.5 {NoAI:1b,Silent:1b}"));
            let pig_id = game.mobs.last().unwrap().id;
            assert!(game.start_riding_target(Target::Player, Target::Boat(boat_id), true));
            assert!(game.start_riding_target(Target::Mob(pig_id), Target::Boat(boat_id), true));
            assert!(game.run_command("summon minecraft:armor_stand 5.5 64.0 4.5 {}"));
            let stand_id = game.armor_stands.last().unwrap().id;
            game.tick_server(false).unwrap();
            spawn_test_hook(
                game,
                Vec3::new(1.5, 64.8, 4.5),
                Vec3::new(1.0, 0.0, 0.0),
                hook_seed,
            );
            (Some(stand_id), None)
        }
        "deflected_hook_catches_owner" => {
            assert!(game.run_command("summon minecraft:breeze 3.5 64.0 4.5 {NoAI:1b,Silent:1b}"));
            spawn_test_hook(
                game,
                Vec3::new(1.8, 65.4, 4.5),
                Vec3::new(1.0, 0.0, 0.0),
                hook_seed,
            );
            (Some(game.own_entity_id() as u32), None)
        }
        "wall_endpoint_boundary" => {
            assert!(game.run_command("setblock 4 64 4 minecraft:stone"));
            assert!(game.run_command("summon minecraft:armor_stand 4.25 64.0 4.75 {NoGravity:1b}"));
            spawn_test_hook(
                game,
                Vec3::new(3.5, 64.5, 4.5),
                Vec3::new(1.0, 0.0, 0.0),
                hook_seed,
            );
            (None, None)
        }
        "tick8_margin_boundary" => {
            assert!(
                game.run_command(
                    "summon minecraft:armor_stand 6.9 64.0 5.050000005 {NoGravity:1b}"
                )
            );
            let stand_id = game.armor_stands.last().unwrap().id;
            spawn_test_hook(
                game,
                Vec3::new(1.5, 65.5, 4.5),
                Vec3::new(0.85, 0.0, 0.0),
                hook_seed,
            );
            (Some(stand_id), None)
        }
        "same_tick_retrieve_item" => {
            let mod4 = case.target_id_mod4.expect("target_id_mod4");
            align_next_entity_mod4(game, mod4);
            assert!(game.run_command(
                r#"summon minecraft:item 3.5 64.0 4.5 {Item:{id:"minecraft:diamond",count:1},Motion:[0.0d,0.0d,0.0d],PickupDelay:32767}"#
            ));
            let item_id = game.dropped.last().unwrap().id;
            assert_eq!(item_id & 3, mod4, "same_tick_retrieve_item tick phase");
            spawn_test_hook(
                game,
                Vec3::new(1.5, 64.15, 4.5),
                Vec3::new(1.0, 0.0, 0.0),
                hook_seed,
            );
            (Some(item_id), Some(item_id))
        }
        other => panic!("unexpected fishing extra case {other}"),
    };
    game.world.random.set_seed(level_seed);

    for expected in &case.ticks {
        game.start_audio_capture();
        sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS.with(|e| {
            *e.borrow_mut() = Some(Vec::new());
        });
        game.tick_server(false).unwrap();
        let posted = sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS
            .with(|e| e.borrow_mut().take().unwrap_or_default());
        check_hook_tick(
            game,
            &label,
            expected,
            expected_hooked_id,
            false,
            &posted,
            &mut errors,
        );
        let exp_deflect = expected
            .sounds
            .iter()
            .filter(|s| s.as_str() == "minecraft:entity.breeze.deflect")
            .count();
        let got_deflect = game
            .server
            .pending_sounds
            .iter()
            .filter(|s| s.event == "minecraft:entity.breeze.deflect")
            .count();
        if got_deflect != exp_deflect {
            errors.push(format!(
                "{label} t={}: breeze.deflect count expected {exp_deflect} got {got_deflect}",
                expected.tick
            ));
        }
        if !errors.is_empty() {
            break;
        }
        if case.retrieve_on_tick == Some(expected.tick) {
            let state_now = match game.hook.as_ref().map(|h| h.state) {
                Some(State::Flying) => "FLYING",
                Some(State::Hooked(_)) => "HOOKED_IN_ENTITY",
                Some(State::Bobbing) => "BOBBING",
                None => "NONE",
            };
            if case.state_at_retrieve.as_deref() != Some(state_now) {
                errors.push(format!(
                    "{label} t={}: state_at_retrieve expected {:?} got {state_now}",
                    expected.tick, case.state_at_retrieve
                ));
            }
            let rod = ItemStack::new("minecraft:fishing_rod", 1);
            let damage = game.retrieve_hook(&rod) as i32;
            if Some(damage) != case.retrieve_damage {
                errors.push(format!(
                    "{label}: retrieve_damage expected {:?} got {damage}",
                    case.retrieve_damage
                ));
            }
            if let Some(item_id) = tracked_item {
                let (_, vel) = target_kinematics(game, TrackedTarget::Item(item_id)).unwrap();
                if Some(vec_bits(vel)) != case.target_vel_after_retrieve_bits {
                    errors.push(format!(
                        "{label}: vel after retrieve expected {:?} got {:?}",
                        case.target_vel_after_retrieve_bits,
                        vec_bits(vel)
                    ));
                }
                game.tick_server(false).unwrap();
                let (pos, vel2) = target_kinematics(game, TrackedTarget::Item(item_id)).unwrap();
                if Some(vec_bits(pos)) != case.target_pos_after_retrieve_tick_bits
                    || Some(vec_bits(vel2)) != case.target_vel_after_retrieve_tick_bits
                {
                    errors.push(format!(
                        "{label}: after retrieve tick expected {:?}/{:?} got {:?}/{:?}",
                        case.target_pos_after_retrieve_tick_bits,
                        case.target_vel_after_retrieve_tick_bits,
                        vec_bits(pos),
                        vec_bits(vel2)
                    ));
                }
            }
        }
    }
    errors
}

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
