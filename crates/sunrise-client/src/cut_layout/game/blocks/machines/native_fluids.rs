#![cfg(test)]
use mod_api::FluidStack;
use std::sync::Arc;
#[cfg(test)]
pub(crate) mod tests {

    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;

    use sunrise_gameplay_common::game::transactions::TransactionsGameExt;

    use super::*;

    #[test]
    #[ignore = "daily"]
    fn a_mods_cauldron_fills_and_drains_by_its_levels_and_a_refusal_names_both_sides() {
        use super::super::super::native_entity_refs::tests::{
            record_key, run_fresh_cases, with_game_at,
        };
        if run_fresh_cases(
            "cut_layout::game::server_side::machines::native_fluids::tests::a_mods_cauldron_fills_and_drains_by_its_levels_and_a_refusal_names_both_sides",
            "SUNRISE_CAULDRON_CONTENT_CHILD",
            &["valid"],
        ) {
            return;
        }
        use mod_api::{
            BlockKey, BlockUpdateFlags, FLUIDS, FluidKey, FluidResource, Hooks, ModEntry, Mods,
            RegisterCauldronFluidContentEvent, Registrar, RegistrationContext,
        };
        use sunrise_level::chunk::Chunk;

        /// The keys the mod's `register` requested, as a real mod keeps them for its hooks.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        struct Keys {
            snow_cauldron: BlockKey,
            water_cauldron: BlockKey,
            composter: BlockKey,
            stone: BlockKey,
            syrup: FluidKey,
            honey: FluidKey,
            water: FluidKey,
        }
        #[derive(Default)]
        struct SyrupCauldron(std::sync::OnceLock<Keys>);
        #[mod_api::__subscriptions]
        impl Hooks for SyrupCauldron {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                let keys = Keys {
                    snow_cauldron: r.block_key("minecraft:powder_snow_cauldron"),
                    water_cauldron: r.block_key("minecraft:water_cauldron"),
                    composter: r.block_key("minecraft:composter"),
                    stone: r.block_key("minecraft:stone"),
                    syrup: r.declare_fluid("syrup"),
                    honey: r.declare_fluid("honey"),
                    water: r.fluid_key("minecraft:water"),
                };
                // Its provider is declared only in the later cauldron-content event below.
                // Coverage queries must not execute that future callback early.
                assert!(!r.is_block_capability_registered(&FLUIDS, &keys.snow_cauldron));
                record_key(&self.0, keys);
            }
            fn register_cauldron_fluid_content(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterCauldronFluidContentEvent<'_>,
            ) {
                let keys = self.0.get().unwrap();
                // The powder snow cauldron holds no fluid here, so it can be the syrup
                // cauldron: 3000 mB over its `level` 1..3.
                event
                    .contents
                    .register(keys.snow_cauldron, keys.syrup, 3000, Some("level"));
                // Refused: the water cauldron already has a content, water already has a
                // cauldron, and stone has no level property.
                event
                    .contents
                    .register(keys.water_cauldron, keys.honey, 1000, Some("level"));
                event
                    .contents
                    .register(keys.composter, keys.water, 8000, Some("level"));
                event
                    .contents
                    .register(keys.stone, keys.honey, 1000, Some("level"));
            }
        }
        let mods: Mods = vec![ModEntry::new(
            "cauldron_test",
            Box::new(SyrupCauldron::default()),
        )]
        .into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            let base =
                std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
            let root =
                sunrise_gameplay_common::game::mods::content_root(&base, &Default::default())
                    .unwrap()
                    .prepared()
                    .to_path_buf();
            with_game_at(root, |game| {
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let level = game.level("minecraft:overworld").unwrap();
                let flags = BlockUpdateFlags::NEIGHBORS;
                let block = |game: &sunrise_gameplay::game::ServerGame<'_>,
                             name: &str,
                             level: Option<u32>| {
                    let state = sunrise_world::chunk::BlockState {
                        name: name.to_owned(),
                        properties: level
                            .map(|n| ("level".to_owned(), n.to_string()))
                            .into_iter()
                            .collect(),
                    };
                    game.pack.assets.registry.resolve(&state).unwrap()
                };
                let resource = |fluid: &str| {
                    FluidResource::of(&FluidStack::new(
                        mod_api::__private::fluid_key(fluid).unwrap(),
                        1,
                    ))
                };
                let (syrup, water, lava) = (
                    resource("cauldron_test:syrup"),
                    resource("minecraft:water"),
                    resource("minecraft:lava"),
                );

                // The refusals name both sides; the rest of the mod's contents still apply,
                // and water and lava keep theirs.
                let (contents, refusals) =
                    sunrise_gameplay_common::game::mods::collect_cauldron_fluid_contents(
                        game.pack.assets.registry,
                    );
                assert_eq!(refusals.len(), 3, "{refusals:?}");
                for (refusal, named) in refusals.iter().zip([
                    [
                        "minecraft:water_cauldron",
                        "minecraft:water",
                        "cauldron_test:honey",
                    ],
                    [
                        "minecraft:water",
                        "minecraft:water_cauldron",
                        "minecraft:composter",
                    ],
                    ["minecraft:stone", "level", "cauldron_test:honey"],
                ]) {
                    assert!(
                        named.iter().all(|name| refusal.contains(name)),
                        "{refusal}: names {named:?}"
                    );
                }
                let content = |block: &str| {
                    contents.of_block(block).map(|(_, content)| {
                        (content.fluid.clone(), content.total, content.levels())
                    })
                };
                assert_eq!(
                    content("minecraft:powder_snow_cauldron"),
                    Some(("cauldron_test:syrup".to_owned(), 3000, 3))
                );
                assert_eq!(
                    content("minecraft:water_cauldron"),
                    Some(("minecraft:water".to_owned(), 1000, 3))
                );
                assert_eq!(
                    content("minecraft:lava_cauldron"),
                    Some(("minecraft:lava".to_owned(), 1000, 1))
                );
                assert_eq!(content("minecraft:composter"), None);
                assert_eq!(content("minecraft:stone"), None);

                // The handler on the syrup cauldron reports syrup and a whole number of
                // levels, 1000 mB each, at each level, with room for 3000 mB of syrup.
                let pos = [0, 64, 0];
                let mut handler = None;
                for (n, held) in [(1, 1000), (2, 2000), (3, 3000)] {
                    let cauldron = block(game, "minecraft:powder_snow_cauldron", Some(n));
                    assert_eq!(game.level_set_block(level, pos, cauldron, flags), Ok(true));
                    // The handler is the position's: one answers every cauldron there.
                    let handler = handler
                        .get_or_insert_with(|| level.capability(game, &FLUIDS, pos, None).unwrap());
                    assert_eq!(handler.size(game), 1);
                    assert_eq!(handler.resource(game, 0), Some(syrup.clone()));
                    assert_eq!(handler.amount(game, 0), held);
                    assert_eq!(handler.capacity(game, 0, Some(&syrup)), 3000);
                    assert_eq!(handler.capacity(game, 0, Some(&water)), 1000);
                }
                let handler = handler.unwrap();
                let extract = |game: &mut sunrise_gameplay::game::ServerGame<'_>,
                               resource: &FluidResource,
                               amount| {
                    game.storage_transaction(|host, tx| {
                        handler.extract_at(host, tx, 0, resource, amount)
                    })
                    .unwrap()
                };
                let insert = |game: &mut sunrise_gameplay::game::ServerGame<'_>,
                              resource: &FluidResource,
                              amount| {
                    game.storage_transaction(|host, tx| {
                        handler.insert_at(host, tx, 0, resource, amount)
                    })
                    .unwrap()
                };
                let at = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                    game.world.block(pos[0], pos[1], pos[2])
                };
                // A drain takes whole levels: 1500 mB asked takes one level, 999 none, and the
                // last level leaves an empty cauldron.
                assert_eq!(extract(game, &syrup, 1500), 1000);
                assert_eq!(
                    at(game),
                    block(game, "minecraft:powder_snow_cauldron", Some(2))
                );
                assert_eq!(extract(game, &syrup, 999), 0);
                assert_eq!(extract(game, &water, 1000), 0);
                assert_eq!(
                    at(game),
                    block(game, "minecraft:powder_snow_cauldron", Some(2))
                );
                assert_eq!(extract(game, &syrup, 2000), 2000);
                assert_eq!(at(game), block(game, "minecraft:cauldron", None));
                assert_eq!(handler.resource(game, 0), None);
                assert_eq!(handler.amount(game, 0), 0);
                // An empty cauldron filled with syrup becomes the syrup cauldron at the level
                // the amount reaches; honey has no content and goes nowhere.
                assert_eq!(insert(game, &resource("cauldron_test:honey"), 3000), 0);
                assert_eq!(at(game), block(game, "minecraft:cauldron", None));
                assert_eq!(insert(game, &syrup, 2500), 2000);
                assert_eq!(
                    at(game),
                    block(game, "minecraft:powder_snow_cauldron", Some(2))
                );
                assert_eq!(insert(game, &water, 1000), 0);
                assert_eq!(insert(game, &syrup, 1000), 1000);
                assert_eq!(
                    at(game),
                    block(game, "minecraft:powder_snow_cauldron", Some(3))
                );
                assert_eq!(insert(game, &syrup, 1000), 0);

                // Water and lava answer exactly as before: water by thirds of a bucket, the
                // top third 334 mB, lava a bucket only whole, and syrup into neither.
                assert_eq!(extract(game, &syrup, 3000), 3000);
                assert_eq!(insert(game, &water, 500), 333);
                assert_eq!(at(game), block(game, "minecraft:water_cauldron", Some(1)));
                assert_eq!(insert(game, &syrup, 1000), 0);
                assert_eq!(insert(game, &water, 1000), 667);
                assert_eq!(at(game), block(game, "minecraft:water_cauldron", Some(3)));
                assert_eq!(handler.resource(game, 0), Some(water.clone()));
                assert_eq!(handler.amount(game, 0), 1000);
                assert_eq!(extract(game, &water, 500), 334);
                assert_eq!(at(game), block(game, "minecraft:water_cauldron", Some(2)));
                assert_eq!(extract(game, &water, 1000), 666);
                assert_eq!(at(game), block(game, "minecraft:cauldron", None));
                assert_eq!(insert(game, &lava, 999), 0);
                assert_eq!(insert(game, &lava, 1500), 1000);
                assert_eq!(at(game), block(game, "minecraft:lava_cauldron", None));
                assert_eq!(handler.resource(game, 0), Some(lava.clone()));
                assert_eq!(handler.amount(game, 0), 1000);
                assert_eq!(handler.capacity(game, 0, Some(&lava)), 1000);
                assert_eq!(insert(game, &syrup, 1000), 0);
                assert_eq!(extract(game, &lava, 999), 0);
                assert_eq!(extract(game, &lava, 1000), 1000);
                assert_eq!(at(game), block(game, "minecraft:cauldron", None));
            })
        });
    }
    #[cfg(any())]
    #[test]
    #[ignore = "daily"]
    fn a_scoped_tank_can_refuse_a_fluid() {
        use super::super::super::native_entity_refs::tests::{run_fresh_cases, with_game_at};
        use sunrise_gameplay::game::machines::native_fluids::read_tanks;
        if run_fresh_cases(
            "cut_layout::game::server_side::machines::native_fluids::tests::a_scoped_tank_can_refuse_a_fluid",
            "SUNRISE_SCOPED_FLUID_CHILD",
            &["valid"],
        ) {
            return;
        }
        use mod_api::{
            BlockUpdateFlags, FLUIDS, FluidResource, Hooks, ModEntry, Mods, Registrar,
            resource_util::move_resources,
        };
        use sunrise_level::{
            chunk::Chunk,
            containers::{ModBlockEntity, ModBlockEntityFields},
            fast_hash::FxHashMap,
        };

        struct WaterTanks;
        #[mod_api::__subscriptions]
        impl Hooks for WaterTanks {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                r.declare_fluid("chocolate");
                let kind = r
                    .block_entity_type("water_tank")
                    .blocks(&["fluid_filter_test:water_tank"])
                    .build();
                r.block_entity_capability_before_game(&FLUIDS, &kind, |_, at, _, entity, offer| {
                    // The same block entity type elsewhere keeps the game's default handler.
                    if at.pos == [2, 64, 0] {
                        offer.offer(|| {
                            Box::new(WaterTank {
                                entity,
                                reached: if at.side == Some(Direction::Down) {
                                    vec![1, 0]
                                } else {
                                    vec![0, 1]
                                },
                            })
                        });
                    }
                });
            }
            fn blocks(
                &self,
                _: &mod_api::RegistrationContext,
                event: &mut mod_api::BlocksEvent<'_>,
            ) {
                event
                    .register(mod_api::Block::new(
                        "fluid_filter_test:water_tank",
                        "minecraft:stone",
                    ))
                    .unwrap();
            }
        }
        let mods: Mods = vec![ModEntry::new("fluid_filter_test", Box::new(WaterTanks))].into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            let base =
                std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
            let root =
                sunrise_gameplay_common::game::mods::content_root(&base, &Default::default())
                    .unwrap()
                    .prepared()
                    .to_path_buf();
            with_game_at(root, |game| {
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let level = game.level("minecraft:overworld").unwrap();
                let block = game
                    .pack
                    .assets
                    .registry
                    .default_state("fluid_filter_test:water_tank")
                    .unwrap();
                let mut table = ModFluidTanks::default();
                table.tanks = vec![4000, 4000];
                table.inputs = vec![0, 1];
                table.outputs = vec![0, 1];
                table.faces = std::array::from_fn(|_| vec![1, 0]);
                game.world
                    .block_entities
                    .set_mod_fluids(FxHashMap::from_iter([(block.0, Arc::new(table))]));
                let (from, to) = ([0, 64, 0], [2, 64, 0]);
                for pos in [from, to] {
                    assert_eq!(
                        game.level_set_block(level, pos, block, BlockUpdateFlags::NEIGHBORS),
                        Ok(true)
                    );
                    game.world.block_entities.insert(
                        (pos[0], pos[1], pos[2]),
                        BlockEntity::Mod(Box::new(ModBlockEntity::new(
                            "fluid_filter_test:water_tank".into(),
                            ModBlockEntityFields::default(),
                        ))),
                    );
                }
                // The default takes a mod's fluid; a scoped answer does not affect it.
                let source = level.capability(game, &FLUIDS, from, None).unwrap();
                let chocolate = FluidResource::of(&FluidStack::new(
                    mod_api::__private::fluid_key("fluid_filter_test:chocolate").unwrap(),
                    1,
                ));
                let water = FluidResource::of(&FluidStack::new(
                    mod_api::__private::fluid_key("minecraft:water").unwrap(),
                    1,
                ));
                assert_eq!(source.owner(), "the game");
                let empty = FluidResource::of(&FluidStack::new(
                    mod_api::__private::fluid_key("minecraft:empty").unwrap(),
                    1000,
                ));
                assert_eq!(source.capacity(game, 0, Some(&empty)), 0);
                assert_eq!(
                    game.storage_transaction(|host, tx| {
                        source.insert_at(host, tx, 0, &empty, 1000)
                    }),
                    Ok(0)
                );
                assert_eq!(source.resource(game, 0), None);
                assert_eq!(mod_api::FluidIdentity::of_name("minecraft:empty"), None);
                let water_identity = mod_api::FluidIdentity::Resolved(
                    mod_api::__private::fluid_key("minecraft:water").unwrap(),
                );
                assert_eq!(
                    mod_api::FluidIdentity::of_name("minecraft:water"),
                    Some(water_identity.clone())
                );
                assert_eq!(
                    mod_api::FluidIdentity::of_name("water"),
                    Some(water_identity)
                );
                for id in ["minecraft:unavailable_native", "unavailable_native"] {
                    game.set_tank_state(from, &tanks(&[(Some(id), 1000), (None, 0)]));
                    let unresolved = source.resource(game, 0).unwrap();
                    let mod_api::FluidIdentity::Unresolved(name) = unresolved.fluid() else {
                        panic!("an unregistered saved name stays unresolved");
                    };
                    assert_eq!(name.id().as_str(), "minecraft:unavailable_native");
                    assert_eq!(source.amount(game, 0), 1000);
                }
                let handle = mod_api::FluidIdentity::of_name("othermod:fluid").unwrap();
                game.set_tank_state(from, &tanks(&[(Some("othermod:fluid"), 1000), (None, 0)]));
                let pipe_target = [4, 64, 0];
                assert_eq!(
                    game.level_set_block(level, pipe_target, block, BlockUpdateFlags::NEIGHBORS),
                    Ok(true)
                );
                game.world.block_entities.insert(
                    (4, 64, 0),
                    BlockEntity::Mod(Box::new(ModBlockEntity::new(
                        "fluid_filter_test:water_tank".into(),
                        ModBlockEntityFields::default(),
                    ))),
                );
                let target = level.capability(game, &FLUIDS, pipe_target, None).unwrap();
                assert_eq!(
                    game.storage_transaction(|host, tx| move_resources(
                        host,
                        &source,
                        &target,
                        |_| true,
                        300,
                        Some(tx)
                    )),
                    Ok(300)
                );
                assert_eq!(
                    game.tank_state(pipe_target, 2).unwrap()[0],
                    (Some("othermod:fluid".into()), 300)
                );
                let mut saved = Chunk::empty(0, 0);
                saved.set(4, 64, 0, block.0);
                saved.block_entities.push((
                    (4, 64, 0),
                    game.world.block_entities.get(&(4, 64, 0)).unwrap().clone(),
                ));
                let mut disk = sunrise_sim::save::SaveDir::new(
                    std::env::temp_dir()
                        .join(format!("sunrise-fluid-roundtrip-{}", std::process::id())),
                    sunrise_sim::save::LevelShape::OVERWORLD,
                    Some(Default::default()),
                );
                disk.mark_server_owned();
                disk.save(
                    &[Arc::new(saved)],
                    game.pack.assets.registry,
                    game.res.biomes(),
                )
                .unwrap();
                let loaded = disk
                    .load(0, 0, game.pack.assets.registry, game.res.biomes())
                    .unwrap();
                let BlockEntity::Mod(loaded) = &loaded.block_entities[0].1 else {
                    panic!("saved mod tank")
                };
                assert_eq!(
                    read_tanks(&loaded.nbt, 2)[0],
                    (Some("othermod:fluid".into()), 300)
                );
                let unresolved = source.resource(game, 0).unwrap();
                assert_eq!(*unresolved.fluid(), handle);
                assert_eq!(source.amount(game, 0), 700);
                assert_eq!(
                    game.storage_transaction(|host, tx| source.extract_at(
                        host,
                        tx,
                        0,
                        &unresolved,
                        200
                    )),
                    Ok(200)
                );
                assert_eq!(
                    game.tank_state(from, 2).unwrap()[0],
                    (Some("othermod:fluid".into()), 500)
                );
                game.set_tank_state(from, &tanks(&[(Some("water"), 1000), (None, 0)]));
                assert_eq!(source.resource(game, 0), Some(water.clone()));
                assert_eq!(source.amount(game, 0), 1000);
                assert_eq!(
                    game.storage_transaction(|host, tx| {
                        source.extract_at(host, tx, 0, &water, 1000)
                    }),
                    Ok(1000)
                );
                assert_eq!(source.resource(game, 0), None);
                assert!(source.is_valid(game, 0, &chocolate));
                assert_eq!(
                    game.storage_transaction(|host, tx| {
                        source.insert_at(host, tx, 0, &chocolate, 1000)
                    }),
                    Ok(1000)
                );
                assert_eq!(source.resource(game, 0), Some(chocolate.clone()));
                assert_eq!(source.amount(game, 0), 1000);

                // A refusal must protect both a previously filled tank and an empty one.
                game.set_tank_state(
                    to,
                    &tanks(&[(Some("fluid_filter_test:chocolate"), 1000), (None, 0)]),
                );
                let target = level.capability(game, &FLUIDS, to, None).unwrap();
                assert_eq!(target.owner(), "fluid_filter_test");
                for index in 0..2 {
                    assert_eq!(target.capacity(game, index, Some(&chocolate)), 4000);
                    assert!(!target.is_valid(game, index, &chocolate));
                    assert_eq!(
                        game.storage_transaction(|host, tx| {
                            target.insert_at(host, tx, index, &chocolate, 500)
                        }),
                        Ok(0)
                    );
                }
                assert_eq!(target.amount(game, 0), 1000);
                assert_eq!(target.resource(game, 0), Some(chocolate.clone()));
                assert_eq!(target.amount(game, 1), 0);
                assert_eq!(target.resource(game, 1), None);
                assert_eq!(
                    game.storage_transaction(|host, tx| move_resources(
                        host,
                        &source,
                        &target,
                        |_| true,
                        500,
                        Some(tx)
                    )),
                    Ok(0)
                );
                assert_eq!(source.amount(game, 0), 1000);
                assert_eq!(target.amount(game, 0), 1000);
                assert_eq!(target.amount(game, 1), 0);

                // Water is valid but cannot replace another fluid; it can fill the empty tank.
                assert!(target.is_valid(game, 0, &water));
                assert_eq!(
                    game.storage_transaction(|host, tx| {
                        target.insert_at(host, tx, 0, &water, 500)
                    }),
                    Ok(0)
                );
                assert!(target.is_valid(game, 1, &water));
                assert_eq!(
                    game.storage_transaction(|host, tx| {
                        target.insert_at(host, tx, 1, &water, 500)
                    }),
                    Ok(500)
                );
                assert_eq!(target.resource(game, 1), Some(water.clone()));
                assert_eq!(target.amount(game, 1), 500);
                let sided = level
                    .capability(game, &FLUIDS, to, Some(Direction::Down))
                    .unwrap();
                assert_eq!(sided.resource(game, 0), Some(water.clone()));
                assert_eq!(sided.amount(game, 0), 500);
                // A drain does not consult acceptance, so an old refused fluid is recoverable.
                assert_eq!(
                    game.storage_transaction(|host, tx| {
                        target.extract_at(host, tx, 0, &chocolate, 100)
                    }),
                    Ok(100)
                );
                assert_eq!(target.amount(game, 0), 900);
                assert_eq!(
                    game.storage_transaction(|host, tx| {
                        source.insert_at(host, tx, 1, &water, 1000)
                    }),
                    Ok(1000)
                );
                assert_eq!(
                    game.storage_transaction(|host, tx| move_resources(
                        host,
                        &source,
                        &target,
                        |resource| resource == &water,
                        200,
                        Some(tx)
                    )),
                    Ok(200)
                );
                assert_eq!(source.amount(game, 1), 800);
                assert_eq!(target.amount(game, 1), 700);
                let abort: Result<(), ()> = game.storage_transaction(|host, tx| {
                    assert_eq!(target.insert_at(host, tx, 1, &water, 200), Ok(200));
                    Err(())
                });
                assert!(abort.is_err());
                assert_eq!(target.amount(game, 1), 700);
                assert_eq!(
                    game.tank_state(from, 2),
                    Some(tanks(&[
                        (Some("fluid_filter_test:chocolate"), 1000),
                        (Some("minecraft:water"), 800)
                    ]))
                );
                assert_eq!(
                    game.tank_state(to, 2),
                    Some(tanks(&[
                        (Some("fluid_filter_test:chocolate"), 900),
                        (Some("minecraft:water"), 700)
                    ]))
                );
            })
        });
    }
}
