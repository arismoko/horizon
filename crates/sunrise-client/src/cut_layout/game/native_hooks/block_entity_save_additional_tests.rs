#![cfg(test)]

#[cfg(test)]
pub(crate) mod tests {
    use mod_api::{BlockEntityPart, BlockEntitySaveAdditionalEvent, Hooks, LevelHost, ModEntry};
    use std::sync::atomic::{AtomicI32, Ordering};
    use std::sync::{Arc, Mutex};
    use sunrise_core::{
        nbt::{Compound, JavaString, Limits, Tag},
        position::ChunkPos,
    };
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    use sunrise_gameplay_common::game::block_items::BlockItemsGameExt1;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_level::{
        chunk::{Chunk, LevelShape},
        containers::{BlockEntity, ModBlockEntity, ModBlockEntityFields, SHAPES_KEY},
    };
    use sunrise_mod_runtime::attachments::ATTACHMENTS_KEY;
    use sunrise_sim::save::SaveDir;
    use sunrise_world::region::RegionFile;

    thread_local! {
        pub(in crate::game) static SAVED_AT: std::cell::RefCell<Vec<([i32; 3], mod_api::LevelRef)>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    pub(crate) struct Counter(pub(crate) Arc<AtomicI32>);
    #[mod_api::sunrise_part]
    impl mod_api::BlockPart for Counter {}
    #[mod_api::sunrise_part]
    impl BlockEntityPart for Counter {
        fn block_entity_save_additional(
            &self,
            _: &dyn LevelHost,
            event: &mut BlockEntitySaveAdditionalEvent<'_>,
        ) {
            SAVED_AT.with_borrow_mut(|seen| seen.push((event.pos, event.level)));
            let serial = self.0.fetch_add(1, Ordering::Relaxed);
            match event.tag.get(&"count".into()) {
                Some(Tag::Int(-3)) => {
                    event.tag.insert(
                        "values".into(),
                        Tag::List(vec![Tag::String(JavaString(vec![0; 32_768]))]),
                    );
                }
                Some(Tag::Int(-2)) => {
                    event.tag.insert(JavaString(vec![0; 32_768]), Tag::Int(-99));
                }
                Some(Tag::Int(-1)) => {
                    event.tag.insert("invalid".into(), Tag::End);
                }
                Some(Tag::Int(0)) => {
                    event.tag.remove(&"count".into());
                }
                Some(Tag::Int(7)) => {
                    event.tag.insert("count".into(), Tag::Int(serial));
                }
                other => panic!("unexpected payload: {other:?}"),
            }
            assert!(!event.tag.contains_key(&ATTACHMENTS_KEY.into()));
            event
                .tag
                .insert("id".into(), Tag::String("test:wrong".into()));
            event.tag.insert(SHAPES_KEY.into(), Tag::Byte(1));
            event.tag.insert(ATTACHMENTS_KEY.into(), Tag::Byte(1));
        }
    }
    pub(crate) struct Mod(pub(crate) Arc<AtomicI32>);
    #[mod_api::__subscriptions]
    impl Hooks for Mod {
        fn register(
            &self,
            registrar: &mut dyn mod_api::Registrar,
            _: &mod_api::RegistrationContext,
        ) {
            let counter = registrar.block_entity_type("counter").build();
            registrar
                .block_entity_part(&counter, Counter(self.0.clone()))
                .unwrap();
            registrar.block_entity_type("silent").build();
        }
    }

    #[test]
    #[ignore = "daily"]
    fn command_and_unload_capture_the_live_payload_without_mutating_storage() {
        let count = Arc::new(AtomicI32::new(11));
        mod_platform::testing::with_mods(
            vec![ModEntry::new("test", Box::new(Mod(count.clone())))].into(),
            || {
                with_game(|game| {
                    mod_api::__private::resolve_block_entity_types(
                        ["test:counter", "test:silent"],
                        |_| Some(()),
                    )
                    .unwrap();
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                    let pos = (1, 64, 1);
                    let silent = (2, 64, 1);
                    let invalid = (3, 64, 1);
                    let erased = (4, 64, 1);
                    for (pos, kind, saved) in [
                        (pos, "test:counter", 7),
                        (silent, "test:silent", 7),
                        (invalid, "test:counter", -1),
                        (erased, "test:counter", 0),
                    ] {
                        let state = game
                            .pack
                            .assets
                            .registry
                            .default_state("minecraft:dirt")
                            .unwrap();
                        game.world.set(pos.0, pos.1, pos.2, state).unwrap();
                        game.world.block_entities.insert(
                            pos,
                            BlockEntity::Mod(Box::new(ModBlockEntity::new(
                                kind.into(),
                                ModBlockEntityFields {
                                    nbt: Compound::from([
                                        ("count".into(), Tag::Int(saved)),
                                        (ATTACHMENTS_KEY.into(), Tag::Byte(7)),
                                    ]),
                                    ..Default::default()
                                },
                            ))),
                        );
                    }
                    let tag = game.block_entity_nbt([1, 64, 1]).unwrap();
                    assert_eq!(tag.get("count"), Some(&Tag::Int(11)));
                    assert_eq!(tag.get("id"), Some(&Tag::String("test:counter".into())));
                    assert_eq!(tag.get(ATTACHMENTS_KEY), Some(&Tag::Byte(7)));
                    assert_eq!(tag.get(SHAPES_KEY), None);

                    // Ctrl-pick must not copy the removed field in the base clone item.
                    let picked = game.picked_stack([4, 64, 1], true).unwrap();
                    assert!(
                        picked
                            .data()
                            .and_then(|d| d.other("minecraft:block_entity_data"))
                            .is_none_or(|tag| tag.get("count").is_none())
                    );
                    assert_eq!(count.load(Ordering::Relaxed), 13);

                    let detached = game.world.chunks.remove(&(0, 0)).unwrap();
                    let capture = game.with_entities([&detached]);
                    let fields = |entity: &BlockEntity| {
                        let BlockEntity::Mod(entity) = entity else {
                            panic!("expected a mod block entity")
                        };
                        entity.nbt.get(&"count".into()).cloned()
                    };
                    let snapshots = &capture[0].block_entities;
                    let stored = fields(&snapshots.iter().find(|(p, _)| *p == pos).unwrap().1);
                    assert_eq!(stored, Some(Tag::Int(7)));
                    assert_eq!(
                        fields(&snapshots.iter().find(|(p, _)| *p == silent).unwrap().1),
                        Some(Tag::Int(7))
                    );
                    assert_eq!(
                        fields(game.world.block_entities.get(&pos).unwrap()),
                        Some(Tag::Int(7))
                    );
                    assert_eq!(count.load(Ordering::Relaxed), 16);
                    // The unload capture names the entity's place after its chunk left the world.
                    let level = game.level("minecraft:overworld").unwrap();
                    SAVED_AT.with_borrow(|seen| {
                        assert!(seen.len() >= 3);
                        assert!(seen.contains(&([1, 64, 1], level)));
                        assert!(seen.iter().all(|(_, at)| *at == level));
                    });

                    // A malformed individual payload is refused at the callback seam.
                    // Other entities' valid edits still reach the actual region record.
                    let scratch = tempfile::tempdir().unwrap();
                    let save = SaveDir::new(
                        scratch.path().to_owned(),
                        LevelShape::OVERWORLD,
                        Some(Arc::new(Default::default())),
                    );
                    save.save_later(&capture, game.pack.assets.registry, game.res.biomes())
                        .unwrap();
                    save.flush().unwrap();
                    let saved = RegionFile::open(scratch.path().join("region/r.0.0.mca"))
                        .unwrap()
                        .read(ChunkPos::new(0, 0))
                        .unwrap()
                        .unwrap()
                        .nbt(Limits::saved())
                        .unwrap();
                    let Some(Tag::List(entities)) = saved.tag.get("block_entities") else {
                        panic!("expected saved block entities");
                    };
                    let at = |x| {
                        entities
                            .iter()
                            .find(|tag| tag.get("x") == Some(&Tag::Int(x)))
                            .unwrap()
                    };
                    assert!(matches!(at(1).get("count"), Some(Tag::Int(13..=15))));
                    assert_eq!(at(3).get("count"), Some(&Tag::Int(-1)));
                    assert_eq!(at(3).get("invalid"), None);
                    assert_eq!(at(4).get("count"), None);
                })
            },
        );
    }

    pub(crate) struct SaveItemsPart;
    #[mod_api::sunrise_part]
    impl mod_api::BlockPart for SaveItemsPart {}
    #[mod_api::sunrise_part]
    impl BlockEntityPart for SaveItemsPart {
        fn block_entity_save_additional(
            &self,
            _: &dyn LevelHost,
            event: &mut BlockEntitySaveAdditionalEvent<'_>,
        ) {
            let key = "Items".into();
            if event.pos[0] <= 2 {
                assert_eq!(event.tag.get(&key), Some(&saved_stack("minecraft:stone")));
            } else {
                assert!(
                    !event.tag.contains_key(&key),
                    "modeled slots are not listener input"
                );
            }
            if event.pos[0] == 2 {
                event.tag.remove(&key);
            } else {
                event.tag.insert(key, saved_stack("minecraft:dirt"));
            }
        }
    }

    pub(crate) fn saved_stack(id: &str) -> Tag {
        Tag::List(vec![Tag::Compound(Compound::from([
            ("Slot".into(), Tag::Byte(0)),
            ("id".into(), Tag::String(id.into())),
            ("count".into(), Tag::Int(1)),
        ]))])
    }

    pub(crate) struct SaveItemsMod;
    #[mod_api::__subscriptions]
    impl Hooks for SaveItemsMod {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let holder = r
                .block_entity_type("holder")
                .blocks(&["save_items:slotless", "save_items:container"])
                .build();
            r.block_entity_part(&holder, SaveItemsPart).unwrap();
            let container = r.block_key("save_items:container");
            r.block_container(
                &container,
                mod_api::ModContainer {
                    slots: 1,
                    ..Default::default()
                },
            );
        }

        fn blocks(&self, _: &mod_api::RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
            for id in ["save_items:slotless", "save_items:container"] {
                event
                    .register(mod_api::Block::new(id, "minecraft:stone").with_item(false))
                    .unwrap();
            }
        }
    }

    pub(crate) struct BreakObserver(pub(crate) Arc<Mutex<Vec<(&'static str, Option<Tag>)>>>);

    #[mod_api::__subscriptions]
    impl Hooks for BreakObserver {
        fn player_destroyed_block(
            &self,
            _: mod_api::HostSide<'_>,
            event: &mut mod_api::PlayerDestroyedBlockEvent,
        ) {
            let stored = event
                .block_entity
                .as_ref()
                .and_then(|tag| tag.get(&"count".into()))
                .cloned();
            self.0.lock().unwrap().push(("removed", stored));
        }
        fn block_drops(
            &self,
            _: &dyn LevelHost,
            event: &mut mod_api::BlockDropsEvent<'_>,
        ) -> mod_api::Flow {
            assert!(
                event.block_entity.is_none(),
                "a player break has already removed the block entity from the world"
            );
            let stored = event
                .block_entity_data
                .and_then(|tag| tag.get(&"count".into()).cloned());
            self.0.lock().unwrap().push(("drops", stored));
            mod_api::Flow::Continue
        }
    }
}
