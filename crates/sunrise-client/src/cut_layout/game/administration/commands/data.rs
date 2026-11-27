#[cfg(test)]
use sunrise_core::nbt::Tag;
#[cfg(test)]
use sunrise_core::nbt::parse_snbt;
#[cfg(test)]
use sunrise_gameplay_common::game::commands_common::Target;
#[cfg(test)]
pub(crate) mod tests {
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;

    /// `lan_guest_joined`'s placement-only pass, between a forced write and
    /// the tracker pass, must not spend the marks in either dimension.
    #[test]
    #[ignore = "daily"]
    fn guest_placement_keeps_forced_metadata_in_each_dimension() {
        use sunrise_sim::lan::entity::{EntityState, States};
        with_game(|game| {
            for elsewhere in [false, true] {
                game.persist.force_meta(12, 8);
                let mut parked = elsewhere.then(|| std::mem::take(&mut game.persist));
                let mut placement = States::default();
                placement.push(EntityState {
                    id: 12,
                    ..Default::default()
                });
                game.lan_entity_uuids(&mut placement);
                if let Some(ref mut saved) = parked {
                    std::mem::swap(&mut game.persist, saved);
                }
                assert_eq!(game.persist.forced_meta.get(&12), Some(&[1 << 8, 0, 0, 0]));
                let mut snapshot = States::default();
                snapshot.push(EntityState {
                    id: 12,
                    ..Default::default()
                });
                game.lan_entity_uuids(&mut snapshot);
                assert_eq!(snapshot.as_slice()[0].forced_meta, [1 << 8, 0, 0, 0]);
                game.spend_forced_meta();
                assert!(game.persist.forced_meta.is_empty());
            }
        });
    }
}

#[cfg(test)]
#[path = "data/orb_uuid_tests.rs"]
pub(crate) mod orb_uuid_tests;

#[cfg(test)]
#[path = "data/eye_item_tests.rs"]
pub(crate) mod eye_item_tests;

#[cfg(test)]
pub(crate) mod wolf_live_anger_tests {
    use sunrise_core::nbt::Tag;
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_gameplay_common::game::commands_common::Target;
    use {
        crate::game::load_resources, crate::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
    };

    #[test]
    #[ignore = "daily"]
    fn live_wolf_anger_writes_use_the_level_clock_and_keep_bee_state() {
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
        game.world.game_time = 12_345;
        // Bee's independently modeled end is the first control, before the Wolf regression.
        for (kind, health, number) in [("bee", 10.0_f32, 1), ("wolf", 8.0, 2)] {
            game.mobs.clear();
            let tag = Tag::Compound(
                [
                    (
                        "id".into(),
                        Tag::String(format!("minecraft:{kind}").as_str().into()),
                    ),
                    ("UUID".into(), Tag::IntArray(vec![316, 1, 0, number])),
                    (
                        "Pos".into(),
                        Tag::List(vec![
                            Tag::Double(10.5_f64.to_bits()),
                            Tag::Double(200.0_f64.to_bits()),
                            Tag::Double(10.5_f64.to_bits()),
                        ]),
                    ),
                    ("Health".into(), Tag::Float(health.to_bits())),
                    ("NoAI".into(), Tag::Byte(1)),
                    ("NoGravity".into(), Tag::Byte(1)),
                    ("Silent".into(), Tag::Byte(1)),
                    ("PersistenceRequired".into(), Tag::Byte(1)),
                    ("anger_end_time".into(), Tag::Long(-1)),
                ]
                .into_iter()
                .collect(),
            );
            let id = game.load_mob(tag, None).unwrap();
            let i = game.mobs.iter().position(|m| m.id == id).unwrap();
            game.mobs[i].invulnerable = 17;
            game.mobs[i].last_hurt = 2.0;
            game.mobs[i].age = 6;
            game.mobs[i].in_powder_snow = true;
            for end in [12_395, 12_395, 12_420, -1] {
                let mut tag = game.mob_tree_nbt(i);
                let Tag::Compound(fields) = &mut tag else {
                    panic!("mob compound")
                };
                fields.insert("anger_end_time".into(), Tag::Long(end));
                game.apply_entity_nbt(Target::Mob(id), tag).unwrap();
                let saved = game.mob_tree_nbt(i);
                assert_eq!(
                    saved.get("anger_end_time"),
                    Some(&Tag::Long(end)),
                    "{kind} end={end}"
                );
                assert_eq!(game.world.game_time, 12_345);
                assert_eq!(game.persist.uuids.get(&id), Some(&[316, 1, 0, number]));
                assert_eq!(game.mobs[i].health, health);
                assert!(game.mobs[i].no_ai);
                assert_eq!(
                    (
                        game.mobs[i].invulnerable,
                        game.mobs[i].last_hurt,
                        game.mobs[i].age
                    ),
                    (17, 2.0, 6)
                );
                assert!(game.mobs[i].in_powder_snow, "{kind} end={end}");
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod boat_passenger_tree_tests {
    use super::*;
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    #[test]
    #[ignore = "daily"]
    fn actual_boat_getter_projects_runtime_children_and_existing_bookkeeping() {
        with_game(|game| {
            game.build_registries();
            game.set_allow_commands(true);
            // Supply the synthetic chunk before forceload's synchronous load barrier.
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            assert!(game.run_command("forceload add 0 0 15 15"));
            assert!(game.world.loaded(8, 8));
            assert!(game.run_command("setblock 8 199 8 stone"));
            assert!(game.run_command("summon oak_boat 8.5 200 8.5 {UUID:[I;7,151,1,1],NoGravity:1b,Silent:1b,CustomName:\"Hull\"}"));
            let bi = game.boats.iter().position(|b| b.position.x == 8.5).unwrap();
            let bid = game.boats[bi].id;
            let child = game
                .spawn_mob(
                    "minecraft:cow",
                    [8.5, 200.0, 8.5],
                    true,
                    sunrise_gameplay_common::game::creatures::SpawnSite::Plain,
                )
                .unwrap();
            let ci = game.mobs.iter().position(|m| m.id == child).unwrap();
            game.mobs[ci].no_ai = true;
            game.mobs[ci].no_gravity = true;
            assert!(
                !game.persist.raw.contains_key(&child),
                "fresh actual runtime child has no raw NBT"
            );
            assert_eq!(
                game.mobs[ci].extra.anger_end, 0,
                "unprimed constructor state"
            );
            game.ride_mount(Target::Mob(child), Target::Boat(bid))
                .unwrap();
            assert_eq!(game.mobs[ci].extra.mount, Some(bid));
            let clock = game.world.game_time;
            let chunks = game.world.chunks.len();
            let raw = game.persist.raw.get(&bid).cloned();
            assert!(raw.as_ref().unwrap().get("Passengers").is_none());
            let before = (
                game.persist.uuids.len(),
                game.persist.uuid_of(child),
                game.mobs[ci].extra.anger_end,
                game.mobs[ci].extra.angry_at,
            );
            let first = game.command_target_nbt(Target::Boat(bid)).unwrap();
            let after = (
                game.persist.uuids.len(),
                game.persist.uuid_of(child),
                game.mobs[ci].extra.anger_end,
                game.mobs[ci].extra.angry_at,
            );
            println!("parent-first bookkeeping before={before:?} after={after:?}");
            let mut failures = Vec::new();
            if after.2 != -1 {
                failures.push("first parent must use existing 0->-1 normalization".to_owned());
            }
            if after.1.is_none() || after.0 != before.0 + usize::from(before.1.is_none()) {
                failures.push("first parent must use existing UUID registration once".into());
            }
            let control = game
                .spawn_mob(
                    "minecraft:cow",
                    [12.5, 200.0, 8.5],
                    true,
                    sunrise_gameplay_common::game::creatures::SpawnSite::Plain,
                )
                .unwrap();
            let di = game.mobs.iter().position(|m| m.id == control).unwrap();
            game.mobs[di].no_ai = true;
            game.mobs[di].no_gravity = true;
            assert!(!game.persist.raw.contains_key(&control));
            assert_eq!(game.mobs[di].extra.anger_end, 0);
            let direct_before = (
                game.persist.uuids.len(),
                game.persist.uuid_of(control),
                game.mobs[di].extra.anger_end,
            );
            let direct_control = game.command_target_nbt(Target::Mob(control)).unwrap();
            let direct_after = (
                game.persist.uuids.len(),
                game.persist.uuid_of(control),
                game.mobs[di].extra.anger_end,
            );
            println!(
                "existing child-getter bookkeeping before={direct_before:?} after={direct_after:?}"
            );
            assert_eq!(direct_after.2, -1);
            assert_eq!(
                direct_after.0,
                direct_before.0 + usize::from(direct_before.1.is_none())
            );
            assert!(direct_after.1.is_some());
            assert_eq!(
                direct_control.get("UUID"),
                Some(&Tag::IntArray(direct_after.1.unwrap().to_vec()))
            );
            let direct_child = game.command_target_nbt(Target::Mob(child)).unwrap();
            let saved_child = |tag: &Tag| match tag.get("Passengers") {
                Some(Tag::List(v)) if v.len() == 1 => Some(v[0].clone()),
                _ => None,
            };
            if saved_child(&first).as_ref() != Some(&direct_child) {
                failures.push("first parent typed child must equal actual child getter".into());
            }
            let stable = game.persist.uuids.len();
            for _ in 0..2 {
                let again = game.command_target_nbt(Target::Boat(bid)).unwrap();
                if saved_child(&again).as_ref() != Some(&direct_child) {
                    failures.push("repeated parent must keep exactly one actual child".into());
                }
                assert_eq!(game.persist.uuids.len(), stable);
                assert_eq!(game.mobs[ci].extra.anger_end, -1);
                assert_eq!(game.mobs[ci].extra.mount, Some(bid));
            }
            assert!(game.run_command("scoreboard objectives add h151 dummy"));
            assert!(game.run_command("scoreboard players set #r h151 91"));
            assert!(game.run_command("scoreboard players set #s h151 17"));
            let accepted=game.run_command("execute store result score #r h151 store success score #s h151 run data get entity @e[nbt={UUID:[I;7,151,1,1]},limit=1] Passengers[0].UUID");
            let score = |name| game.score.board.score(name, "h151").unwrap().value;
            if (accepted, score("#r"), score("#s")) != (true, 4, 1) {
                failures.push("registered data command must reach live tree".into());
            }
            game.boats.update(bi, |boat| {
                boat.leash = Some(sunrise_sim::mobs::leash::Holder::Mob(control))
            });
            let leashed = game.command_target_nbt(Target::Boat(bid)).unwrap();
            assert_eq!(
                leashed.get("leash").and_then(|t| t.get("UUID")),
                direct_control.get("UUID")
            );
            assert_eq!(leashed.get("CustomName"), Some(&Tag::String("Hull".into())));
            assert_eq!(leashed.get("Silent"), Some(&Tag::Byte(1)));
            game.dismount_mob(child);
            assert!(
                game.command_target_nbt(Target::Boat(bid))
                    .unwrap()
                    .get("Passengers")
                    .is_none()
            );
            game.ride_mount(Target::Mob(child), Target::Boat(bid))
                .unwrap();
            let remount_child = game.command_target_nbt(Target::Mob(child)).unwrap();
            if saved_child(&game.command_target_nbt(Target::Boat(bid)).unwrap()).as_ref()
                != Some(&remount_child)
            {
                failures.push("actual remount tree".into());
            }
            assert_eq!(
                game.persist.raw.get(&bid),
                raw.as_ref(),
                "getter must not replay/mutate raw passenger input"
            );
            assert_eq!(game.world.chunks.len(), chunks, "no chunk loading");
            assert_eq!(game.world.game_time, clock, "no scene ticks");
            assert!(
                failures.is_empty(),
                "actual boat passenger getter mismatches: {failures:?}"
            );
        });
    }
}

#[cfg(test)]
pub(crate) mod loot_table_tests {
    use std::sync::Arc;
    use sunrise_core::nbt::Tag;
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_items::items::ItemStack;
    use sunrise_level::{chunk::Chunk, containers::BlockEntity};
    use sunrise_sim::save::{self, ChunkExtras, PendingContainerLoot};
    use {
        crate::game::load_resources, crate::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
    };

    const TABLE: &str = "minecraft:chests/simple_dungeon";

    /// The chest as a chunk save writes it, and its slots when that save loads.
    fn saved_and_reloaded(
        game: &sunrise_gameplay::game::ServerGame<'_>,
        pos: (i32, i32, i32),
    ) -> (Tag, Vec<Option<ItemStack>>) {
        let live = game.world.chunks.get(&(0, 0)).unwrap().clone();
        let chunk = game.with_entities([&live]).remove(0);
        let registry = game.pack.assets.registry;
        let encoded = save::encode(&chunk, registry, game.res.biomes(), &Default::default());
        let Some(Tag::List(list)) = encoded.tag.get("block_entities") else {
            panic!("no block entities");
        };
        let saved = list
            .iter()
            .find(|t| save::number(t.get("x")) == Some(i64::from(pos.0)))
            .unwrap()
            .clone();
        let reloaded = save::decode(&encoded, registry, game.res.biomes()).unwrap();
        let (_, BlockEntity::Chest(slots)) = reloaded
            .block_entities
            .iter()
            .find(|(p, _)| *p == pos)
            .unwrap()
        else {
            panic!("chest");
        };
        (saved, slots.clone())
    }

    /// RandomizableContainer.tryLoadLootTable: a tag without a table loads an
    /// empty chest that stays so, and one with a table keeps it.
    #[test]
    #[ignore = "daily"]
    fn data_commands_on_an_unopened_loaded_chest_survive_a_save_and_reload() {
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

        // Two unopened chests, with one owner for their pending table tags.
        let removed = (1, 64, 2);
        let merged = (2, 64, 2);
        let chest = game
            .pack
            .assets
            .registry
            .default_state("minecraft:chest")
            .unwrap();
        let mut chunk = Chunk::empty(0, 0);
        let mut extras = ChunkExtras::default();
        for pos in [removed, merged] {
            let slots = vec![None; 27];

            chunk.set(pos.0, pos.1, pos.2, chest.0);
            extras.pending_loot.insert(
                pos,
                PendingContainerLoot {
                    table: Tag::String(TABLE.into()),
                    seed: Some(Tag::Long(42)),
                },
            );
            game.world
                .block_entities
                .insert(pos, BlockEntity::Chest(slots));
        }
        chunk.saved = Some(Arc::new(extras));
        game.world.chunks.insert((0, 0), Arc::new(chunk));

        for field in ["LootTable", "LootTableSeed"] {
            assert!(game.run_command(&format!("data get block 2 64 2 {field}")));
        }
        let raw = game.block_entity_nbt([2, 64, 2]).unwrap();
        assert_eq!(raw.get("LootTable"), Some(&Tag::String(TABLE.into())));
        assert_eq!(raw.get("LootTableSeed"), Some(&Tag::Long(42)));
        assert!(
            game.world
                .block_entities
                .get(&merged)
                .unwrap()
                .slots()
                .iter()
                .all(Option::is_none)
        );
        assert!(game.run_command("data remove block 1 64 2 LootTable"));
        let (saved, slots) = saved_and_reloaded(&game, removed);
        assert!(
            saved.get("LootTable").is_none(),
            "the emptied chest rolls again on load: {saved:?}"
        );
        assert!(saved.get("LootTableSeed").is_none());
        assert!(slots.iter().all(Option::is_none));

        assert!(game.run_command("data merge block 2 64 2 {LootTableSeed:7L}"));
        let (saved, _) = saved_and_reloaded(&game, merged);
        assert_eq!(save::text(saved.get("LootTable")).as_deref(), Some(TABLE));
        assert_eq!(save::number(saved.get("LootTableSeed")), Some(7));
        assert!(saved.get("Items").is_none());
    }
}

#[cfg(test)]
#[path = "data/portal_cooldown_tests.rs"]
pub(crate) mod portal_cooldown_tests;

#[cfg(test)]
pub(crate) mod compute_tests;

#[cfg(test)]
pub(crate) mod saved_tag_tests;

#[cfg(test)]
#[path = "data/stand_living_tests.rs"]
pub(crate) mod stand_living_tests;

#[cfg(test)]
pub(crate) mod mod_block_entity_tests;

#[cfg(test)]
#[path = "data/fire_tests.rs"]
pub(crate) mod fire_tests;

#[cfg(test)]
pub(crate) mod residency_tests;

#[cfg(test)]
pub(crate) mod hopper_cooldown_tests;

#[cfg(test)]
#[path = "data/rotation_tests.rs"]
pub(crate) mod rotation_tests;

#[cfg(test)]
pub(crate) mod arrow_reload_tests;

#[cfg(test)]
pub(crate) mod arrow_owner_tests {
    use sunrise_core::nbt::Tag;
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_gameplay_common::game::commands_common::Target;
    use {
        crate::game::load_resources, crate::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
    };

    #[test]
    #[ignore = "daily"]
    fn loaded_arrow_getter_uses_registered_owner_reference_without_raw() {
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

        let position = Tag::List(vec![
            Tag::Double(8.5_f64.to_bits()),
            Tag::Double(202.0_f64.to_bits()),
            Tag::Double(4.5_f64.to_bits()),
        ]);
        let mob_uuid = [213, 1, 2, 3];
        let mob = Tag::compound([
            ("id", Tag::String("minecraft:pig".into())),
            ("UUID", Tag::IntArray(mob_uuid.to_vec())),
            ("Pos", position.clone()),
            ("Health", Tag::Float(10.0_f32.to_bits())),
            ("NoAI", Tag::Byte(1)),
        ]);
        game.load_mob(mob, None).unwrap();
        assert!(game.mob_by_uuid(mob_uuid).is_some());
        let time = game.world.game_time;
        for (index, owner) in [Some(mob_uuid), Some([213, 91, 92, 93]), None]
            .into_iter()
            .enumerate()
        {
            let uuid = [213, 10, 0, index as i32 + 1];
            let mut tag = Tag::compound([
                ("id", Tag::String("minecraft:arrow".into())),
                ("UUID", Tag::IntArray(uuid.to_vec())),
                ("Pos", position.clone()),
                ("NoGravity", Tag::Byte(1)),
                ("CustomName", Tag::String("kept name".into())),
                ("Tags", Tag::List(vec![Tag::String("kept_tag".into())])),
            ]);
            if let (Some(owner), Tag::Compound(fields)) = (owner, &mut tag) {
                fields.insert("Owner".into(), Tag::IntArray(owner.to_vec()));
            }
            game.load_arrow(tag, Some(uuid), None);
            let p = game.creatures.projectiles.last().unwrap();
            // Prove actual loader adoption before reading any saved NBT.
            assert_eq!(p.owner_uuid, owner);
            let id = p.id;
            assert_eq!(game.persist.uuids.get(&id), Some(&uuid));
            let expected = owner.map(|u| Tag::IntArray(u.to_vec()));
            let loaded_raw = game.persist.raw.get(&id).cloned();
            let mut stale = loaded_raw.clone().unwrap();
            if let Tag::Compound(fields) = &mut stale {
                fields.insert("Owner".into(), Tag::IntArray(vec![99; 4]));
            }
            let mut raw_cases = vec![None, loaded_raw];
            // A positive modeled reference must overwrite a conflicting raw UUID.
            // Generic stale optional-key cleanup is outside this getter bridge.
            if owner.is_some() {
                raw_cases.push(Some(stale));
            }
            for raw in raw_cases {
                if let Some(raw) = &raw {
                    game.persist.raw.insert(id, raw.clone());
                } else {
                    game.persist.raw.remove(&id);
                }
                let saved = game.command_target_nbt(Target::Projectile(id)).unwrap();
                assert_eq!(saved.get("Owner"), expected.as_ref());
                assert_eq!(saved.get("UUID"), Some(&Tag::IntArray(uuid.to_vec())));
                if let Some(raw) = &raw {
                    assert_eq!(saved.get("CustomName"), raw.get("CustomName"));
                    assert_eq!(saved.get("Tags"), raw.get("Tags"));
                }
                assert_eq!(game.creatures.projectiles.last().unwrap().owner_uuid, owner);
                assert_eq!(game.persist.raw.get(&id), raw.as_ref());
                assert_eq!(game.world.game_time, time);
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod bell_command_identity_tests {
    use super::*;
    use std::sync::Arc;
    use sunrise_core::{geometry::Vec3, position::ChunkPos};
    use sunrise_level::chunk::Chunk;
    use sunrise_level::containers::BlockEntity;
    use sunrise_sim::motion::PlayerMotion;
    use sunrise_sim::save;
    fn with_game(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
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
            test(game);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn placed_bell_is_a_real_stored_entity_with_command_metadata() {
        with_game(|game| {
            let clock = game.world.game_time;
            assert!(game.run_command("setblock 8 200 4 chest[facing=north] strict"));
            assert!(game.world.block_entities.contains_key(&(8, 200, 4)));
            assert!(game.run_command("setblock 10 200 4 stone strict"));
            assert!(game.block_entity_nbt([10, 200, 4]).is_none());
            assert!(game.run_command(
                "setblock 4 200 4 bell[facing=north,attachment=floor,powered=false] strict"
            ));
            assert!(game.run_command(
                "execute if block 4 200 4 bell[facing=north,attachment=floor,powered=false]"
            ));
            assert!(game.world.chunks.contains_key(&(0, 0)));
            let entity = game
                .world
                .block_entities
                .get(&(4, 200, 4))
                .expect("actual placed bell stored in collection");
            assert_eq!(entity.id(), "minecraft:bell");
            assert!(entity.fits("minecraft:bell"));
            assert!(!entity.fits("minecraft:chest"));
            for _ in 0..2 {
                let tag = game.block_entity_nbt([4, 200, 4]).unwrap();
                for (key, value) in [
                    ("id", Tag::String("minecraft:bell".into())),
                    ("x", Tag::Int(4)),
                    ("y", Tag::Int(200)),
                    ("z", Tag::Int(4)),
                ] {
                    assert_eq!(tag.get(key), Some(&value));
                }
                assert!(game.run_command(
                    "execute if data block 4 200 4 {id:\"minecraft:bell\",x:4,y:200,z:4}"
                ));
            }
            // No raw input: factory/model and the real common writer supply metadata.
            let fresh = BlockEntity::for_block("minecraft:bell").unwrap();
            let tag = save::block_entity_encode::data(
                (-3, 204, 11),
                &fresh,
                "minecraft:bell",
                Default::default(),
                &Default::default(),
            );
            assert_eq!(tag.get("x"), Some(&Tag::Int(-3)));
            assert_eq!(tag.get("y"), Some(&Tag::Int(204)));
            assert_eq!(tag.get("z"), Some(&Tag::Int(11)));
            let (pos, back) = save::decode_block_entity(&tag).unwrap();
            assert_eq!(pos, (-3, 204, 11));
            assert_eq!(back.id(), "minecraft:bell");
            assert!(game.run_command(
                "setblock 4 200 4 bell[facing=east,attachment=floor,powered=false] strict"
            ));
            assert_eq!(
                game.world.block_entities.get(&(4, 200, 4)).unwrap().id(),
                "minecraft:bell"
            );
            assert!(game.run_command("setblock 4 200 4 air strict"));
            assert!(!game.world.block_entities.contains_key(&(4, 200, 4)));
            assert!(game.world.block_entities.contains_key(&(8, 200, 4)));
            assert!(game.run_command(
                "setblock 4 200 4 bell[facing=south,attachment=floor,powered=false] strict"
            ));
            assert_eq!(
                game.world.block_entities.get(&(4, 200, 4)).unwrap().id(),
                "minecraft:bell"
            );
            assert_eq!(game.world.game_time, clock);
        });
    }
}

#[cfg(test)]
pub(crate) mod falling_live_adoption_tests {
    use std::sync::Arc;
    use sunrise_core::position::ChunkPos;
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_level::chunk::Chunk;
    use {
        crate::game::load_resources, crate::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
    };
    pub(crate) fn with_game(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
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
                .join(format!("h098-{:?}", std::thread::current().id())),
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
        assert!(game.run_command("difficulty normal"));
        if !game.server.streaming.forced.contains(&(0, 0)) {
            assert!(game.run_command("forceload add 0 0"));
        }
        test(&mut game);
    }
}

#[cfg(test)]
pub(crate) mod vehicle_container_tests {
    use super::*;

    #[test]
    #[ignore = "daily"]
    fn loaded_vehicle_saves_own_only_their_container_fields() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.build_registries();
            game.set_allow_commands(true);
            let cases = [
                (
                    "boat_plain",
                    "minecraft:oak_boat",
                    "Items:[{Slot:0b,id:\"minecraft:diamond\",count:3,components:{\"minecraft:custom_data\":{h052:9}}}],LootTable:\"minecraft:empty\",LootTableSeed:73L",
                    None,
                    false,
                    "",
                ),
                (
                    "cart_plain",
                    "minecraft:minecart",
                    "Items:[{Slot:0b,id:\"minecraft:diamond\",count:3,components:{\"minecraft:custom_data\":{h052:9}}}],LootTable:\"minecraft:empty\",LootTableSeed:73L",
                    None,
                    false,
                    "",
                ),
                // Only container carts read a loot table: one on any other cart is dropped.
                (
                    "tnt_cart_plain",
                    "minecraft:tnt_minecart",
                    "Items:[{Slot:0b,id:\"minecraft:diamond\",count:3,components:{\"minecraft:custom_data\":{h052:9}}}],LootTable:\"minecraft:empty\",LootTableSeed:73L",
                    None,
                    false,
                    "",
                ),
                (
                    "boat_items",
                    "minecraft:oak_chest_boat",
                    "Items:[{Slot:0b,id:\"minecraft:diamond\",count:3,components:{\"minecraft:custom_data\":{h052:9}}}],LootTableSeed:73L",
                    Some(1usize),
                    false,
                    "",
                ),
                (
                    "cart_items",
                    "minecraft:chest_minecart",
                    "Items:[{Slot:0b,id:\"minecraft:diamond\",count:3,components:{\"minecraft:custom_data\":{h052:9}}}],LootTableSeed:73L",
                    Some(1usize),
                    false,
                    "",
                ),
                (
                    "boat_table",
                    "minecraft:oak_chest_boat",
                    "LootTable:\"minecraft:empty\",LootTableSeed:73L",
                    None,
                    true,
                    "minecraft:empty",
                ),
                (
                    "cart_table",
                    "minecraft:chest_minecart",
                    "LootTable:\"minecraft:empty\",LootTableSeed:73L",
                    None,
                    true,
                    "minecraft:empty",
                ),
                (
                    "boat_empty_table",
                    "minecraft:oak_chest_boat",
                    "LootTable:\"\",LootTableSeed:73L",
                    None,
                    true,
                    "minecraft:",
                ),
                (
                    "cart_empty_table",
                    "minecraft:chest_minecart",
                    "LootTable:\"\",LootTableSeed:73L",
                    None,
                    true,
                    "minecraft:",
                ),
                (
                    "boat_empty",
                    "minecraft:oak_chest_boat",
                    "",
                    Some(0usize),
                    false,
                    "",
                ),
                (
                    "cart_empty",
                    "minecraft:chest_minecart",
                    "",
                    Some(0usize),
                    false,
                    "",
                ),
            ];
            let clock = game.world.game_time;
            let mut differences = Vec::new();
            for (i, (name, kind, fields, expected_items, pending, table_text)) in
                cases.iter().enumerate()
            {
                let uuid = [52, 0, 0, i as i32 + 1];
                let source = [
                    r#"{Pos:[4.5d,200d,4.5d],NoGravity:1b,Tags:["h052"]"#,
                    if fields.is_empty() { "" } else { "," },
                    fields,
                    "}",
                ]
                .concat();
                let (tag, _) = parse_snbt(&source).unwrap();
                let target = if kind.contains("boat") {
                    game.load_boat(tag, Some(uuid), kind);
                    let b = game.boats.last().unwrap();
                    assert_eq!(
                        b.items
                            .first()
                            .and_then(Option::as_ref)
                            .map_or(0, |s| s.count),
                        if *expected_items == Some(1) { 3 } else { 0 },
                        "{name}: actual boat inventory"
                    );
                    Target::Boat(b.id)
                } else {
                    game.load_cart(tag, Some(uuid), kind);
                    let c = game.carts.last().unwrap();
                    assert_eq!(
                        c.items
                            .first()
                            .and_then(Option::as_ref)
                            .map_or(0, |s| s.count),
                        if *expected_items == Some(1) { 3 } else { 0 },
                        "{name}: actual cart inventory"
                    );
                    Target::Cart(c.id)
                };
                let id = match target {
                    Target::Boat(id) | Target::Cart(id) => id,
                    _ => unreachable!(),
                };
                let saved = game.command_target_nbt(target).unwrap();
                assert_eq!(saved.get("UUID"), Some(&Tag::IntArray(uuid.to_vec())));
                assert_eq!(
                    saved.get("Tags"),
                    Some(&Tag::List(vec![Tag::String("h052".into())]))
                );
                let actual_items = match saved.get("Items") {
                    Some(Tag::List(items)) => Some(items.len()),
                    None => None,
                    other => panic!("{name}: {other:?}"),
                };
                if actual_items != *expected_items {
                    differences.push(format!(
                        "{name}: Items {actual_items:?} != {expected_items:?}"
                    ));
                }
                let table = pending.then(|| Tag::String((*table_text).into()));
                let seed = pending.then_some(Tag::Long(73));
                for (key, expected) in [
                    ("LootTable", table.as_ref()),
                    ("LootTableSeed", seed.as_ref()),
                ] {
                    if saved.get(key) != expected {
                        differences.push(format!(
                            "{name}: {key} {:?} != {expected:?}",
                            saved.get(key)
                        ));
                    }
                }
                if *expected_items == Some(1) {
                    // The model, not a stale loaded Items list, owns the inventory.
                    match target {
                        Target::Boat(_) => {
                            game.common
                                .boats
                                .update(game.common.boats.len() - 1, |boat| {
                                    boat.items[0].as_mut().unwrap().count = 4
                                });
                        }
                        Target::Cart(_) => {
                            game.common
                                .carts
                                .update(game.common.carts.len() - 1, |cart| {
                                    cart.items[0].as_mut().unwrap().count = 4
                                });
                        }
                        _ => unreachable!(),
                    }
                }
                // A normal model can save without any raw input. Boat pending tables
                // already live in the retained source; no replacement pending state is invented.
                if !pending {
                    game.persist.raw.remove(&id);
                    let saved = game.command_target_nbt(target).unwrap();
                    let expected = if *expected_items == Some(1) { 4 } else { 0 };
                    let count = match saved.get("Items") {
                        Some(Tag::List(items)) => {
                            items.first().and_then(|t| t.get("count")).cloned()
                        }
                        _ => None,
                    };
                    assert_eq!(
                        count,
                        (expected != 0).then_some(Tag::Int(expected)),
                        "{name}: no-raw model count"
                    );
                    assert!(
                        saved.get("LootTable").is_none() && saved.get("LootTableSeed").is_none()
                    );
                    assert_eq!(saved.get("UUID"), Some(&Tag::IntArray(uuid.to_vec())));
                }
            }
            // Retained loader diagnostic, not a writer parity assertion: an input
            // containing both a table and Items currently keeps modeled contents.
            for kind in ["minecraft:oak_chest_boat", "minecraft:chest_minecart"] {
                let source = r#"{Pos:[4.5d,200d,4.5d],LootTable:"minecraft:empty",LootTableSeed:73L,Items:[{Slot:0b,id:"minecraft:diamond",count:3}]}"#;
                let (tag, _) = parse_snbt(source).unwrap();
                let count = if kind.contains("boat") {
                    game.load_boat(tag, Some([52, 1, 0, 1]), kind);
                    game.boats.last().unwrap().items[0]
                        .as_ref()
                        .map_or(0, |s| s.count)
                } else {
                    game.load_cart(tag, Some([52, 1, 0, 2]), kind);
                    game.carts.last().unwrap().items[0]
                        .as_ref()
                        .map_or(0, |s| s.count)
                };
                eprintln!(
                    "separate loader diagnostic {kind}: supplied table+Items retains modeled count {count}"
                );
            }
            assert_eq!(game.world.game_time, clock);
            assert!(
                differences.is_empty(),
                "class-owned save differences:\n{}",
                differences.join("\n")
            );
        });
    }
}

#[cfg(test)]
#[path = "finalize_marker_tests.rs"]
pub(crate) mod finalize_marker_tests;
#[cfg(test)]
use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
