#![cfg(test)]
use sunrise_core::nbt::Tag;
use sunrise_gameplay_common::game::maps::MapData;
use sunrise_items::items::ItemStack;
#[cfg(test)]
pub(crate) mod live_frame_tests;

#[cfg(test)]
pub(crate) mod tests {

    use super::*;

    /// `ItemStack.useOn` / `MapItem.useOn`: in adventure mode, a map whose
    /// `can_place_on` component includes the banner block toggles that banner's
    /// marker, while one without does not.
    #[test]
    #[ignore = "daily"]
    fn map_on_banner_in_adventure_mode_honours_can_place_on() {
        use std::sync::Arc;
        use sunrise_command::args::GameMode;
        use sunrise_level::chunk::Chunk;
        use {
            crate::game::Settings, crate::game::load_resources,
            sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
        };

        let Some(root) = std::env::var_os("SUNRISE_TEST_RESOURCES") else {
            return;
        };
        let root = std::path::PathBuf::from(root);
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

        let pos = [0, 64, 0];
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let banner_state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:red_banner")
            .unwrap();
        game.world
            .set(pos[0], pos[1], pos[2], banner_state)
            .unwrap();
        game.world.block_entities.insert(
            (pos[0], pos[1], pos[2]),
            sunrise_level::containers::BlockEntity::Banner(sunrise_level::containers::Banner {
                layers: Vec::new(),
                name: None,
                components: Default::default(),
            }),
        );

        let map_id = 42;
        game.maps
            .data
            .insert(map_id, MapData::fresh(0.0, 0.0, 0, "minecraft:overworld"));
        game.set_mode(GameMode::Adventure);
        let hit = sunrise_gameplay_common::game::Hit {
            pos,
            face: [0, 1, 0],
            inside: false,
            point: [
                pos[0] as f64 + 0.5,
                pos[1] as f64 + 0.5,
                pos[2] as f64 + 0.5,
            ],
        };
        let mut stack = ItemStack::new("minecraft:filled_map", 1);
        stack.data_mut().map_id = Some(map_id);

        // Without can_place_on, use_map_on_banner fails in adventure mode.
        *game.held_mut() = Some(stack.clone());
        assert!(!game.use_map_on_banner(&hit, &stack, &stack));
        assert!(game.map(map_id).unwrap().banners.is_empty());

        // With can_place_on listing the banner block, the marker toggles.
        stack.data_mut().set_other(
            "minecraft:can_place_on",
            Some(Tag::compound([(
                "blocks",
                Tag::List(vec![Tag::String("minecraft:red_banner".into())]),
            )])),
        );
        *game.held_mut() = Some(stack.clone());
        assert!(game.use_map_on_banner(&hit, &stack, &stack));
        assert_eq!(game.map(map_id).unwrap().banners.len(), 1);
        assert_eq!(game.map(map_id).unwrap().banners[0].pos, pos);
        assert_eq!(game.map(map_id).unwrap().banners[0].color, "red");

        // Second use toggles the marker off.
        assert!(game.use_map_on_banner(&hit, &stack, &stack));
        assert!(game.map(map_id).unwrap().banners.is_empty());
    }
}

#[cfg(test)]
pub(crate) mod command_exploration_map_test_file;
