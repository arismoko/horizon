#![cfg(test)]
#[cfg(test)]
pub(crate) mod tests {

    use sunrise_gameplay::game::commands::compute::ComputeContext;
    use sunrise_gameplay_common::game::commands_common::Target;

    #[test]
    #[ignore = "daily"]
    fn published_predicates_and_nested_provider_shorthand_evaluate() {
        use crate::game::*;
        use serde_json::json;
        use std::{path::PathBuf, sync::Arc, time::Instant};
        use sunrise_assets::resource_source::ResourceRoot;

        let root = PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES")
                .expect("set SUNRISE_TEST_RESOURCES to prepared resources"),
        );
        let settings = Settings::default();
        let mut loaded = load_resources_uncached(
            ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let fixture = scratch.path().join("compute-owner-providers");
        for (registry, value) in [
            ("context_int_provider", json!(17)),
            ("context_float_provider", json!(2.5)),
        ] {
            let dir = fixture.join("data/minecraft").join(registry).join("nested");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("value.json"), value.to_string()).unwrap();
        }
        Arc::get_mut(&mut loaded.resources)
            .unwrap()
            .rest
            .get_mut()
            .unwrap()
            .providers = sunrise_level::providers::Providers::load(&fixture);
        std::fs::remove_dir_all(&fixture).unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = execution_role::ExecutionRole::IntegratedServer;
        let mut game = sunrise_gameplay::game::ServerGame::new(
            &loaded.resources,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        game.server.fns.0.store.predicates.insert(
            "minecraft:nested/true".into(),
            json!({"condition": "minecraft:all_of", "terms": []}),
        );
        let context = ComputeContext::default();
        for id in ["nested/value", "minecraft:nested/value"] {
            assert_eq!(
                game.provider_int(
                    &json!({"type": "minecraft:add", "inputs": [id, 3]}),
                    &context
                ),
                Ok(20),
            );
            assert_eq!(
                game.provider_float(
                    &json!({"type": "minecraft:add", "inputs": [id, 0.5]}),
                    &context
                ),
                Ok(3.0),
            );
        }
        for predicate in ["nested/true", "minecraft:nested/true"] {
            assert_eq!(
                game.provider_int(
                    &json!({
                        "type": "minecraft:conditional", "condition": predicate,
                        "on_true": "nested/value", "on_false": -1
                    }),
                    &context
                ),
                Ok(17),
            );
            assert_eq!(
                game.provider_float(
                    &json!({
                        "type": "minecraft:number_dispatcher",
                        "cases": [{"condition": predicate, "value": "nested/value"}],
                        "default": -1
                    }),
                    &context
                ),
                Ok(2.5),
            );
        }
        // `/compute entity`: `target_entity` is the selected entity, and the
        // source (here with no entity) is only `this`.
        game.set_allow_commands(true);
        assert!(game.run_command("scoreboard objectives add points dummy"));
        game.player = Some(
            sunrise_sim::motion::PlayerMotion::new(
                game.motion,
                sunrise_core::geometry::Vec3::new(0.5, 64.0, 0.5),
            )
            .unwrap(),
        );
        assert!(game.run_command("scoreboard players set @s points 5"));
        let mut source = game.player_source();
        source.entity = None;
        game.server.command_source = Some(source);
        let on_target = |kind: &str| {
            json!({
                "type": "minecraft:conditional",
                "condition": {"condition": "minecraft:entity_properties", "entity": "target_entity",
                    "predicate": {"minecraft:entity_type": kind}},
                "on_true": 1, "on_false": 0
            })
        };
        let entity = ComputeContext {
            target: Some(Target::Player),
            ..ComputeContext::default()
        };
        assert_eq!(
            game.provider_int(&on_target("minecraft:player"), &entity),
            Ok(1)
        );
        assert_eq!(
            game.provider_int(&on_target("minecraft:cow"), &entity),
            Ok(0)
        );
        assert_eq!(
            game.provider_int(&on_target("minecraft:player"), &ComputeContext::default()),
            Ok(0)
        );
        // `entity_scores` reads the selected holder's live scoreboard value.
        let scored = |n: i32| {
            json!({
                "type": "minecraft:conditional",
                "condition": {"condition": "minecraft:entity_scores", "entity": "target_entity",
                    "scores": {"points": n}},
                "on_true": 1, "on_false": 0
            })
        };
        assert_eq!(game.provider_int(&scored(5), &entity), Ok(1));
        assert_eq!(game.provider_int(&scored(4), &entity), Ok(0));
    }
}
