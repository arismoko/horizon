//! Structure templates placed for server mods: `/place template`'s path, for a
//! template the game has by id or for vanilla structure NBT a mod carries.
use super::{
    native_structure_values::{engine_mirror, engine_rotation, template_id},
    structure_blocks::load_settings,
};
use mod_api::{
    LevelRef, StructureMirror, StructureRotation, StructureTemplateError, StructureTemplateSource,
};
use sunrise_core::nbt::Limits;
use sunrise_core::position::BlockPos;
use sunrise_world::compression::Compression;
use sunrise_world::level::TemplateSettings;
use sunrise_world::registry::BlockRegistry;
use sunrise_world::worldgen::structure::{StateTransforms, StructureTemplate};

/// Native mods use this; nothing in Sunrise does.
impl crate::game::ServerGame<'_> {
    /// For a server mod: template `source` placed at `at` in this server level
    /// (active or parked) the way `/place template` and a structure block's load
    /// place one, with `rotation`, `mirror` and `pivot`, the level position the
    /// turn and the mirroring go around (`at` is what `/place template` uses).
    /// Blocks, block-entity data and the template's own entities come through
    /// vanilla's placement.
    pub(in crate::game) fn level_place_template(
        &mut self,
        level: LevelRef,
        source: &StructureTemplateSource,
        at: [i32; 3],
        pivot: [i32; 3],
        rotation: StructureRotation,
        mirror: StructureMirror,
    ) -> Result<(), StructureTemplateError> {
        let rotation =
            engine_rotation(rotation).ok_or(StructureTemplateError::UnsupportedTransform)?;
        let mirror = engine_mirror(mirror).ok_or(StructureTemplateError::UnsupportedTransform)?;
        self.with_server_level_mut(
            level,
            StructureTemplateError::Unavailable,
            StructureTemplateError::Stale,
            Some(StructureTemplateError::InTransaction),
            |game| {
                let transforms = game
                    .state_transforms()
                    .ok_or(StructureTemplateError::Unavailable)?;
                let template = match source {
                    StructureTemplateSource::Id(id) => {
                        let id = template_id(id).ok_or(StructureTemplateError::InvalidId)?;
                        let entry = game
                            .template_entry(&id)
                            .ok_or(StructureTemplateError::NotFound)?;
                        StructureTemplate::from_nbt(
                            &entry.tag,
                            game.common.pack.assets.registry,
                            transforms,
                        )
                    }
                    StructureTemplateSource::Nbt(bytes) => {
                        template_from_nbt(bytes, game.common.pack.assets.registry, transforms)?
                    }
                    _ => return Err(StructureTemplateError::UnsupportedTransform),
                };
                // Vanilla turns the template about its own origin for `pivot == at`,
                // and `PlaceSettings.pivot` counts from the template's corner.
                let local_pivot = [0, 1, 2].map(|axis| pivot[axis].wrapping_sub(at[axis]));
                if template.palettes().is_empty() || template.size.iter().any(|side| *side < 1) {
                    return Err(StructureTemplateError::EmptyTemplate);
                }
                // `PlaceCommand.checkLoaded`, over the box the template covers once
                // turned and mirrored: the chunks it writes into all have to be here.
                let area = template.bounding_box(at, rotation, local_pivot, mirror);
                for cx in area.min[0] >> 4..=area.max[0] >> 4 {
                    for cz in area.min[2] >> 4..=area.max[2] >> 4 {
                        if !game.common.world.chunks.contains_key(&(cx, cz)) {
                            return Err(StructureTemplateError::UnloadedChunk { chunk: [cx, cz] });
                        }
                    }
                }
                if game
                    .common
                    .res
                    .behaviours()
                    .live_features()
                    .and_then(|features| features.structure_resources())
                    .is_none()
                {
                    return Err(StructureTemplateError::Unavailable);
                }
                // `/place template` without `integrity`, `seed` or `strict`: everything
                // places, the loot seeds come from the clock, and neighbours are not
                // told.
                let settings = load_settings(mirror, rotation, 1.0, 0, false);
                let settings = TemplateSettings {
                    pivot: local_pivot,
                    ..settings
                };
                if !game.place_template_object(
                    &template,
                    BlockPos::new(at[0], at[1], at[2]),
                    settings,
                    mirror,
                    rotation,
                ) {
                    return Err(StructureTemplateError::EmptyTemplate);
                }
                Ok(())
            },
        )
    }
}

/// Vanilla's `StructureTemplate.load` on bytes a mod supplied: a `structure/<namespace>/<path>.nbt`
/// as the game saves one (gzipped) or the same NBT plain, read under vanilla's limits for a template
/// file. `Err` says which of the two failed.
fn template_from_nbt(
    bytes: &[u8],
    registry: &BlockRegistry,
    transforms: &StateTransforms,
) -> Result<StructureTemplate, StructureTemplateError> {
    let limits = Limits::templates();
    let raw = if bytes.starts_with(&[0x1f, 0x8b]) {
        Compression::Gzip
            .decode(bytes, limits.max_bytes)
            .map_err(|_| StructureTemplateError::InvalidNbt)?
    } else {
        bytes.to_vec()
    };
    // `NbtIo.readCompressed` refuses a file whose root is not a compound, where
    // the streaming read answers an empty template instead.
    if raw.first() != Some(&10) {
        return Err(StructureTemplateError::InvalidNbt);
    }
    StructureTemplate::from_bytes(&raw, limits, registry, transforms)
        .map_err(|_| StructureTemplateError::InvalidNbt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_core::nbt::{self, Tag};
    use sunrise_core::version::DATA_VERSION;
    use sunrise_world::worldgen::structure::Rotation;

    /// Two states, `minecraft:air` and `minecraft:stone`: enough for a template's palette to resolve.
    fn registry() -> BlockRegistry {
        let state = |name: &str| Tag::compound([("id", Tag::String(name.into()))]);
        BlockRegistry::from_nbt(&Tag::compound([
            ("DataVersion", Tag::Int(DATA_VERSION)),
            (
                "states",
                Tag::List(vec![state("minecraft:air"), state("minecraft:stone")]),
            ),
            (
                "defaults",
                Tag::compound([
                    ("minecraft:air", Tag::Int(0)),
                    ("minecraft:stone", Tag::Int(1)),
                ]),
            ),
        ]))
        .unwrap()
    }

    fn transforms() -> StateTransforms {
        StateTransforms::from_json(
            r#"{"rotate":[[0,0,0],[1,1,1]],"mirror":[[0,0],[1,1]],"full_block":[false,true],
                "loot_containers":[],"block_entities":{}}"#,
            2,
        )
        .unwrap()
    }

    /// A structure file as the game writes one: the root compound named `""`, a 2x1x1 box of stone.
    fn structure_nbt() -> Vec<u8> {
        let tag = Tag::compound([
            (
                "size",
                Tag::List(vec![Tag::Int(2), Tag::Int(1), Tag::Int(1)]),
            ),
            (
                "palettes",
                Tag::List(vec![Tag::List(vec![Tag::compound([(
                    "Name",
                    Tag::String("minecraft:stone".into()),
                )])])]),
            ),
            (
                "blocks",
                Tag::List(vec![Tag::compound([
                    (
                        "pos",
                        Tag::List(vec![Tag::Int(0), Tag::Int(0), Tag::Int(0)]),
                    ),
                    ("state", Tag::Int(0)),
                ])]),
            ),
        ]);
        let named = nbt::NamedTag {
            name: "".into(),
            tag,
        };
        nbt::write_named(&named, Limits::templates()).unwrap()
    }

    #[test]
    fn a_gzipped_and_a_plain_structure_file_read_the_same_template() {
        let plain = structure_nbt();
        let gzipped = Compression::Gzip.encode(&plain).unwrap();
        let registry = registry();
        let transforms = transforms();
        let from_plain = template_from_nbt(&plain, &registry, &transforms).unwrap();
        let from_gzipped = template_from_nbt(&gzipped, &registry, &transforms).unwrap();
        assert_eq!(from_plain.size, [2, 1, 1], "the template's own size");
        assert_eq!(from_plain.size(Rotation::Clockwise90), [1, 1, 2]);
        assert_eq!(from_gzipped.size, [2, 1, 1]);
        assert_eq!(from_gzipped.palettes().len(), 1);
        assert_eq!(from_gzipped.palettes()[0].blocks().len(), 1);
        assert_eq!(
            from_gzipped.palettes()[0].blocks()[0].state,
            sunrise_world::registry::BlockStateId(1),
            "the palette entry resolved to `minecraft:stone`"
        );
    }

    #[test]
    fn nbt_that_is_not_a_structure_is_refused_by_name() {
        let registry = registry();
        let transforms = transforms();
        // Not gzip, not a compound: the read fails.
        assert_eq!(
            template_from_nbt(b"not nbt at all", &registry, &transforms).unwrap_err(),
            StructureTemplateError::InvalidNbt
        );
        // An empty file is no template either.
        assert_eq!(
            template_from_nbt(b"", &registry, &transforms).unwrap_err(),
            StructureTemplateError::InvalidNbt
        );
        // Gzip over bytes that are not NBT: the decompression succeeds and the
        // read is what refuses.
        let gzipped = Compression::Gzip.encode(b"\x00\x00junk").unwrap();
        assert_eq!(
            template_from_nbt(&gzipped, &registry, &transforms).unwrap_err(),
            StructureTemplateError::InvalidNbt
        );
        // Gzip that is cut short: decompression itself refuses.
        let truncated = &Compression::Gzip.encode(&structure_nbt()).unwrap()[..12];
        assert_eq!(
            template_from_nbt(truncated, &registry, &transforms).unwrap_err(),
            StructureTemplateError::InvalidNbt
        );
    }

    /// A structure file whose "extra" entry is `depth` lists around an empty one, written by hand
    /// because the game's NBT writer refuses to produce one past the read cap.
    fn nested_lists_file(depth: usize) -> Vec<u8> {
        let mut out = vec![10u8, 0, 0]; // a compound root named ""
        out.extend_from_slice(&[9, 0, 5]); // a list, key "extra"
        out.extend_from_slice(b"extra");
        for _ in 0..depth {
            out.extend_from_slice(&[9, 0, 0, 0, 1]); // one element, itself a list
        }
        out.extend_from_slice(&[3, 0, 0, 0, 0]); // the innermost list holds no int
        out.push(0); // the root compound's end
        out
    }

    #[test]
    fn nbt_past_vanillas_depth_limit_is_refused_by_name() {
        let registry = registry();
        let transforms = transforms();
        // 512 lists inside the root compound: the deepest sits at the read cap.
        assert_eq!(
            template_from_nbt(&nested_lists_file(512), &registry, &transforms).unwrap_err(),
            StructureTemplateError::InvalidNbt
        );
        // The same file, well inside the cap, reads: the refusal is the cap.
        assert!(template_from_nbt(&nested_lists_file(64), &registry, &transforms).is_ok());
    }

    /// A structure file as a mod would carry one: a stone step, a chest with data
    /// beside it, and a pig, in a 2x1x1 box.
    fn carried_structure_nbt() -> Vec<u8> {
        let pos = |p: [i32; 3]| Tag::List(p.iter().map(|v| Tag::Int(*v)).collect());
        let named_state = |name: &str| Tag::compound([("Name", Tag::String(name.into()))]);
        let mut chest = sunrise_core::nbt::Compound::new();
        chest.insert("id".into(), Tag::String("minecraft:chest".into()));
        chest.insert("Custom".into(), Tag::String("kept".into()));
        let double = |v: f64| Tag::Double(v.to_bits());
        let tag = Tag::compound([
            (
                "size",
                Tag::List(vec![Tag::Int(2), Tag::Int(1), Tag::Int(1)]),
            ),
            (
                "palettes",
                Tag::List(vec![Tag::List(vec![
                    named_state("minecraft:stone"),
                    named_state("minecraft:chest"),
                ])]),
            ),
            (
                "blocks",
                Tag::List(vec![
                    Tag::compound([("pos", pos([0, 0, 0])), ("state", Tag::Int(0))]),
                    Tag::compound([
                        ("pos", pos([1, 0, 0])),
                        ("state", Tag::Int(1)),
                        ("nbt", Tag::Compound(chest)),
                    ]),
                ]),
            ),
            (
                "entities",
                Tag::List(vec![Tag::compound([
                    (
                        "pos",
                        Tag::List(vec![double(0.5), double(0.0), double(0.5)]),
                    ),
                    (
                        "blockPos",
                        Tag::List(vec![Tag::Int(0), Tag::Int(0), Tag::Int(0)]),
                    ),
                    (
                        "nbt",
                        Tag::Compound(sunrise_core::nbt::Compound::from([(
                            "id".into(),
                            Tag::String("minecraft:pig".into()),
                        )])),
                    ),
                ])]),
            ),
        ]);
        let named = nbt::NamedTag {
            name: "".into(),
            tag,
        };
        nbt::write_named(&named, Limits::templates()).unwrap()
    }

    /// The game fixture's own chunk, the level of the placement.
    fn ready_chunked_game(game: &mut crate::game::ServerGame<'_>) {
        // The templates' resources the level places from, as opening a world
        // installs them.
        sunrise_gameplay_common::game::growth_features::install_live_features(
            &game.res,
            &game.pack,
            &game.options,
            &Default::default(),
        );
        game.world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
        );
        // The shared fixture is unticked: a dedicated server is what a server
        // host call asks for.
        game.options.dedicated = Some(Vec::new());
    }

    #[test]
    #[ignore = "daily"]
    fn a_carried_structure_places_its_blocks_block_entity_and_entity() {
        use crate::game::native_entity_refs::tests::with_game;
        use sunrise_level::containers::BlockEntity;
        use sunrise_sim::mobs::Species;

        with_game(|game| {
            ready_chunked_game(game);
            let level = game.level_ref(game.native_lifecycle.epoch().unwrap(), game.dim.key());
            let registry = game.pack.assets.registry;
            let at = [2, 64, 2];
            let placed = game.level_place_template(
                level,
                &StructureTemplateSource::nbt(carried_structure_nbt()),
                at,
                at,
                StructureRotation::Clockwise90,
                StructureMirror::None,
            );
            assert_eq!(
                placed,
                Ok(()),
                "a carried structure places as /place template does"
            );
            // A quarter turn takes (x, y, z) to (-z, y, x): the stone stays on the
            // corner `at`, the chest moves one block along +z, and the block it
            // would have covered without the turn keeps the air it had.
            let name = |state| {
                registry
                    .state(state)
                    .map(|state| state.name.clone())
                    .unwrap_or_else(|| format!("no state {}", state.0))
            };
            assert_eq!(
                name(game.world.block(at[0], at[1], at[2])),
                "minecraft:stone"
            );
            assert_eq!(
                name(game.world.block(at[0], at[1], at[2] + 1)),
                "minecraft:chest"
            );
            assert_eq!(
                name(game.world.block(at[0] + 1, at[1], at[2])),
                "minecraft:air"
            );
            // The block entity came with the block, its data with it.
            let entity = game
                .world
                .block_entities
                .get(&(at[0], at[1], at[2] + 1))
                .cloned()
                .expect("the chest's block entity");
            assert!(
                matches!(&entity, BlockEntity::Chest(_)),
                "the block entity is the chest the template held, not another"
            );
            // And the template's own entity is in the level, where vanilla's
            // placement of a relative (0.5, 0, 0.5) puts it.
            assert!(
                game.mobs.iter().any(|mob| mob.species() == Species::Pig),
                "the pig the template brought was spawned"
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_chunk_the_template_covers_that_is_not_loaded_refuses_and_places_nothing() {
        use crate::game::native_entity_refs::tests::with_game;

        with_game(|game| {
            ready_chunked_game(game);
            let level = game.level_ref(game.native_lifecycle.epoch().unwrap(), game.dim.key());
            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            // A 17x1x1 box from `at`: its first column is in chunk (0, 0), which
            // this fixture has loaded, and its last one in chunk (1, 0), which it
            // has not.
            let pos = |x: i32| Tag::List(vec![Tag::Int(x), Tag::Int(0), Tag::Int(0)]);
            let block = |x: i32| Tag::compound([("pos", pos(x)), ("state", Tag::Int(0))]);
            let tag = Tag::compound([
                (
                    "size",
                    Tag::List(vec![Tag::Int(17), Tag::Int(1), Tag::Int(1)]),
                ),
                (
                    "palettes",
                    Tag::List(vec![Tag::List(vec![Tag::compound([(
                        "Name",
                        Tag::String("minecraft:stone".into()),
                    )])])]),
                ),
                ("blocks", Tag::List(vec![block(0), block(16)])),
            ]);
            let named = nbt::NamedTag {
                name: "".into(),
                tag,
            };
            let bytes = nbt::write_named(&named, Limits::templates()).unwrap();
            let at = [10, 64, 2];
            // The loaded column holds stone the placement would overwrite.
            game.world
                .set(at[0], at[1], at[2], stone)
                .expect("a write in the loaded chunk");
            let refused = game.level_place_template(
                level,
                &StructureTemplateSource::nbt(bytes),
                at,
                at,
                StructureRotation::None,
                StructureMirror::None,
            );
            assert_eq!(
                refused,
                Err(StructureTemplateError::UnloadedChunk { chunk: [1, 0] })
            );
            assert_eq!(
                game.world.block(at[0], at[1], at[2]),
                stone,
                "the checked box refused the whole placement, the loaded part included"
            );
        });
    }
}
