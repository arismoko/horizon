//! Entity.kill, BlockAttachedEntity.kill, ArmorStand.kill and ItemEntity.dampensVibrations.
use crate::game::load_resources;
use crate::settings::Settings;
use std::sync::Arc;
use sunrise_core::{geometry::Vec3, position::ChunkPos};
use sunrise_game_data::who::EventSource;
use sunrise_gameplay_common::game::entity_common::EntityCommonGameExt1 as _;
use sunrise_gameplay_common::game::execution_role;
use sunrise_gameplay_common::game::{Options, Progress};
use sunrise_level::{chunk::Chunk, containers::BlockEntity};
use sunrise_sim::mobs::{Mob, Who, warden};

fn deaths() -> Vec<(&'static str, [f64; 3], Option<Who>)> {
    sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS.with(|events| {
        events
            .borrow_mut()
            .take()
            .unwrap()
            .into_iter()
            .filter(|(event, _, _)| *event == "minecraft:entity_die")
            .collect()
    })
}
fn capture() {
    sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS
        .with(|events| *events.borrow_mut() = Some(Vec::new()));
}

fn with_game(f: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
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
        execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
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
    game.run_command("forceload add 0 0");
    game.run_command("fill 0 199 0 15 199 15 stone");
    game.run_command("gamerule entity_drops false");
    assert!(
        !game
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::ENTITY_DROPS)
    );
    f(&mut game);
}

#[test]
#[ignore = "daily"]
fn receiver_kill_stages_orb_removal_until_commit_and_kills_a_resident_knot_once() {
    use sunrise_gameplay::game::native_entity_refs::tests::{fixture_resident_column, with_game};
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
    use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

    with_game(|game| {
        fixture_resident_column(&mut game.world, (0, 0));
        let at = [4.25, 70.0, 4.75];
        game.summon_with_nbt("minecraft:experience_orb", at, "{Value:1s}")
            .unwrap();
        let orb = game.entity(game.orbs[0].id).unwrap();

        for commit in [false, true] {
            capture();
            game.storage_scope(|game, _| {
                assert_eq!(orb.kill(game), Ok(true));
                assert!(game.entity(orb.id()).is_none());
                assert!(orb.position(game).is_none());
                assert_eq!(orb.kill(game), Err(mod_api::LevelWriteError::Stale));
                assert!(deaths().is_empty());
                capture();
                ((), commit)
            });
            assert_eq!(game.entity(orb.id()), (!commit).then_some(orb));
            let expected = if commit { vec![at] } else { vec![] };
            assert_eq!(
                deaths()
                    .into_iter()
                    .map(|(_, pos, _)| pos)
                    .collect::<Vec<_>>(),
                expected
            );
        }

        let fence = [8, 70, 8];
        game.tie_knot(fence);
        let knot = game.entity(game.server.knot_ids[&fence]).unwrap();
        let at = knot.position(game).unwrap();
        capture();
        assert_eq!(knot.kill(game), Ok(true));
        assert_eq!(knot.kill(game), Err(mod_api::LevelWriteError::Stale));
        assert_eq!(game.natural_knot(knot.id()), None);
        assert_eq!(
            deaths()
                .into_iter()
                .map(|(_, pos, _)| pos)
                .collect::<Vec<_>>(),
            [at]
        );
    });
}

#[test]
#[ignore = "daily"]
fn entity_die_commands() {
    with_game(|game| {
        let at = [4.25, 200.0, 4.75];
        let kind = game
            .res
            .mob_types()
            .get("minecraft:warden")
            .unwrap()
            .clone();
        game.mobs
            .push(Mob::new(700, kind, Vec3::new(7.5, 200.0, 4.5), 0.0));
        for (kind, nbt, heard) in [
            ("item", "{Item:{id:\"minecraft:stone\",count:1}}", true),
            (
                "item",
                "{Item:{id:\"minecraft:white_wool\",count:1}}",
                false,
            ),
            ("item", "{Item:{id:\"minecraft:red_wool\",count:1}}", false),
            ("cushion", "{color:\"red\"}", true),
            ("end_crystal", "{Invulnerable:1b}", true),
            ("marker", "{}", true),
            ("armor_stand", "{}", true),
        ] {
            let target = game
                .summon_with_nbt(&format!("minecraft:{kind}"), at, nbt)
                .unwrap()
                .unwrap_or_else(|| panic!("summon did not return {kind} target"));
            assert_eq!(game.target_position(target), Some(at));
            game.run_command("setblock 1 200 4 air strict");
            game.run_command("setblock 1 200 4 sculk_sensor strict");
            game.sculk_prepare();
            game.sculk_sync_index();
            warden::register(&mut game.mobs[0]);
            // The killed entity is the event's source, except an end crystal, whose kill posts
            // without one.
            let source = (kind != "end_crystal")
                .then(|| game.network_entity_id(target))
                .flatten()
                .map(|id| EventSource::Entity(id as u32));
            capture();
            game.run_command(&format!("kill @e[type=minecraft:{kind}]"));
            assert_eq!(game.target_position(target), None, "{kind} removed");
            assert_eq!(deaths(), vec![("minecraft:entity_die", at, None)], "{kind}");
            let Some(BlockEntity::Sculk(sensor)) = game.world.block_entities.get(&(1, 200, 4))
            else {
                panic!("sensor must exist");
            };
            assert_eq!(
                sensor.vibration.selector.current.is_some(),
                heard,
                "{kind} {nbt}"
            );
            if let Some((info, _)) = sensor.vibration.selector.current {
                assert_eq!(info.event, "minecraft:entity_die");
                assert_eq!(info.pos, at);
                assert_eq!(info.source, source, "{kind} source");
            }
            let vibration = &game.mobs[0].extra.warden.as_ref().unwrap().vibration;
            assert_eq!(
                vibration.selector.current.is_some(),
                heard && kind != "armor_stand",
                "warden accepts {kind} source"
            );
            assert!(
                game.dropped.is_empty(),
                "/kill must not drop the cushion or crystal"
            );
        }
        game.run_command("summon end_crystal 4.25 200 4.75");
        capture();
        game.run_command("damage @e[type=end_crystal,limit=1] 1 minecraft:explosion");
        assert!(game.crystals.is_empty());
        assert!(
            deaths().is_empty(),
            "EndCrystal.hurtServer removes without Entity.kill"
        );
    });
}

#[test]
#[ignore = "daily"]
fn entity_die_explosions() {
    with_game(|game| {
        let at = [4.25, 200.0, 4.75];
        // A real blast must attribute hanging/stand deaths to its causing mob,
        // while vehicles use Entity.kill's self source. No cause stays self-sourced.
        let kind = game
            .res
            .mob_types()
            .get("minecraft:creeper")
            .unwrap()
            .clone();
        game.mobs
            .push(Mob::new(700, kind, Vec3::new(10.0, 200.0, 10.0), 0.0));
        for source in [None, Some(700)] {
            let expected_source = source.map(Who::Mob);
            // The loaded entity exists even on main, where it has no command target.
            game.summon_with_nbt("minecraft:cushion", at, "{}").unwrap();
            assert_eq!(game.cushions.len(), 1);
            let stand_at = [5.25, 200.0, 4.75];
            let stand = game
                .summon_with_nbt("minecraft:armor_stand", stand_at, "{}")
                .unwrap()
                .unwrap();
            let painting = game
                .summon_with_nbt(
                    "minecraft:painting",
                    [4.5, 200.5, 4.5],
                    "{variant:\"minecraft:kebab\",facing:0b}",
                )
                .unwrap()
                .unwrap();
            let frame = game
                .summon_with_nbt("minecraft:item_frame", [4.5, 200.5, 4.5], "{Facing:3b}")
                .unwrap()
                .unwrap();
            let boat = game
                .summon_with_nbt("minecraft:oak_boat", [3.5, 200.0, 4.5], "{}")
                .unwrap()
                .unwrap();
            game.summon_with_nbt(
                "minecraft:item",
                at,
                r#"{Item:{id:"minecraft:stone",count:1}}"#,
            )
            .unwrap();
            game.summon_with_nbt("minecraft:experience_orb", at, "{Value:1s}")
                .unwrap();
            game.server.combat.0.explosion_owner = source;
            capture();
            game.server_explode(
                [4.5, 200.5, 4.5],
                2.0,
                sunrise_gameplay::game::server_side::explosion::BlockInteraction::Keep,
                false,
            )
            .unwrap();
            for target in [stand, painting, frame, boat] {
                assert_eq!(game.target_position(target), None, "{target:?} removed");
            }
            assert!(game.cushions.is_empty());
            assert!(
                game.dropped.is_empty() && game.orbs.is_empty(),
                "blast discards items and orbs: items={:?} orbs={}",
                game.dropped
                    .iter()
                    .map(|i| (&i.stack.item, i.health))
                    .collect::<Vec<_>>(),
                game.orbs.len()
            );
            let events = deaths();
            assert_eq!(events.len(), 5);
            assert!(events.contains(&("minecraft:entity_die", at, expected_source)));
            assert!(events.contains(&("minecraft:entity_die", stand_at, expected_source)));
            assert_eq!(
            events
                .iter()
                .filter(|(_, pos, who)| *pos == [4.5, 200.5, 4.03125] && *who == expected_source)
                .count(),
            2,
            "painting and frame centres: {events:?}"
        );
            assert!(events.contains(&("minecraft:entity_die", [3.5, 200.0, 4.5], None)));
        }
        // Explosion attribution must not leak into subsequent unrelated removals.
        game.server.combat.0.explosion_owner = None;
        let target = game
            .summon_with_nbt("minecraft:cushion", at, "{}")
            .unwrap()
            .unwrap();
        capture();
        game.kill_target(target);
        assert_eq!(deaths(), vec![("minecraft:entity_die", at, None)]);
    });
}

#[test]
#[ignore = "daily"]
fn painting_falling_off_the_wall_posts_no_entity_die() {
    with_game(|game| {
        game.run_command("gamerule entity_drops true");
        // A south-facing painting at (4, 200, 4) hangs on the block at z 3.
        game.run_command("setblock 4 200 3 stone strict");
        game.run_command("setblock 1 200 4 sculk_sensor strict");
        let painting = game
            .summon_with_nbt(
                "minecraft:painting",
                [4.5, 200.5, 4.5],
                "{variant:\"minecraft:kebab\",facing:0b}",
            )
            .unwrap()
            .expect("the painting joins");
        assert_eq!(game.paintings.len(), 1);
        game.sculk_prepare();
        game.sculk_sync_index();
        capture();
        // BlockAttachedEntity.tick: `ticksSinceLastCheck++ >= 100`, the 101st.
        for _ in 0..101 {
            game.tick_paintings();
        }
        assert_eq!(game.paintings.len(), 1, "the wall still holds it");
        game.run_command("setblock 4 200 3 air strict");
        for _ in 0..101 {
            game.tick_paintings();
        }
        assert_eq!(game.target_position(painting), None, "discarded");
        assert!(game.paintings.is_empty());
        // BlockAttachedEntity.tick: discard(), then dropItem().
        assert_eq!(
            game.dropped
                .iter()
                .map(|i| &*i.stack.item)
                .collect::<Vec<_>>(),
            ["minecraft:painting"],
        );
        // ...and no gameEvent: a sculk sensor beside it stays quiet.
        assert!(
            deaths().is_empty(),
            "a discarded painting posts no entity_die"
        );
        let Some(BlockEntity::Sculk(sensor)) = game.world.block_entities.get(&(1, 200, 4)) else {
            panic!("sensor must exist");
        };
        assert!(
            sensor.vibration.selector.current.is_none(),
            "the sensor must not hear the painting fall"
        );
        // BlockAttachedEntity.kill (a hit or /kill) still posts it.
        let killed_at = [6.5, 200.5, 4.03125];
        game.summon_with_nbt("minecraft:painting", [6.5, 200.5, 4.5], "{facing:0b}")
            .unwrap()
            .expect("the second painting joins");
        capture();
        game.run_command("kill @e[type=minecraft:painting]");
        assert!(game.paintings.is_empty());
        assert_eq!(deaths(), vec![("minecraft:entity_die", killed_at, None)]);
        let Some(BlockEntity::Sculk(sensor)) = game.world.block_entities.get(&(1, 200, 4)) else {
            panic!("sensor must exist");
        };
        let heard = sensor
            .vibration
            .selector
            .current
            .expect("the sensor hears it");
        assert_eq!(heard.0.event, "minecraft:entity_die");
        assert_eq!(heard.0.pos, killed_at);
    });
}

#[test]
#[ignore = "daily"]
fn unknown_painting_variant_keeps_the_default_one() {
    with_game(|game| {
        // Painting.defineSynchedData: VariantUtils.getAny, the registry's first.
        game.summon_with_nbt("minecraft:painting", [4.5, 200.5, 4.5], "{facing:0b}")
            .unwrap()
            .expect("a painting with no saved variant joins");
        // Painting.readAdditionalSaveData: a variant that does not resolve
        // leaves that default in place.
        game.summon_with_nbt(
            "minecraft:painting",
            [6.5, 200.5, 4.5],
            "{variant:\"minecraft:nosuch\",facing:0b}",
        )
        .unwrap()
        .expect("a painting with an unknown variant still joins");
        // Identifier.CODEC: a bare id is read in the `minecraft` namespace.
        game.summon_with_nbt(
            "minecraft:painting",
            [8.5, 200.5, 4.5],
            "{variant:\"kebab\",facing:0b}",
        )
        .unwrap()
        .expect("a painting with a bare variant id still joins");
        assert_eq!(game.paintings.len(), 3);
        let (default, unknown) = (&game.paintings[0], &game.paintings[1]);
        // RegistryDataLoader registers the registry's entries sorted by id, so
        // VariantUtils.getAny is `minecraft:alban`.
        assert_eq!(&*default.variant, "minecraft:alban");
        assert_eq!(unknown.variant, default.variant);
        assert_eq!(
            (unknown.width, unknown.height),
            (default.width, default.height)
        );
        assert_eq!(&*game.paintings[2].variant, "minecraft:kebab");
        let variant = sunrise_gameplay_common::world_paintings!(game)
            .get(&default.variant)
            .expect("the default variant is a real one");
        assert_ne!(&*variant.id, "minecraft:nosuch");
    });
}

#[test]
#[ignore = "daily"]
fn a_data_packs_placeable_painting_variant_is_placed() {
    with_game(|game| {
        let dir =
            std::env::temp_dir().join(format!("sunrise-painting-pack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pack = dir.join("datapacks/paintings");
        let variants = pack.join("data/test/painting_variant");
        let tags = pack.join("data/minecraft/tags/painting_variant");
        std::fs::create_dir_all(&variants).unwrap();
        std::fs::create_dir_all(&tags).unwrap();
        std::fs::write(
            pack.join("pack.mcmeta"),
            r#"{"pack":{"description":"fixture"}}"#,
        )
        .unwrap();
        std::fs::write(
            variants.join("big_square.json"),
            r#"{"asset_id":"test:big_square","width":2,"height":2}"#,
        )
        .unwrap();
        std::fs::write(
            tags.join("placeable.json"),
            r#"{"replace":true,"values":["test:big_square"]}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("datapacks.json"),
            r#"{"enabled":["file/paintings"],"disabled":[]}"#,
        )
        .unwrap();
        game.server.world_root = dir.clone();
        let stack = game.reload_datapacks(false, &[]).unwrap();
        let root = game.options.resources.clone();
        game.server.world_tables.0 = game
            .stage_world_tables(&stack, false, &root)
            .unwrap()
            .tables;
        // A 2x2 wall of stone at z = 5, painted on its south face.
        game.run_command("fill 4 200 5 5 201 5 stone");
        let hit = sunrise_gameplay_common::game::Hit {
            pos: [4, 200, 5],
            face: [0, 0, 1],
            point: [4.5, 200.5, 6.0],
            inside: false,
        };
        let painting = sunrise_items::items::ItemStack::new("minecraft:painting", 1);
        assert!(game.painting_fits(&hit, &painting));
        assert!(game.place_painting(&hit, &painting));
        assert_eq!(game.paintings.len(), 1);
        assert_eq!(&*game.paintings[0].variant, "test:big_square");
        assert_eq!((game.paintings[0].width, game.paintings[0].height), (2, 2));
        let _ = std::fs::remove_dir_all(&dir);
    });
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
