//! Witness-driven parity tests for FishingHook hitting pickable targets and ItemEntities.
#![cfg(feature = "client")]
use serde::Deserialize;
use std::path::PathBuf;
use sunrise_core::geometry::Vec3;
use sunrise_core::random::LegacyRandom;
use {
    sunrise_gameplay_common::game::fishing_common::Hook,
    sunrise_gameplay_common::game::fishing_common::State,
};

#[derive(Deserialize)]
pub(crate) struct FishingWitness {
    pub(in crate::game) game_version: String,
    pub(in crate::game) hook_seed: i64,
    pub(in crate::game) level_seed: i64,
    pub(in crate::game) cases: Vec<FishingCase>,
    pub(in crate::game) extra_cases: Vec<FishingExtraCase>,
}

#[derive(Deserialize)]
pub(crate) struct FishingCase {
    pub(in crate::game) target: String,
    pub(in crate::game) spawn_y: f64,
    pub(in crate::game) target_id_mod4: u32,
    pub(in crate::game) ticks: Vec<FishingTick>,
    pub(in crate::game) retrieve_damage: i32,
    pub(in crate::game) target_vel_after_retrieve_bits: [String; 3],
    pub(in crate::game) target_pos_after_retrieve_tick_bits: [String; 3],
}

#[derive(Deserialize)]
pub(crate) struct FishingTick {
    pub(in crate::game) tick: usize,
    pub(in crate::game) hook_state: String,
    pub(in crate::game) hook_pos_bits: [String; 3],
    pub(in crate::game) hook_vel_bits: [String; 3],
    pub(in crate::game) hook_yaw_bits: String,
    pub(in crate::game) hooked_type: Option<String>,
    #[allow(dead_code)]
    pub(in crate::game) target_vel_bits: [String; 3],
    pub(in crate::game) game_events: Vec<GameEventSnap>,
    #[serde(default)]
    pub(in crate::game) sounds: Vec<String>,
}

#[derive(Deserialize)]
pub(in crate::game) struct GameEventSnap {
    pub(in crate::game) id: String,
    pub(in crate::game) pos_bits: [String; 3],
}

#[derive(Deserialize)]
pub(crate) struct FishingExtraCase {
    pub(in crate::game) case: String,
    pub(in crate::game) target_id_mod4: Option<u32>,
    pub(in crate::game) retrieve_on_tick: Option<usize>,
    pub(in crate::game) state_at_retrieve: Option<String>,
    pub(in crate::game) retrieve_damage: Option<i32>,
    pub(in crate::game) target_vel_after_retrieve_bits: Option<[String; 3]>,
    pub(in crate::game) target_pos_after_retrieve_tick_bits: Option<[String; 3]>,
    pub(in crate::game) target_vel_after_retrieve_tick_bits: Option<[String; 3]>,
    pub(in crate::game) ticks: Vec<FishingTick>,
}

pub(crate) fn load_witness() -> FishingWitness {
    let dir = std::env::var_os("SUNRISE_PROJECTILE_WITNESS_DIR").unwrap_or_else(|| {
        panic!(
            "SUNRISE_PROJECTILE_WITNESS_DIR is unset; run tools/test_projectile_targets_parity.py"
        )
    });
    let path = PathBuf::from(dir).join("fishing_witness.json");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "failed to read fishing witness {} (record via tools/test_projectile_targets_parity.py): {e}",
            path.display()
        )
    });
    let witness: FishingWitness = serde_json::from_str(&raw).expect("valid 26.3 fishing witness");
    assert_eq!(witness.game_version, "26.3");
    witness
}

pub(crate) fn bits(v: f64) -> String {
    format!("{:016x}", v.to_bits())
}

pub(crate) fn fbits(v: f32) -> String {
    format!("{:08x}", v.to_bits())
}

pub(crate) fn vec_bits(v: Vec3) -> [String; 3] {
    [bits(v.x), bits(v.y), bits(v.z)]
}

pub(crate) fn align_next_entity_mod4(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    want_mod4: u32,
) {
    while ((game.server.next_entity + 1) & 3) != (want_mod4 & 3) {
        game.server.next_entity += 1;
    }
}

pub(crate) fn setup_game<'a>(
    resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
) -> sunrise_gameplay::game::ServerGame<'a> {
    let mut game = sunrise_gameplay::game::game_tests::tests::server_game(resources);
    game.build_registries();
    game.set_allow_commands(true);
    game.load_ticking_neighbourhood(0, 0);
    assert!(game.run_command("forceload add 0 0"));
    assert!(game.run_command("difficulty normal"));
    for command in [
        "gamerule spawn_mobs false",
        "gamerule advance_time false",
        "gamerule advance_weather false",
        "gamerule mob_griefing true",
        "gamerule entity_drops true",
        "gamerule random_tick_speed 0",
    ] {
        let _ = game.run_command(command);
    }
    game.player = Some(
        sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 4.5)).unwrap(),
    );
    sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(&mut game, 7, 7);
    game.server.next_entity = game.server.next_entity.max(100);
    game
}

pub(crate) fn clear_entities(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    game.hook.discard();
    game.mobs.clear();
    game.dropped.clear();
    game.boats.clear();
    game.carts.clear();
    game.armor_stands.clear();
    game.tnts.clear();
    game.falling.clear();
    game.orbs.clear();
    game.creatures.projectiles.clear();
    game.rockets.clear();
    game.tridents.clear();
    game.item_frames.clear();
    game.paintings.clear();
    game.cushions.clear();
    game.crystals.clear();
    game.discard_resident_dragon();
    game.server.others.0.clear();
    game.server.knot_ids.clear();
    game.server.broken_knots.clear();
    game.resident_entities.knots.clear();
    game.reset_entity_residency();
    game.bind_local_player_residency();
}

#[derive(Clone, Copy)]
pub(crate) enum TrackedTarget {
    Frame(u32),
    Painting(u32),
    Falling(u32),
    Crystal(u32),
    Dragon,
    Stand(u32),
    ExplicitKnot(u32),
    NaturalKnot(u32),
    Cushion(u32),
    Item(u32),
}

impl TrackedTarget {
    pub(crate) fn id(self) -> Option<u32> {
        match self {
            Self::Frame(id)
            | Self::Painting(id)
            | Self::Falling(id)
            | Self::Crystal(id)
            | Self::Stand(id)
            | Self::ExplicitKnot(id)
            | Self::NaturalKnot(id)
            | Self::Cushion(id)
            | Self::Item(id) => Some(id),
            Self::Dragon => None,
        }
    }
}

pub(crate) fn summon_target(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    target: &str,
) -> TrackedTarget {
    match target {
        "item_frame" => {
            assert!(game.run_command(
                r#"summon minecraft:item_frame 3.5 64.5 4.5 {block_pos:[I;3,64,4],Facing:4b,Item:{id:"minecraft:diamond",count:1}}"#
            ));
            TrackedTarget::Frame(game.item_frames.last().unwrap().id)
        }
        "item_frame_empty" => {
            assert!(game.run_command(
                "summon minecraft:item_frame 3.5 64.5 4.5 {block_pos:[I;3,64,4],Facing:4b}"
            ));
            TrackedTarget::Frame(game.item_frames.last().unwrap().id)
        }
        "item_frame_fixed" => {
            assert!(game.run_command(
                r#"summon minecraft:item_frame 3.5 64.5 4.5 {block_pos:[I;3,64,4],Facing:4b,Fixed:1b,Item:{id:"minecraft:diamond",count:1}}"#
            ));
            TrackedTarget::Frame(game.item_frames.last().unwrap().id)
        }
        "glow_item_frame" => {
            assert!(game.run_command(
                r#"summon minecraft:glow_item_frame 3.5 64.5 4.5 {block_pos:[I;3,64,4],Facing:4b,Item:{id:"minecraft:diamond",count:1}}"#
            ));
            TrackedTarget::Frame(game.item_frames.last().unwrap().id)
        }
        "painting" => {
            assert!(game.run_command(
                r#"summon minecraft:painting 3.5 64.5 4.5 {block_pos:[I;3,64,4],facing:1b,variant:"minecraft:kebab"}"#
            ));
            TrackedTarget::Painting(game.paintings.last().unwrap().id)
        }
        "falling_block" => {
            assert!(game.run_command(
                r#"summon minecraft:falling_block 3.5 65.0 4.5 {BlockState:"minecraft:sand",Time:1,Motion:[0.0d,0.0d,0.0d]}"#
            ));
            TrackedTarget::Falling(game.falling.last().unwrap().id)
        }
        "end_crystal" => {
            assert!(game.run_command("summon minecraft:end_crystal 4.5 64.0 4.5 {}"));
            TrackedTarget::Crystal(game.crystals.last().unwrap().id)
        }
        "ender_dragon" => {
            assert!(game.run_command(
                "summon minecraft:ender_dragon 7.5 64.0 4.5 {DragonPhase:10,Silent:1b}"
            ));
            game.tick_server(false).unwrap();
            TrackedTarget::Dragon
        }
        "armor_stand" => {
            assert!(game.run_command("summon minecraft:armor_stand 3.5 64.0 4.5 {}"));
            TrackedTarget::Stand(game.armor_stands.last().unwrap().id)
        }
        "armor_stand_marker" => {
            assert!(game.run_command("summon minecraft:armor_stand 3.5 64.0 4.5 {Marker:1b}"));
            TrackedTarget::Stand(game.armor_stands.last().unwrap().id)
        }
        "armor_stand_small" => {
            assert!(game.run_command("summon minecraft:armor_stand 3.5 64.0 4.5 {Small:1b}"));
            TrackedTarget::Stand(game.armor_stands.last().unwrap().id)
        }
        "armor_stand_nogravity" => {
            assert!(game.run_command("summon minecraft:armor_stand 3.5 64.0 4.5 {NoGravity:1b}"));
            TrackedTarget::Stand(game.armor_stands.last().unwrap().id)
        }
        "leash_knot" => {
            assert!(game.run_command("summon minecraft:leash_knot 3.5 64.375 4.5 {}"));
            TrackedTarget::ExplicitKnot(game.server.others.0.last().unwrap().id)
        }
        "leash_knot_tied" => {
            assert!(game.run_command(
                "summon minecraft:pig 3.5 64.0 6.5 {NoAI:1b,Silent:1b,leash:[I;3,64,4]}"
            ));
            game.tick_server(false).unwrap();
            let id = *game
                .server
                .knot_ids
                .get(&[3, 64, 4])
                .expect("natural leash knot created on fence");
            TrackedTarget::NaturalKnot(id)
        }
        "cushion" => {
            assert!(game.run_command(
                r#"summon minecraft:cushion 3.5 64.0 4.5 {block_pos:[I;3,64,4],color:"white"}"#
            ));
            TrackedTarget::Cushion(game.cushions.last().unwrap().id)
        }
        "dropped_item" => {
            assert!(game.run_command(
                r#"summon minecraft:item 3.5 64.0 4.5 {Item:{id:"minecraft:diamond",count:1},Motion:[0.0d,0.0d,0.0d],PickupDelay:32767}"#
            ));
            TrackedTarget::Item(game.dropped.last().unwrap().id)
        }
        other => panic!("unexpected fishing witness target {other}"),
    }
}

pub(crate) fn target_kinematics(
    game: &sunrise_gameplay::game::ServerGame<'_>,
    target: TrackedTarget,
) -> Option<(Vec3, Vec3)> {
    match target {
        TrackedTarget::Falling(id) => game
            .falling
            .iter()
            .find(|f| f.id == id)
            .map(|f| (f.position, f.velocity)),
        TrackedTarget::Stand(id) => game
            .armor_stands
            .iter()
            .find(|a| a.id == id)
            .map(|a| (a.position, a.velocity)),
        TrackedTarget::Item(id) => game
            .dropped
            .iter()
            .find(|i| i.id == id)
            .map(|i| (i.position, i.velocity)),
        _ => None,
    }
}

pub(crate) fn check_hook_tick(
    game: &sunrise_gameplay::game::ServerGame<'_>,
    label: &str,
    expected: &FishingTick,
    expected_hooked_id: Option<u32>,
    is_dragon: bool,
    posted: &[(&'static str, [f64; 3], Option<sunrise_sim::mobs::Who>)],
    errors: &mut Vec<String>,
) {
    let exp_lands: Vec<&[String; 3]> = expected
        .game_events
        .iter()
        .filter(|g| g.id == "minecraft:projectile_land")
        .map(|g| &g.pos_bits)
        .collect();
    let got_lands: Vec<[String; 3]> = posted
        .iter()
        .filter(|(id, _, _)| *id == "minecraft:projectile_land")
        .map(|(_, at, _)| [bits(at[0]), bits(at[1]), bits(at[2])])
        .collect();
    if got_lands.len() != exp_lands.len() || got_lands.iter().zip(&exp_lands).any(|(a, e)| a != *e)
    {
        errors.push(format!(
            "{label} t={}: projectile_land expected {exp_lands:?} got {got_lands:?}",
            expected.tick
        ));
    }

    let Some(h) = game.hook.as_ref() else {
        errors.push(format!("{label} t={}: hook vanished", expected.tick));
        return;
    };
    let got_state = match h.state {
        State::Flying => "FLYING",
        State::Hooked(_) => "HOOKED_IN_ENTITY",
        State::Bobbing => "BOBBING",
    };
    if got_state != expected.hook_state {
        errors.push(format!(
            "{label} t={}: hook_state expected {} got {got_state}",
            expected.tick, expected.hook_state
        ));
    }
    if !is_dragon {
        let want_hooked = expected.hooked_type.is_some();
        if h.hooked_in.is_some() != want_hooked {
            errors.push(format!(
                "{label} t={}: hooked_in present expected {want_hooked} ({:?}) got {:?}",
                expected.tick, expected.hooked_type, h.hooked_in
            ));
        } else if want_hooked
            && let Some(tid) = expected_hooked_id
            && h.hooked_in != Some(tid)
        {
            errors.push(format!(
                "{label} t={}: hooked_in id expected {tid} got {:?}",
                expected.tick, h.hooked_in
            ));
        }
    }
    let got_pos = vec_bits(h.position);
    if got_pos != expected.hook_pos_bits {
        errors.push(format!(
            "{label} t={}: hook_pos expected {:?} got {got_pos:?} ({:?})",
            expected.tick, expected.hook_pos_bits, h.position
        ));
    }
    let got_vel = vec_bits(h.velocity);
    if got_vel != expected.hook_vel_bits {
        errors.push(format!(
            "{label} t={}: hook_vel expected {:?} got {got_vel:?} ({:?})",
            expected.tick, expected.hook_vel_bits, h.velocity
        ));
    }
    let got_yaw = fbits(h.yaw);
    if got_yaw != expected.hook_yaw_bits {
        errors.push(format!(
            "{label} t={}: hook_yaw expected {} got {got_yaw} ({})",
            expected.tick, expected.hook_yaw_bits, h.yaw
        ));
    }
}

pub(crate) fn spawn_test_hook(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    pos: Vec3,
    vel: Vec3,
    hook_seed: i64,
) {
    game.server.next_entity += 1;
    let id = game.server.next_entity;
    let mut hook = Hook::remote(id, pos, [0, 0, 0, id as i32]);
    hook.yaw = -90.0;
    hook.velocity = vel;
    hook.random = LegacyRandom::new(hook_seed);
    game.install_fishing_hook(hook);
}

#[allow(unused_imports)]
pub(crate) use crate::game::client_side::fishing_client::fishing_targets_tests::run_case;

#[allow(unused_imports)]
pub(crate) use crate::game::client_side::fishing_client::fishing_targets_tests::run_extra_case;

#[test]
#[ignore = "run by tools/test_projectile_targets_parity.py"]
fn witness_fishing_bobber_targets() {
    run_witness_fishing_bobber_targets();
}

pub(crate) fn run_witness_fishing_bobber_targets() {
    let witness = load_witness();
    let mut resources = None;
    let mut game = setup_game(&mut resources);
    let mut failures = Vec::new();
    for case in &witness.cases {
        let errs = run_case(&mut game, case, witness.hook_seed, witness.level_seed);
        if !errs.is_empty() {
            failures.push(errs.join("\n  "));
        }
    }
    for case in &witness.extra_cases {
        let errs = run_extra_case(&mut game, case, witness.hook_seed, witness.level_seed);
        if !errs.is_empty() {
            failures.push(errs.join("\n  "));
        }
    }
    assert!(
        failures.is_empty(),
        "{} fishing witness cases failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
