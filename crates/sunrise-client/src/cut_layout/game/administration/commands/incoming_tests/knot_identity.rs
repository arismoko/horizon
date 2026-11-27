//! The identity of a natural fence knot (`LeashFenceKnotEntity`): the tie
//! makes one real entity, with one id and one UUID, which commands and a
//! hosted server's tracker both name. Two leads on one fence share it, a
//! knot made again after its last lead went is a new entity, and the same
//! fence position in another level is another knot.
use std::sync::Arc;
use sunrise_core::position::ChunkPos;
use sunrise_level::chunk::Chunk;
use sunrise_sim::{mobs::leash::Holder, motion::PlayerMotion};
use {
    crate::game::*,
    sunrise_gameplay_common::game::{Dim, Options, Progress, Vec3},
};

pub(crate) const PIG: &str = "0000609a-0000-0000-0000-000000000001";
pub(crate) const OTHER_PIG: &str = "0000609a-0000-0000-0000-000000000003";
pub(crate) const AT_FENCE: &str =
    "@e[type=minecraft:leash_knot,x=8.5,y=201.375,z=0.5,distance=..0.001]";

pub(crate) fn game<'a>(
    resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
) -> sunrise_gameplay::game::ServerGame<'a> {
    let root = std::path::PathBuf::from(
        std::env::var_os("SUNRISE_TEST_RESOURCES")
            .expect("set SUNRISE_TEST_RESOURCES to prepared resources"),
    );
    let settings = Settings::default();
    let loaded = load_resources(
        sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role =
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
    let res = resources.insert(loaded.resources);
    let mut game = sunrise_gameplay::game::ServerGame::new(
        res,
        loaded.pack,
        options,
        settings.sim_settings(),
        None,
        crate::game::profile_resolver,
    )
    .unwrap();
    game.build_registries();
    game.set_allow_commands(true);
    for x in 0..2 {
        game.world
            .chunks
            .insert((x, 0), Arc::new(Chunk::empty(x, 0)));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(x, 0))
            .unwrap();
    }
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(12.5, 201.0, 2.5)).unwrap());
    for command in [
        "forceload add 0 0 31 15",
        "fill 0 200 0 31 200 15 stone",
        "tick freeze",
        "setblock 8 201 0 oak_fence",
    ] {
        assert!(game.run_command(command), "{command}");
    }
    game
}

pub(crate) fn summon_pig(game: &mut sunrise_gameplay::game::ServerGame<'_>, n: i32, x: f64) {
    let command = format!(
        "summon pig {x} 201 0.5 {{NoAI:1b,PersistenceRequired:1b,NoGravity:1b,leash:[I;8,201,0],UUID:[I;24730,0,0,{n}]}}"
    );
    assert!(game.run_command(&command), "{command}");
}

pub(crate) fn step(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    assert!(game.run_command("tick step 1"));
    game.tick_server(false).unwrap();
}

pub(crate) fn mob(game: &mut sunrise_gameplay::game::ServerGame<'_>, uuid: &str) -> usize {
    match game.target_with_uuid(sunrise_sim::save::uuid_from_text(uuid).unwrap()) {
        Some(super::Target::Mob(id)) => game.mobs.iter().position(|m| m.id == id).unwrap(),
        other => panic!("{other:?}"),
    }
}

/// The knots the level in place's selectors list, by entity id.
pub(crate) fn knots(game: &sunrise_gameplay::game::ServerGame<'_>) -> Vec<u32> {
    game.level_candidates(false)
        .into_iter()
        .filter_map(|t| match t {
            super::Target::Knot(id) => Some(id),
            _ => None,
        })
        .collect()
}

/// The UUID `/data` reads from the one knot of the level in place.
pub(crate) fn knot_uuid(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> [i32; 4] {
    let [id] = knots(game)[..] else {
        panic!("one knot: {:?}", knots(game));
    };
    let target = super::Target::Knot(id);
    match game
        .command_target_nbt(target)
        .and_then(|t| t.get("UUID").cloned())
    {
        Some(sunrise_core::nbt::Tag::IntArray(v)) => [v[0], v[1], v[2], v[3]],
        other => panic!("{other:?}"),
    }
}

#[test]
#[ignore = "daily"]
fn two_leads_on_one_fence_share_one_knot() {
    let mut resources = None;
    let mut game = game(&mut resources);
    summon_pig(&mut game, 1, 16.5);
    summon_pig(&mut game, 3, 4.5);
    step(&mut game);
    for uuid in [PIG, OTHER_PIG] {
        let i = mob(&mut game, uuid);
        assert_eq!(game.mobs[i].extra.leash, Some(Holder::Knot([8, 201, 0])));
    }
    assert_eq!(knots(&game).len(), 1);
    assert!(game.run_command(&format!("execute if entity {AT_FENCE}")));
    // One lead goes: the knot stays the same entity.
    let id = knots(&game)[0];
    let uuid = knot_uuid(&mut game);
    let other = mob(&mut game, OTHER_PIG);
    let other = game.mobs[other].id;
    game.break_leash(other, false, false);
    assert_eq!(knots(&game), [id]);
    assert_eq!(knot_uuid(&mut game), uuid);
}

#[test]
#[ignore = "daily"]
fn a_knot_tied_again_after_its_last_lead_is_a_new_entity() {
    let mut resources = None;
    let mut game = game(&mut resources);
    summon_pig(&mut game, 1, 13.5);
    step(&mut game);
    let first = knots(&game)[0];
    let first_uuid = knot_uuid(&mut game);
    let pig = mob(&mut game, PIG);
    let pig = game.mobs[pig].id;
    game.break_leash(pig, false, false);
    assert!(knots(&game).is_empty(), "no lead holds it: gone");
    assert!(!game.run_command(&format!("execute if entity {AT_FENCE}")));
    // The player leads the pig and ties it to the fence again.
    assert!(game.run_command("item replace entity @s weapon.mainhand with lead"));
    let i = mob(&mut game, PIG);
    assert!(game.interact_leash(i, "minecraft:lead"));
    assert!(game.use_knot([8, 201, 0], false));
    let i = mob(&mut game, PIG);
    assert_eq!(game.mobs[i].extra.leash, Some(Holder::Knot([8, 201, 0])));
    let [second] = knots(&game)[..] else {
        panic!("{:?}", knots(&game));
    };
    assert_ne!(second, first, "a new entity id");
    assert_ne!(knot_uuid(&mut game), first_uuid, "a new UUID");
}

#[test]
#[ignore = "daily"]
fn the_same_fence_position_in_another_level_is_another_knot() {
    let mut resources = None;
    let mut game = game(&mut resources);
    game.enter_command_level(Dim::Nether);
    assert_eq!(game.dim, Dim::Nether);
    for x in 0..2 {
        game.world
            .chunks
            .insert((x, 0), Arc::new(Chunk::empty(x, 0)));
        let _ = game.block_ticks.add_empty_chunk(ChunkPos::new(x, 0));
    }
    for command in [
        "forceload add 0 0 31 15",
        "fill 0 200 0 31 200 15 stone",
        "setblock 8 201 0 oak_fence",
        "summon pig 16.5 201 0.5 {NoAI:1b,PersistenceRequired:1b,NoGravity:1b,leash:[I;8,201,0],UUID:[I;24730,0,0,5]}",
    ] {
        assert!(game.run_command(command), "{command}");
    }
    game.enter_command_level(Dim::Overworld);
    assert_eq!(game.dim, Dim::Overworld);
    summon_pig(&mut game, 1, 16.5);
    step(&mut game);
    // The server tick leaves its first active level in place (the Nether, whose chunks are
    // forced), not the one the test was in: name the Overworld again before reading its knot.
    game.enter_command_level(Dim::Overworld);
    assert_eq!(game.dim, Dim::Overworld);
    let here = knots(&game);
    game.enter_command_level(Dim::Nether);
    let there = knots(&game);
    game.enter_command_level(Dim::Overworld);
    let ([here], [there]) = (&here[..], &there[..]) else {
        panic!("one knot in each level: {here:?} {there:?}");
    };
    let (here, there) = (*here, *there);
    assert_ne!(here, there, "two entities");
    assert_ne!(game.persist.uuids.get(&here), None);
    // Each level's selector finds its own knot, by its own UUID (a selector
    // with a position looks in the source's level only).
    let uuid_here = knot_uuid(&mut game);
    assert!(game.run_command(&format!(
        "execute in minecraft:the_nether if entity {AT_FENCE}"
    )));
    let nether = format!(
        "execute in minecraft:the_nether if entity @e[type=minecraft:leash_knot,x=8.5,y=201.375,z=0.5,distance=..0.001,nbt={{UUID:[I;{},{},{},{}]}}]",
        uuid_here[0], uuid_here[1], uuid_here[2], uuid_here[3]
    );
    assert!(
        !game.run_command(&nether),
        "the overworld knot is not the nether's"
    );
}

#[test]
#[ignore = "daily"]
fn commands_that_would_move_or_remove_a_natural_knot_fail() {
    let mut resources = None;
    let mut game = game(&mut resources);
    summon_pig(&mut game, 1, 16.5);
    step(&mut game);
    let id = knots(&game)[0];
    for command in [
        format!("kill {AT_FENCE}"),
        format!("tp {AT_FENCE} 20 201 4"),
        format!("teleport {AT_FENCE} ~ ~1 ~"),
        format!("rotate {AT_FENCE} 90 0"),
        format!("tag {AT_FENCE} add moved"),
        format!("spreadplayers 16 8 0 2 false {AT_FENCE}"),
        format!("data merge entity {AT_FENCE} {{Invulnerable:1b}}"),
    ] {
        assert!(!game.run_command(&command), "{command}");
    }
    assert_eq!(knots(&game), [id], "the knot is still there");
    let i = mob(&mut game, PIG);
    assert_eq!(game.mobs[i].extra.leash, Some(Holder::Knot([8, 201, 0])));
    let at = sunrise_sim::mobs::leash::knot_state([8, 201, 0]).position;
    assert_eq!(
        game.target_position(super::Target::Knot(id)),
        Some([at.x, at.y, at.z])
    );
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;
