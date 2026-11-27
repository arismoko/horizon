#![cfg(test)]
#[cfg(test)]
mod tests {
    use std::ops::ControlFlow;
    use std::sync::Arc;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
    use sunrise_gameplay_common::world_biomes;

    #[test]
    #[ignore = "daily"]
    fn read_columns_returns_server_water_colors_without_colormaps() {
        use std::path::Path;
        use sunrise_assets::resource_source::ResourceRoot;
        use sunrise_level::{biome::Biomes, chunk::Chunk};

        fn link_tree(src: &Path, dst: &Path) {
            #[cfg(unix)]
            std::os::unix::fs::symlink(src, dst).unwrap();
            #[cfg(not(unix))]
            {
                if src.is_dir() {
                    std::fs::create_dir(dst).unwrap();
                    for entry in std::fs::read_dir(src).unwrap() {
                        let entry = entry.unwrap();
                        link_tree(&entry.path(), &dst.join(entry.file_name()));
                    }
                } else if std::fs::hard_link(src, dst).is_err() {
                    std::fs::copy(src, dst).unwrap();
                }
            }
        }

        fn mirror_omitting(src: &Path, dst: &Path, omit: &[&[&str]]) {
            for entry in std::fs::read_dir(src).unwrap() {
                let entry = entry.unwrap();
                let name = entry.file_name();
                let Some(name_str) = name.to_str() else {
                    link_tree(&entry.path(), &dst.join(&name));
                    continue;
                };
                let under: Vec<&[&str]> = omit
                    .iter()
                    .filter_map(|path| {
                        path.split_first()
                            .filter(|(head, _)| **head == name_str)
                            .map(|(_, tail)| tail)
                    })
                    .collect();
                if under.iter().any(|rest| rest.is_empty()) {
                    continue;
                }
                let target = dst.join(&name);
                if under.is_empty() {
                    link_tree(&entry.path(), &target);
                } else {
                    std::fs::create_dir(&target).unwrap();
                    mirror_omitting(&entry.path(), &target, &under);
                }
            }
        }

        const OMIT: &[&[&str]] = &[
            &[sunrise_core::prepared_files::FILE_NAME],
            &["assets", "minecraft", "textures", "colormap", "grass.png"],
            &["assets", "minecraft", "textures", "colormap", "foliage.png"],
            &[
                "assets",
                "minecraft",
                "textures",
                "colormap",
                "dry_foliage.png",
            ],
        ];
        let scratch = tempfile::tempdir().unwrap();
        let resources = scratch.path().join("resources");
        std::fs::create_dir(&resources).unwrap();
        mirror_omitting(
            &super::super::native_entity_refs::tests::fixture_root(),
            &resources,
            OMIT,
        );
        for name in ["grass.png", "foliage.png", "dry_foliage.png"] {
            assert!(
                !resources
                    .join("assets/minecraft/textures/colormap")
                    .join(name)
                    .exists(),
                "private fixture must omit {name}"
            );
        }
        assert!(
            Biomes::load(&ResourceRoot::directory(resources.clone())).is_err(),
            "client biome load must fail when colormaps are absent"
        );

        super::super::native_entity_refs::tests::with_game_role(
            resources,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::DedicatedServer,
            |game| {
                game.options.dedicated = Some(Vec::new());
                assert!(game.world_biomes.is_none());
                let plains = world_biomes!(game).id("minecraft:plains");
                let swamp = world_biomes!(game).id("minecraft:swamp");
                let mut chunk = Chunk::empty(0, 0);
                for qy in 0..chunk.sections.len() as i32 * 4 {
                    for qz in 0..4 {
                        chunk.set_biome_cell(0, qy, qz, plains);
                        chunk.set_biome_cell(1, qy, qz, swamp);
                    }
                }
                game.world.chunks.insert((0, 0), Arc::new(chunk));
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(0, 64, 0, stone).unwrap();
                game.world.set(4, 64, 0, stone).unwrap();

                let overworld = game.level(game.dim.id()).unwrap();
                let mut bases = Vec::new();
                let mut water_colors = Vec::new();
                let mut stone_seen = Vec::new();
                let flow = overworld
                    .visit_columns(
                        game,
                        &[(0, 0), (4, 0), (32, 0)],
                        mod_api::Heightmap::WorldSurface,
                        (1, 1),
                        &mut |_, slice| {
                            match slice {
                                Some(slice) => {
                                    bases.push(Some(slice.base));
                                    water_colors.push(slice.water_color);
                                    stone_seen.push(slice.blocks[0].block());
                                }
                                None => {
                                    bases.push(None);
                                    water_colors.push(0);
                                }
                            }
                            ControlFlow::Continue(())
                        },
                    )
                    .unwrap();
                assert_eq!(flow, ControlFlow::Continue(()));
                assert_eq!(bases, vec![Some(65), Some(65), None]);
                assert_eq!(water_colors, vec![0x3F_76E4, 0x61_7B64, 0]);
                assert_eq!(
                    stone_seen,
                    vec![
                        mod_api::__private::block_key("minecraft:stone").unwrap(),
                        mod_api::__private::block_key("minecraft:stone").unwrap(),
                    ]
                );

                // Both height and column streaming work on parked open levels too.
                let here = game.dim;
                game.enter_level(sunrise_gameplay_common::game::Dim::Nether)
                    .unwrap();
                let mut parked_heights = Vec::new();
                assert_eq!(
                    overworld.visit_surface_heights(
                        game,
                        &[(0, 0), (4, 0), (32, 0)],
                        mod_api::Heightmap::WorldSurface,
                        &mut |pos, height| {
                            parked_heights.push((pos, height));
                            ControlFlow::Continue(())
                        },
                    ),
                    Ok(ControlFlow::Continue(()))
                );
                assert_eq!(
                    parked_heights,
                    vec![((0, 0), Some(65)), ((4, 0), Some(65)), ((32, 0), None)]
                );
                let mut parked_bases = Vec::new();
                assert_eq!(
                    overworld.visit_columns(
                        game,
                        &[(0, 0), (32, 0)],
                        mod_api::Heightmap::WorldSurface,
                        (1, 1),
                        &mut |_, slice| {
                            parked_bases.push(slice.map(|s| (s.base, s.water_color)));
                            ControlFlow::Continue(())
                        },
                    ),
                    Ok(ControlFlow::Continue(()))
                );
                assert_eq!(parked_bases, vec![Some((65, 0x3F_76E4)), None]);
                game.enter_level(here).unwrap();
            },
        );
    }
}
