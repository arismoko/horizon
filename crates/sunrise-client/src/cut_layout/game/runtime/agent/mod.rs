#![cfg(test)]
#[cfg(feature = "client")]
#[cfg(test)]
#[path = "face_pick_tests.rs"]
pub(crate) mod face_pick_tests;

#[cfg(test)]
#[cfg(feature = "client")]
pub(crate) mod tests {

    use serde_json::json;

    #[test]
    #[ignore = "daily"]
    fn region_adoption_agent_state_reports_the_actual_title_mode() {
        crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
            use sunrise_client::title::{Mode, Title, list_worlds};
            let scratch = tempfile::tempdir().unwrap();
            let saves = scratch.path().join("saves");
            sunrise_sim::save::Level {
                name: "Moor".into(),
                ..Default::default()
            }
            .write(&saves.join("Moor"), &Default::default())
            .unwrap();
            let world = list_worlds(&saves).remove(0);
            let mut title = Title::new(saves, scratch.path(), "");
            title.mode = Mode::Edit {
                world: world.clone(),
                name: "Typed name".into(),
                focused: true,
                adopt_regions: true,
            };
            game.client.title.0 = Some(title);
            assert_eq!(
                game.title_value(),
                json!({"mode":"edit-world", "folder":"Moor", "fields":{"name":"Typed name"}, "focus":"name", "adopt_biome_regions":true})
            );
            game.client.title.0.as_mut().unwrap().mode = Mode::AdoptBiomeRegions {
                world,
                name: "Typed name".into(),
            };
            assert_eq!(
                game.title_value(),
                json!({"mode":"adopt-biome-regions", "folder":"Moor", "name":"Moor"})
            );
        });
    }
}
