#[cfg(test)]
use sunrise_command::args::ItemInput;
#[cfg(test)]
use sunrise_command::args::Selector;
#[cfg(test)]
use sunrise_core::geometry::Vec3;
#[cfg(test)]
use sunrise_gameplay_common::game::commands_common::Target;
#[cfg(test)]
use sunrise_items::items::ItemStack;
#[cfg(test)]
pub(crate) mod enchant_tests;

#[cfg(test)]
pub(crate) mod clone_writes_tests;

#[cfg(test)]
pub(crate) mod air_filter_tests;
#[cfg(test)]
pub(crate) mod teleport_pitch_tests;

#[cfg(test)]
#[path = "summon_item_orb_tests.rs"]
pub(crate) mod summon_item_orb_tests;

#[cfg(test)]
#[path = "summon_bolt_tests.rs"]
pub(crate) mod summon_bolt_tests;

#[cfg(test)]
pub(crate) mod peaceful_tests {

    use sunrise_gameplay_common::game::server_side::commands::peaceful_tests::TRADER;

    use sunrise_command::{Arg, Context};

    use crate::game::execution_role::ExecutionRole;

    use sunrise_command::ParsedArg;

    pub(crate) use sunrise_gameplay::game::server_side::creatures_server::peaceful_fixture::with_types;
    #[test]
    #[ignore = "daily"]
    fn summon_checks_the_mod_type_instead_of_its_template() {
        with_types(ExecutionRole::IntegratedServer, |game| {
            let context = |id: &str, nbt: Option<&str>| {
                let arg = |name: &str, value| ParsedArg {
                    name: name.into(),
                    value,
                    range: (0, 0),
                };
                let mut args = vec![arg("entity", Arg::Resource(id.into()))];
                if let Some(nbt) = nbt {
                    args.push(arg("nbt", Arg::Str(nbt.into())));
                }
                Context {
                    root: 0,
                    nodes: vec![],
                    args,
                    command: None,
                    executor: None,
                    range: (0, 0),
                    child: None,
                    literals: vec![],
                }
            };
            for id in [TRADER, "peaceful_test:inherited_zombie", "minecraft:zombie"] {
                for nbt in [None, Some("{NoAI:1b}")] {
                    let error = game.cmd_summon(&context(id, nbt)).unwrap_err();
                    assert_eq!(
                        error.key, "commands.summon.failed.peaceful",
                        "{id}, NBT {nbt:?}"
                    );
                }
            }
            assert!(game.mobs.is_empty());
            assert_eq!(
                game.cmd_summon(&context("peaceful_test:zombie", Some("{UUID:[I;1,2,3,4]}"))),
                Ok(1)
            );
            assert!(
                game.mobs
                    .iter()
                    .any(|mob| mob.kind.id == "peaceful_test:zombie")
            );
            for uuid in ["", ",UUID:[I;1,2,3,4]"] {
                // The already loaded zombie owns the second rider's UUID.
                // The skipped rider's own permitted rider must not spawn either.
                let nbt = format!(
                    "{{Passengers:[{{id:\"{TRADER}\"{uuid},Passengers:[{{id:\"minecraft:pig\"}}]}}]}}"
                );
                let before = game.mobs.len();
                assert_eq!(
                    game.cmd_summon(&context("minecraft:cow", Some(&nbt))),
                    Ok(1)
                );
                assert_eq!(game.mobs.len(), before + 1);
                let cow = &game.mobs[before];
                assert_eq!(cow.kind.id, "minecraft:cow");
                assert!(cow.extra.rider.is_none());
            }
        });
    }
}

#[cfg(test)]
#[path = "teleport_mob_tests.rs"]
pub(crate) mod teleport_mob_tests;

#[cfg(test)]
#[path = "teleport_item_tests.rs"]
pub(crate) mod teleport_item_tests;

#[cfg(test)]
#[path = "teleport_level_tests.rs"]
pub(crate) mod teleport_level_tests;

#[cfg(test)]
#[path = "teleport_player_rider_tests.rs"]
pub(crate) mod teleport_player_rider_tests;

#[cfg(test)]
pub(crate) mod selector_bounds_tests {
    use crate::game::server_side::commands::*;
    use sunrise_core::geometry::Vec3;
    use sunrise_sim::{entities::ArmorStand, mobs::Mob};
    use {
        crate::game::load_resources, crate::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
    };

    #[test]
    #[ignore = "daily"]
    fn actual_selector_uses_body_overlap_but_distance_stays_at_the_origin() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists() {
            eprintln!("Prepared resources unavailable; Game selector coverage did not run");
            return;
        }
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
        let mut game = sunrise_gameplay::game::ServerGame::new(
            &loaded.resources,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        game.build_registries();
        let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
        game.mobs
            .push(Mob::new(30, kind, Vec3::new(10.5, 220.0, 10.5), 0.0));
        let target = Target::Mob(30);
        let mut selector = Selector {
            kind: 'e',
            include_entities: true,
            limit: usize::MAX,
            delta: [Some(0.0); 3],
            ..Default::default()
        };
        let origin = [9.5, 220.0, 10.5];
        assert_eq!(
            game.select_in_level(vec![target], &selector, origin),
            vec![(target, 1.0)]
        );
        selector.distance = Some((None, Some(0.9)));
        assert!(
            game.select_in_level(vec![target], &selector, origin)
                .is_empty()
        );
        selector.distance = None;
        assert!(
            game.select_in_level(vec![target], &selector, [11.0, 220.0, 10.5])
                .is_empty()
        );
        selector.delta = [Some(-1.0), None, None];
        assert_eq!(
            game.select_in_level(vec![target], &selector, [11.5, 220.0, 10.5])
                .len(),
            1
        );
        game.armor_stands
            .push(ArmorStand::new(31, Vec3::new(30.5, 220.0, 10.5), 0.0));
        selector.delta = [Some(0.0); 3];
        for (x, count) in [(29.5, 1), (29.25, 0), (30.75, 0), (29.250000000000004, 1)] {
            assert_eq!(
                game.select_in_level(vec![Target::Stand(31)], &selector, [x, 220.0, 10.5])
                    .len(),
                count
            );
        }
    }
}

#[cfg(test)]
#[path = "local_coordinate_command_tests.rs"]
pub(crate) mod local_coordinate_command_tests;

#[cfg(test)]
pub(crate) mod effective_presence_tests {
    use crate::game::server_side::commands::*;
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    use sunrise_gameplay_common::game::tables::GameItems;
    #[test]
    #[ignore = "daily"]
    fn registered_presence_uses_actual_catalog_and_stack_patch() {
        with_game(|game| {
            game.build_registries();
            game.set_allow_commands(true);
            let key = "minecraft:jukebox_playable";
            assert!(
                game.items()
                    .get("minecraft:music_disc_cat")
                    .unwrap()
                    .components
                    .iter()
                    .any(|k| k == key)
            );
            assert!(
                !game
                    .items()
                    .get("minecraft:stone")
                    .unwrap()
                    .components
                    .iter()
                    .any(|k| k == key)
            );
            // Supply the synthetic chunk before forceload's synchronous load barrier.
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            assert!(game.run_command("forceload add 0 0 15 15"));
            assert!(game.world.loaded(4, 4));
            assert!(game.run_command("setblock 4 199 4 stone"));
            assert!(game.run_command("setblock 4 200 4 chest{Items:[{Slot:0b,id:\"minecraft:music_disc_cat\",count:1},{Slot:1b,id:\"minecraft:stone\",count:1},{Slot:2b,id:\"minecraft:stone\",count:1,components:{\"minecraft:jukebox_playable\":\"minecraft:cat\",\"minecraft:custom_data\":{h193:1}}},{Slot:3b,id:\"minecraft:music_disc_cat\",count:1,components:{\"!minecraft:jukebox_playable\":{},\"minecraft:custom_data\":{h193:1}}}]}"));
            assert!(game.run_command("scoreboard objectives add h193 dummy"));
            let clock = game.world.game_time;
            let sunrise_level::containers::BlockEntity::Chest(stacks) =
                game.world.block_entities.get(&(4, 200, 4)).unwrap()
            else {
                panic!("actual chest missing")
            };
            let stacks = stacks.clone();
            assert_eq!(stacks.len(), 27);
            assert!(
                stacks[0].as_ref().unwrap().extra.is_none(),
                "plain modeled stack has no patch"
            );
            assert!(
                stacks[2]
                    .as_ref()
                    .unwrap()
                    .extra
                    .as_ref()
                    .unwrap()
                    .other(key)
                    .is_some()
            );
            assert!(
                stacks[3]
                    .as_ref()
                    .unwrap()
                    .extra
                    .as_ref()
                    .unwrap()
                    .other("!minecraft:jukebox_playable")
                    .is_some()
            );
            let saved = game.block_entity_nbt([4, 200, 4]).unwrap();
            let mut failures = Vec::new();
            for (slot, present) in [true, false, true, false].into_iter().enumerate() {
                let stack = stacks[slot].as_ref().unwrap();
                assert_eq!(stack.count, 1);
                for neg in [false, true] {
                    let expected = present != neg;
                    let term = format!("{}minecraft:jukebox_playable", if neg { "!" } else { "" });
                    let predicate = ItemInput {
                        id: String::new(),
                        tag: false,
                        any: true,
                        components: Some(format!("[{term}]")),
                    };
                    if game.stack_matches(&predicate, stack) != expected {
                        failures.push(format!("direct slot{slot} {term}"));
                    }
                    for sign in ["if", "unless"] {
                        for (name, n) in [("r", 91), ("s", 17)] {
                            assert!(
                                game.run_command(&format!(
                                    "scoreboard players set #{name} h193 {n}"
                                ))
                            );
                        }
                        let accepted = game.run_command(&format!("execute store result score #r h193 store success score #s h193 {sign} items block 4 200 4 container.{slot} *[{term}]"));
                        let yes = if sign == "if" { expected } else { !expected };
                        let score = |name| game.score.board.score(name, "h193").unwrap().value;
                        if (accepted, score("#r"), score("#s"))
                            != (yes, i32::from(yes), i32::from(yes))
                        {
                            failures.push(format!("command slot{slot} {sign} {term}"));
                        }
                    }
                }
            }
            let plain = ItemStack::new("minecraft:music_disc_cat", 1);
            assert!(plain.extra.is_none());
            let p = ItemInput {
                id: String::new(),
                tag: false,
                any: true,
                components: Some("[minecraft:jukebox_playable]".into()),
            };
            if !game.stack_matches(&p, &plain) {
                failures.push("no-raw modeled prototype".into());
            }
            assert_eq!(
                game.block_entity_nbt([4, 200, 4]).unwrap(),
                saved,
                "read-only saved inventory"
            );
            assert_eq!(game.world.game_time, clock, "zero actor ticks");
            assert!(game.world.loaded(4, 4));
            assert!(
                failures.is_empty(),
                "actual effective component presence mismatches: {failures:?}"
            );
        });
    }
}

#[cfg(test)]
pub(crate) mod fill_note_instrument_tests {
    use crate::game::server_side::commands::*;
    use std::sync::Arc;
    use sunrise_core::position::ChunkPos;
    use sunrise_level::chunk::Chunk;
    use sunrise_sim::motion::PlayerMotion;
    use {
        crate::game::load_resources, crate::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
    };
    fn with_game(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
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
        options.game_dir = Some(
            std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap())
                .join(format!("h164-{:?}", std::thread::current().id())),
        );
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = sunrise_gameplay::game::ServerGame::new(
            &loaded.resources,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        game.build_registries();
        game.set_allow_commands(true);

        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap());
        game.creative = true;
        assert!(game.run_command("forceload add 0 0"));
        assert!(game.run_command("tick freeze"));
        assert!(game.run_command("fill 0 198 0 15 199 15 stone"));
        test(&mut game);
    }

    fn instrument(game: &sunrise_gameplay::game::ServerGame<'_>, x: i32, z: i32) -> String {
        let state = game
            .pack
            .assets
            .registry
            .state(game.world.block(x, 200, z))
            .unwrap();
        assert_eq!(state.name, "minecraft:note_block");
        state.properties["instrument"].clone()
    }

    #[test]
    #[ignore = "daily"]
    fn ordinary_fill_normalizes_a_note_block_against_real_stone() {
        with_game(|game| {
            let clock = game.world.game_time;
            assert!(
                game.world.chunks.contains_key(&(0, 0)),
                "actual fill chunk is resident"
            );
            assert!(game.run_command("execute if block 4 199 4 stone"));
            assert!(game.run_command("execute if block 4 200 4 air"));
            assert!(game.run_command("setblock 8 200 4 note_block"));
            assert_eq!(
                instrument(game, 8, 4),
                "basedrum",
                "positive setblock counterpart"
            );
            assert!(game.run_command("fill 4 200 4 4 200 4 note_block"));
            assert_eq!(
                instrument(game, 4, 4),
                "basedrum",
                "actual registered fill must normalize instrument"
            );
            assert!(game.run_command("execute if block 4 200 4 note_block[note=0,powered=false]"));
            assert_eq!(game.world.game_time, clock);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn an_existing_default_state_does_not_skip_note_normalization() {
        with_game(|game| {
            let clock = game.world.game_time;
            assert!(game.run_command("setblock 4 200 4 note_block strict"));
            assert_eq!(instrument(game, 4, 4), "harp");
            assert!(
                game.run_command("fill 4 200 4 4 200 4 note_block"),
                "valid fill changes existing harp to basedrum"
            );
            assert_eq!(instrument(game, 4, 4), "basedrum");
            assert_eq!(game.world.game_time, clock);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn strict_explicit_and_filtered_out_inputs_keep_their_existing_contracts() {
        with_game(|game| {
            let clock = game.world.game_time;
            assert!(game.run_command("fill 4 200 4 4 200 4 note_block strict"));
            assert_eq!(instrument(game, 4, 4), "harp");
            assert!(game.run_command("fill 8 200 4 8 200 4 note_block[instrument=banjo,note=7]"));
            assert_eq!(instrument(game, 8, 4), "banjo");
            assert!(game.run_command("execute if block 8 200 4 note_block[note=7,powered=false]"));
            assert!(game.run_command("fill 12 200 4 12 200 4 note_block[instrument=harp,note=7]"));
            assert_eq!(instrument(game, 12, 4), "harp");
            assert!(!game.run_command("fill 4 200 4 4 200 4 note_block[instrument=banjo] keep"));
            assert!(
                !game
                    .run_command("fill 4 200 4 4 200 4 note_block[instrument=banjo] replace glass")
            );
            assert_eq!(instrument(game, 4, 4), "harp");
            assert_eq!(game.world.game_time, clock);
        });
    }
}

#[cfg(test)]
pub(crate) mod item_damage_range_tests {
    use crate::game::server_side::commands::*;
    use sunrise_core::position::BlockPos;
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    use sunrise_gameplay_common::game::tables::GameItems;

    #[test]
    #[ignore = "daily"]
    fn actual_chest_damage_range_uses_live_damage_and_effective_presence() {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            game.build_registries();
            game.set_allow_commands(true);
            game.block_ticks
                .add_empty_chunk(BlockPos::new(0, 0, 0).chunk())
                .unwrap();
            assert!(game.run_command("forceload add 0 0"));
            assert!(game.run_command("setblock 4 199 4 stone"));
            assert!(game.run_command(r#"setblock 4 200 4 chest{Items:[{Slot:0b,id:"minecraft:diamond_sword",count:1,components:{"minecraft:custom_data":{marker:195}}},{Slot:1b,id:"minecraft:diamond_sword",count:1,components:{"minecraft:damage":3,"minecraft:custom_data":{marker:195}}},{Slot:2b,id:"minecraft:diamond_sword",count:1,components:{"minecraft:damage":4,"minecraft:custom_data":{marker:195}}},{Slot:3b,id:"minecraft:diamond_sword",count:1,components:{"minecraft:damage":5,"minecraft:custom_data":{marker:195}}},{Slot:4b,id:"minecraft:stone",count:1,components:{"minecraft:custom_data":{marker:195}}},{Slot:5b,id:"minecraft:diamond_sword",count:1,components:{"!minecraft:damage":{},"minecraft:custom_data":{marker:195}}}]}"#));
            assert!(game.run_command("scoreboard objectives add h195 dummy"));
            assert!(game.world.loaded(4, 4));
            assert!(game.run_command("execute if block 4 199 4 stone"));
            let clock = game.world.game_time;
            let stacks = match game.world.block_entities.get(&(4, 200, 4)).unwrap() {
                sunrise_level::containers::BlockEntity::Chest(slots) => slots.clone(),
                _ => panic!("actual loaded chest missing"),
            };
            for (slot, damage) in [0, 3, 4, 5, 0, 0].into_iter().enumerate() {
                let stack = stacks[slot].as_ref().unwrap();
                assert_eq!(stack.damage, damage);
                assert_eq!(stack.count, 1);
                assert_eq!(
                    &*stack.item,
                    if slot == 4 {
                        "minecraft:stone"
                    } else {
                        "minecraft:diamond_sword"
                    }
                );
                assert_eq!(
                    stack.custom_data().unwrap().get(&"marker".into()),
                    Some(&sunrise_core::nbt::Tag::Int(195))
                );
                assert!(game.run_command(&format!(
                    "execute if items block 4 200 4 container.{slot} {}",
                    stack.item
                )));
            }
            assert!(stacks[6].is_none());
            assert!(
                game.items()
                    .get("minecraft:diamond_sword")
                    .unwrap()
                    .components
                    .iter()
                    .any(|c| c == "minecraft:damage")
            );
            assert!(
                !game
                    .items()
                    .get("minecraft:stone")
                    .unwrap()
                    .components
                    .iter()
                    .any(|c| c == "minecraft:damage")
            );
            let removed =
                sunrise_items::item_data::component_snapshot(stacks[5].as_ref().unwrap()).unwrap();
            assert!(
                removed.get("!minecraft:damage").is_some(),
                "actual removal patch must survive load before evaluating it"
            );
            assert!(game.run_command(
                "execute unless data block 4 200 4 Items[{Slot:0b}].components.\"minecraft:damage\""
            ));
            assert!(game.run_command(
                "execute if data block 4 200 4 Items[{Slot:2b,components:{\"minecraft:damage\":4}}]"
            ));
            assert!(game.run_command(
                "execute if data block 4 200 4 Items[{Slot:5b}].components.\"!minecraft:damage\""
            ));
            let mut cases = Vec::new();
            for (slot, damage) in [(1, 3), (2, 4), (3, 5)] {
                for (shape, expected) in [
                    ("4", damage == 4),
                    ("{min:4}", damage >= 4),
                    ("{max:4}", damage <= 4),
                    ("{min:3,max:4}", (3..=4).contains(&damage)),
                ] {
                    cases.push((
                        slot,
                        format!("minecraft:damage~{{damage:{shape}}}"),
                        expected,
                    ));
                }
            }
            cases.extend([
                (0, "damage~{damage:0}".into(), true),
                (0, "damage~{damage:{min:1}}".into(), false),
                (4, "damage~{damage:0}".into(), false),
                (5, "damage~{damage:0}".into(), false),
                (2, "!damage~{damage:4}".into(), false),
                (1, "!damage~{damage:4}".into(), true),
                (1, "damage=3".into(), true),
                (1, "damage=4".into(), false),
                (1, "count~1".into(), true),
                (1, "custom_data~{marker:195}".into(), true),
            ]);
            let mut mismatches = Vec::new();
            for (slot, term, expected) in cases {
                let stack = stacks[slot].as_ref().unwrap();
                let predicate = ItemInput {
                    id: stack.item.to_string(),
                    tag: false,
                    any: false,
                    components: Some(format!("[{term}]")),
                };
                let actual = game.stack_matches(&predicate, stack);
                if actual != expected {
                    mismatches.push(format!(
                        "direct slot{slot} {term}: {actual}, expected{expected}"
                    ));
                }
                assert!(game.run_command("scoreboard players set #r h195 91"));
                assert!(game.run_command("scoreboard players set #s h195 17"));
                let command = format!(
                    "execute store result score #r h195 store success score #s h195 if items block 4 200 4 container.{slot} {}[{term}]",
                    stack.item
                );
                game.run_command(&command);
                let r = game.score.board.score("#r", "h195").map(|s| s.value);
                let s = game.score.board.score("#s", "h195").map(|s| s.value);
                if (r, s) != (Some(i32::from(expected)), Some(i32::from(expected))) {
                    mismatches.push(format!("command {term}: {r:?}/{s:?}, expected{expected}"));
                }
                assert!(game.run_command("scoreboard players set #branch h195 23"));
                game.run_command(&format!("execute unless items block 4 200 4 container.{slot} {}[{term}] run scoreboard players set #branch h195 7",stack.item));
                let branch = game.score.board.score("#branch", "h195").map(|s| s.value);
                if branch != Some(if expected { 23 } else { 7 }) {
                    mismatches.push(format!("unless {term}: {branch:?}"));
                }
            }
            assert!(!game.run_command(
                "execute if items block 4 200 4 container.4 diamond_sword[damage~{damage:4}]"
            ));
            assert!(!game.run_command(
                "execute if items block 4 200 4 container.6 diamond_sword[damage~{damage:4}]"
            ));
            let actual = match game.world.block_entities.get(&(4, 200, 4)).unwrap() {
                sunrise_level::containers::BlockEntity::Chest(slots) => slots,
                _ => panic!("source chest changed"),
            };
            assert_eq!(
                actual.as_slice(),
                stacks.as_slice(),
                "queries must leave the authoritative loaded stacks unchanged"
            );
            assert_eq!(game.world.game_time, clock);
            assert!(
                mismatches.is_empty(),
                "damage-only command predicate mismatches: {mismatches:#?}"
            );
        });
    }
}

#[cfg(test)]
#[path = "vehicle_kill_tests.rs"]
pub(crate) mod vehicle_kill_tests;

#[cfg(test)]
pub(crate) mod global_selector_order_tests {
    use sunrise_command::args::Sort;
    use sunrise_core::nbt::Tag;
    use sunrise_core::position::ChunkPos;
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2;
    use sunrise_sim::join_seq::Joined;
    use {sunrise_command::args::Selector, sunrise_gameplay_common::game::commands_common::Target};

    #[test]
    #[ignore = "daily"]
    fn actual_registered_global_selector_keeps_interleaved_insertion_and_limits() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            game.build_registries();
            game.set_allow_commands(true);
            for x in 0..2 {
                for z in 0..2 {
                    game.world.chunks.insert(
                        (x, z),
                        std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(x, z)),
                    );
                    game.block_ticks
                        .add_empty_chunk(ChunkPos::new(x, z))
                        .unwrap();
                }
            }
            assert!(game.run_command("fill 0 199 0 31 199 31 stone"));
            assert!(game.run_command("scoreboard objectives add h268 dummy"));
            let clock = game.world.game_time;
            let actors = [
                ("pig", 41, [17.5, 200.0, 16.25]),
                ("armor_stand", 7, [15.75, 200.0, 15.5]),
                ("item", 93, [15.5, 200.0, 17.0]),
                ("pig", 2, [17.0, 200.0, 15.75]),
                ("armor_stand", 68, [18.5, 200.0, 16.5]),
                ("item", 11, [14.0, 200.0, 16.75]),
            ];
            let mut targets = Vec::new();
            let mut last_join = 0;
            for (i, (kind, uuid, pos)) in actors.iter().enumerate() {
                let extra = match *kind {
                    "pig" => ",NoAI:1b,Health:10f,PersistenceRequired:1b",
                    "armor_stand" => ",Health:20f",
                    _ => ",Item:{id:\"minecraft:stone\",count:1},PickupDelay:32767s",
                };
                assert!(game.run_command(&format!(
                "summon {kind} {:?} {:?} {:?} {{UUID:[I;268,0,0,{uuid}],Tags:[\"h268\"],NoGravity:1b,Motion:[0d,0d,0d]{extra}}}",
                pos[0], pos[1], pos[2]
            )));
                let target = game
                    .target_by_uuid((268u128 << 96) | *uuid as u128)
                    .unwrap();
                assert_eq!(game.target_type(target), format!("minecraft:{kind}"));
                assert_eq!(game.target_position(target), Some(*pos));
                assert!(game.world.loaded(pos[0] as i32, pos[2] as i32));
                let join = match target {
                    Target::Mob(id) => game.mobs.iter().find(|m| m.id == id).unwrap().join_seq(),
                    Target::Stand(id) => game
                        .armor_stands
                        .iter()
                        .find(|m| m.id == id)
                        .unwrap()
                        .join_seq(),
                    Target::Item(id) => {
                        game.dropped.iter().find(|m| m.id == id).unwrap().join_seq()
                    }
                    _ => panic!("actual admitted family"),
                };
                assert!(
                    join.is_joined() && join.get() > last_join,
                    "actual registered creation chronology {i}"
                );
                last_join = join.get();
                targets.push(target);
                assert!(game.run_command("scoreboard players set #r h268 91"));
                assert!(game.run_command(
                    "execute store result score #r h268 run execute if entity @e[tag=h268]"
                ));
                assert_eq!(
                    game.score.board.score("#r", "h268").unwrap().value,
                    i as i32 + 1
                );
            }
            let selector = Selector {
                kind: 'e',
                include_entities: true,
                limit: usize::MAX,
                tags: vec![("h268".into(), false)],
                ..Default::default()
            };
            // Existing sorted paths remain controls, with no equal-distance ties.
            let mut nearest = selector.clone();
            nearest.sort = Sort::Nearest;
            nearest.pos = [Some(16.0), Some(200.0), Some(16.0)];
            assert_eq!(
                game.select(&nearest),
                [1, 3, 2, 0, 5, 4].map(|i| targets[i])
            );
            nearest.sort = Sort::Furthest;
            assert_eq!(
                game.select(&nearest),
                [4, 5, 0, 2, 3, 1].map(|i| targets[i])
            );
            let mut spatial = selector.clone();
            spatial.distance = Some((None, Some(3.0)));
            spatial.pos = [Some(16.0), Some(200.0), Some(16.0)];
            let old_spatial = game.select(&spatial);
            let mut failures = Vec::new();
            for (name, options, expected) in [
                ("all", "sort=arbitrary", vec![41, 7, 93, 2, 68, 11]),
                ("two", "sort=arbitrary,limit=2", vec![41, 7]),
                (
                    "filtered",
                    "type=!minecraft:armor_stand,sort=arbitrary,limit=3",
                    vec![41, 93, 2],
                ),
            ] {
                assert!(game.run_command(&format!(
                    "data modify storage h268:proof {name} set value []"
                )));
                assert!(game.run_command("scoreboard players set #r h268 91"));
                assert!(game.run_command("scoreboard players set #s h268 17"));
                assert!(game.run_command(&format!("execute positioned 16.0 200.0 16.0 store result score #r h268 store success score #s h268 as @e[tag=h268,{options}] run data modify storage h268:proof {name} append from entity @s UUID")));
                assert_eq!(game.score.board.score("#r", "h268").unwrap().value, 1);
                assert_eq!(game.score.board.score("#s", "h268").unwrap().value, 1);
                let actual = game.score.storage.get("h268:proof").get(name).cloned();
                let wanted = Some(Tag::List(
                    expected
                        .iter()
                        .map(|v| Tag::IntArray(vec![268, 0, 0, *v]))
                        .collect(),
                ));
                if actual != wanted {
                    failures.push(format!("{name}: {actual:?} != {wanted:?}"));
                }
            }
            assert_eq!(
                game.select(&spatial),
                old_spatial,
                "unchanged spatial path within the scene"
            );
            assert_eq!(game.world.game_time, clock);
            for (target, (_, _, pos)) in targets.iter().zip(actors.iter()) {
                assert_eq!(game.target_position(*target), Some(*pos));
            }
            assert!(
                failures.is_empty(),
                "actual registered selector order/limits: {failures:#?}"
            );
        });
    }
}

#[cfg(test)]
#[path = "incoming_tests/knot_identity.rs"]
pub(crate) mod knot_identity_tests;

#[cfg(test)]
pub(crate) mod mob_display_name_tests;

#[cfg(test)]
#[path = "item_count_predicate_tests.rs"]
pub(crate) mod item_count_predicate_tests;

#[cfg(test)]
use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

pub(crate) mod world_cmds;

pub(crate) mod terrain_cmds;

pub(crate) mod raid;

pub(crate) mod place;

pub(crate) mod misc;

pub(crate) mod items;

pub(crate) mod execute;

pub(crate) mod data;
