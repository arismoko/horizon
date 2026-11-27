#![cfg(test)]
#[cfg(test)]
mod tests {
    use mod_api::{
        BlockPart, Hooks, MirrorEvent, ModEntry, Mods, RotateEvent, StructureMirror,
        StructureRotation, WorldgenHost,
    };
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex, PoisonError};
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    use sunrise_gameplay_common::game::api_values::block_state::Index;
    use sunrise_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs};
    use sunrise_world::registry::BlockRegistry;
    use sunrise_world::worldgen::structure::Mirror;
    use {
        crate::game::native_rotate::*,
        sunrise_gameplay_common::game::native_rotate::{
            BlockState, EngineState, Rotation, mirror, native_worldgen_parts, rotate,
        },
    };

    /// The states the furnace's part was asked with.
    static ASKED: Mutex<Vec<BlockState>> = Mutex::new(Vec::new());

    /// A furnace that always faces north once placed.
    struct North;
    #[mod_api::sunrise_part]
    impl BlockPart for North {
        fn rotate(&self, context: &dyn WorldgenHost, event: &mut RotateEvent<'_>) {
            ASKED
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(event.state);
            *event.rotated = context
                .state_with_property(event.state, "facing", "north")
                .unwrap();
        }
    }
    /// The state of another block the smoker's part answers.
    static OTHER: Mutex<Option<BlockState>> = Mutex::new(None);
    /// A smoker's part answering a state of another block.
    struct Stone;
    #[mod_api::sunrise_part]
    impl BlockPart for Stone {
        fn rotate(&self, _: &dyn WorldgenHost, event: &mut RotateEvent<'_>) {
            *event.rotated = OTHER
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .unwrap();
        }
    }
    /// A lectern's part that panics.
    struct Panics;
    #[mod_api::sunrise_part]
    impl BlockPart for Panics {
        fn rotate(&self, _: &dyn WorldgenHost, _: &mut RotateEvent<'_>) {
            panic!("a part that panics");
        }
    }
    /// What the dispenser's part read: its `facing`, and a key its block does not have.
    static READ: Mutex<Vec<(Option<String>, Option<String>)>> = Mutex::new(Vec::new());
    /// A dispenser turned as vanilla's `rotate` overrides turn one: read `facing`, turn it, set it.
    struct ReadsFacing;
    #[mod_api::sunrise_part]
    impl BlockPart for ReadsFacing {
        fn rotate(&self, context: &dyn WorldgenHost, event: &mut RotateEvent<'_>) {
            let facing = context.state_property(event.state, "facing");
            READ.lock().unwrap_or_else(PoisonError::into_inner).push((
                facing.clone(),
                context.state_property(event.state, "no_such_property"),
            ));
            let turned = match (event.rotation, facing.as_deref()) {
                (StructureRotation::Clockwise90, Some("north")) => "east",
                (StructureRotation::Clockwise90, Some("east")) => "south",
                (StructureRotation::Clockwise90, Some("south")) => "west",
                (StructureRotation::Clockwise90, Some("west")) => "north",
                _ => return,
            };
            *event.rotated = context
                .state_with_property(event.state, "facing", turned)
                .unwrap();
        }
    }
    /// The states the observer's part was asked to mirror, with the mirror.
    static MIRRORED: Mutex<Vec<(BlockState, StructureMirror)>> = Mutex::new(Vec::new());
    /// An observer mirrored as a block overriding `BlockBehaviour.mirror` might: front to back,
    /// it faces up whatever it faced, so its answer differs from the game's table.
    struct MirrorsUp;
    #[mod_api::sunrise_part]
    impl BlockPart for MirrorsUp {
        fn mirror(&self, context: &dyn WorldgenHost, event: &mut MirrorEvent<'_>) {
            MIRRORED
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((event.state, event.mirror));
            if event.mirror == StructureMirror::FrontBack {
                *event.mirrored = context
                    .state_with_property(event.state, "facing", "up")
                    .unwrap();
            }
        }
    }
    /// A comparator's part answering a state of another block for a mirror.
    struct MirrorsToStone;
    #[mod_api::sunrise_part]
    impl BlockPart for MirrorsToStone {
        fn mirror(&self, _: &dyn WorldgenHost, event: &mut MirrorEvent<'_>) {
            *event.mirrored = OTHER
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .unwrap();
        }
    }
    struct Turns;
    #[mod_api::__subscriptions]
    impl Hooks for Turns {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let furnace = r.block_key("minecraft:furnace");
            r.block_part(furnace, North);
            let smoker = r.block_key("minecraft:smoker");
            r.block_part(smoker, Stone);
            let lectern = r.block_key("minecraft:lectern");
            r.block_part(lectern, Panics);
            let dispenser = r.block_key("minecraft:dispenser");
            r.block_part(dispenser, ReadsFacing);
            let observer = r.block_key("minecraft:observer");
            r.block_part(observer, MirrorsUp);
            let comparator = r.block_key("minecraft:comparator");
            r.block_part(comparator, MirrorsToStone);
        }
    }

    /// `name` facing `facing`.
    fn facing(registry: &BlockRegistry, name: &str, facing: &str) -> EngineState {
        let default = registry.default_state(name).unwrap();
        sunrise_gameplay_common::game::native_hooks::with_property(
            registry, default, "facing", facing,
        )
        .unwrap()
    }

    /// Runs `test` on a server game that resolved and published `Turns`' parts and a worldgen
    /// context, with `place(name)`: `/place template … clockwise_90 left_right` of `name` facing
    /// north (LEFT_RIGHT turns north to south, then a clockwise quarter turn makes it west), as
    /// the parts place it and as the game's own table does.
    fn with_published_parts(
        test: impl FnOnce(&BlockRegistry, &Index, &dyn Fn(&str) -> (EngineState, EngineState)),
    ) {
        let _publishing = native_worldgen_parts::PUBLISHING
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mods: Mods = vec![ModEntry::new("turns", Box::new(Turns))].into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let registry = game.pack.assets.registry;
                let transforms = game.state_transforms().unwrap();
                // A server resolves its parts against its tags and publishes them.
                game.options.dedicated = Some(Vec::new());
                game.resolve_parts().unwrap();
                let index = native_worldgen_parts::index().unwrap();
                *OTHER.lock().unwrap_or_else(PoisonError::into_inner) = index
                    .to_api(registry.default_state("minecraft:stone").unwrap())
                    .ok();
                let root = std::env::temp_dir().join("sunrise-rotate-no-settings");
                let load = LoadContext::new(
                    DataLoadOrigin::ServerDataLoad,
                    Arc::new(LoadInputs::new(BTreeMap::new(), root)),
                    [],
                );
                sunrise_world::native_worldgen_hooks::publish_context(Arc::new(
                    load.worldgen_context([]),
                ));
                let place = |name: &str| {
                    let mirrored =
                        transforms.mirror(facing(registry, name, "north"), Mirror::LeftRight);
                    let table = transforms.rotate(mirrored, Rotation::Clockwise90);
                    (
                        rotate(mirrored, [4, 64, 2], Rotation::Clockwise90, table),
                        table,
                    )
                };
                test(registry, &index, &place);
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_part_turns_its_block_as_a_template_places_it() {
        with_published_parts(|registry, index, place| {
            let (blast, table) = place("minecraft:blast_furnace");
            assert_eq!(blast, facing(registry, "minecraft:blast_furnace", "west"));
            assert_eq!(blast, table);
            let (furnace, _) = place("minecraft:furnace");
            assert_eq!(furnace, facing(registry, "minecraft:furnace", "north"));
            let asked = ASKED.lock().unwrap_or_else(PoisonError::into_inner).clone();
            let south = index
                .to_api(facing(registry, "minecraft:furnace", "south"))
                .unwrap();
            assert_eq!(asked, [south]);
            // A state of another block, and a panicking part, keep the game's state.
            let (smoker, table) = place("minecraft:smoker");
            assert_eq!(
                (smoker, table),
                (table, facing(registry, "minecraft:smoker", "west"))
            );
            let (lectern, table) = place("minecraft:lectern");
            assert_eq!(
                (lectern, table),
                (table, facing(registry, "minecraft:lectern", "west"))
            );
        });
    }

    /// A template mirrored front to back places an observer facing east as its part answers
    /// (`BlockBehaviour.mirror` overridden), where the game's table flips it to west; a block with
    /// no part, and a part answering a state of another block, keep the table's state.
    #[test]
    #[ignore = "daily"]
    fn a_part_mirrors_its_block_as_a_template_places_it() {
        with_published_parts(|registry, index, _| {
            MIRRORED
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clear();
            let pos = [4, 64, 2];
            let observer = facing(registry, "minecraft:observer", "east");
            let west = facing(registry, "minecraft:observer", "west");
            assert_eq!(
                mirror(observer, pos, Mirror::FrontBack, west),
                facing(registry, "minecraft:observer", "up")
            );
            // Left to right, the part keeps the game's answer.
            assert_eq!(mirror(observer, pos, Mirror::LeftRight, observer), observer);
            let asked = index.to_api(observer).unwrap();
            assert_eq!(
                *MIRRORED.lock().unwrap_or_else(PoisonError::into_inner),
                [
                    (asked, StructureMirror::FrontBack),
                    (asked, StructureMirror::LeftRight),
                ]
            );
            for block in ["minecraft:blast_furnace", "minecraft:comparator"] {
                let east = facing(registry, block, "east");
                let west = facing(registry, block, "west");
                assert_eq!(mirror(east, pos, Mirror::FrontBack, west), west, "{block}");
            }
        });
    }

    /// `WorldgenHost::state_property` reads the state a part is asked with, so a part turns its
    /// block as vanilla's `rotate` overrides do (`state.getValue(FACING)`) and lands where the
    /// game's own table puts it; a key the block does not have reads None.
    #[test]
    #[ignore = "daily"]
    fn a_part_reads_the_property_it_turns() {
        with_published_parts(|registry, _, place| {
            READ.lock().unwrap_or_else(PoisonError::into_inner).clear();
            let (dispenser, table) = place("minecraft:dispenser");
            assert_eq!(dispenser, facing(registry, "minecraft:dispenser", "west"));
            assert_eq!(dispenser, table);
            assert_eq!(
                *READ.lock().unwrap_or_else(PoisonError::into_inner),
                [(Some("south".to_owned()), None)]
            );
        });
    }
}

#[cfg(test)]
#[path = "native_rotate/jigsaw_tests.rs"]
mod jigsaw_tests;
use sunrise_gameplay_common::game::tables::WorldTablesExt as _;
