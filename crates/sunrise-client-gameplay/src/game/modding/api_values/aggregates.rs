#[cfg(test)]
mod tests {
    use crate::game::client_side::api_values_client::aggregates::scene_blocks;
    use mod_api::{Identifier, SceneBlockEntity};
    use sunrise_gameplay_common::game::api_values::TryIntoApi;
    use sunrise_gameplay_common::game::api_values::aggregates::block_state;
    use sunrise_world::registry::{BlockRegistry, BlockStateId};

    #[test]
    fn aggregates_preserve_properties_reorder_and_refuse_whole_values() {
        mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
        let facts = r#"{"defaults":{"minecraft:air":0,"minecraft:stairs":2},"states":[["a",true,false,"","minecraft:air"],["s",false,false,"",{"id":"minecraft:stairs","properties":{"facing":"north","waterlogged":"false"}}],["s",false,false,"",{"id":"minecraft:stairs","properties":{"facing":"east","waterlogged":"true"}}]]}"#;
        let registry = BlockRegistry::from_facts_json(facts.as_bytes()).unwrap();
        sunrise_gameplay_common::game::api_values::resolve_block_definitions(&registry).unwrap();
        let north = BlockStateId(1).try_into_api().unwrap();
        let east = BlockStateId(2).try_into_api().unwrap();
        let metadata = mod_api::__private::block_definitions().unwrap();
        assert_eq!((north.ordinal(), east.ordinal()), (0, 1));
        assert_eq!(east.property(metadata, "facing"), Ok(Some("east")));
        assert_eq!(east.property(metadata, "waterlogged"), Ok(Some("true")));
        let reordered = r#"{"defaults":{"minecraft:air":0,"minecraft:stairs":1},"states":[["a",true,false,"","minecraft:air"],["s",false,false,"",{"id":"minecraft:stairs","properties":{"facing":"east","waterlogged":"true"}}],["s",false,false,"",{"id":"minecraft:stairs","properties":{"facing":"north","waterlogged":"false"}}]]}"#;
        let registry = BlockRegistry::from_facts_json(reordered.as_bytes()).unwrap();
        sunrise_gameplay_common::game::api_values::resolve_block_definitions(&registry).unwrap();

        let entity = SceneBlockEntity::new(
            Identifier::parse("test:scene").unwrap(),
            east,
            Default::default(),
        );
        let mut scene = vec![
            mod_api::SceneBlock::new([0, 0, 0], north, None),
            mod_api::SceneBlock::new([1, 0, 0], east, Some(entity)),
            mod_api::SceneBlock::new([2, 0, 0], north, None),
        ];
        let original = scene.clone();
        let prepared = scene_blocks(&scene).unwrap();
        assert_eq!(
            prepared.iter().map(|b| b.state).collect::<Vec<_>>(),
            [BlockStateId(2), BlockStateId(1), BlockStateId(2)]
        );
        assert_eq!(prepared[1].pos, [1, 0, 0]);
        assert!(std::ptr::eq(
            prepared[1].entity.unwrap(),
            scene[1].entity.as_ref().unwrap()
        ));
        assert_eq!(scene, original);
        scene[1].state = mod_api::__private::block_state_from_ordinal(east.block(), u16::MAX);
        assert!(matches!(
            scene_blocks(&scene),
            Err(mod_api::SceneError::BadBlock { index: 1 })
        ));
        assert_eq!(scene[1].state.ordinal(), u16::MAX);

        // A valid API key absent from this world's index refuses, rather than drawing air.
        let only_air = BlockRegistry::from_facts_json(
            br#"{"defaults":{"minecraft:air":0},"states":[["a",true,false,"","minecraft:air"]]}"#
                .as_slice(),
        )
        .unwrap();
        block_state::bind(&only_air).unwrap();
        scene[1].state = east;
        assert!(matches!(
            scene_blocks(&scene),
            Err(mod_api::SceneError::BadBlock { index: 0 })
        ));
        block_state::clear();
        assert!(matches!(
            scene_blocks(&scene),
            Err(mod_api::SceneError::Unavailable)
        ));
        assert_eq!(scene, original);
    }
}
