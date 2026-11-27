#[cfg(test)]
pub(in crate::game) mod source_tests {
    #[test]
    fn frame_model_reads_resolved_resources_without_files() {
        use sunrise_assets::resource_source::{Resource, ResourceSource};
        let source = ResourceSource::indexed(std::collections::BTreeMap::from([
            ("assets/minecraft/models/block/item_frame.json".into(), Resource::bytes(br##"{"textures":{"x":"example:block/frame"},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"north":{"texture":"#x"}}}]}"##.to_vec())),
        ]));
        let library = sunrise_client::model_library::ParsedModels::load(&source)
            .into_item_library()
            .unwrap();
        let [model, ..] = crate::game::client_side::frames_client::bake_frames(Some(&library));
        assert_eq!(model.len(), 1);
        assert_eq!(model[0].2, "example:block/frame");
    }
}
