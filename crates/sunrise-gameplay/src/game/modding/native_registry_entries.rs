#[cfg(test)]
mod condition_tests {
    #[test]
    fn registry_tags_ignore_top_level_conditions() {
        let root = tempfile::tempdir().unwrap();
        let files = vec![(
            "tags/test:spell/hot".to_owned(),
            vec![
                br#"{"neoforge:conditions":[{"type":"neoforge:never"}],"values":["test:fire"]}"#
                    .to_vec(),
            ],
        )];
        let tags = crate::game::server_side::native_registry_entries_server::load_registry_tags(
            root.path(),
            &sunrise_world::loot::PackOverlay {
                files: &files,
                ..sunrise_world::loot::PackOverlay::default()
            },
            "spell",
            |id| id == "test:fire",
        )
        .unwrap();
        assert_eq!(tags["test:hot"], ["test:fire"]);
    }
}
