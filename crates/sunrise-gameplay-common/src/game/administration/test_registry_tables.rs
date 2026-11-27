use crate::test_instances::{TestDef, parse_test_def, parse_test_def_with_env};

pub fn build_game_test_tables(
    packed: &sunrise_sim::lan::data::PackRegistryDefinitions,
    functions: &mod_api::GameTestFunctions,
) -> (
    std::sync::Arc<std::collections::HashMap<String, crate::test_environments::Environment>>,
    std::sync::Arc<std::collections::HashMap<String, TestDef>>,
) {
    let env_jsons: std::collections::HashMap<String, serde_json::Value> = packed
        .validated("minecraft:test_environment")
        .into_iter()
        .map(|(id, value)| (id, value.clone()))
        .collect();
    let rules = sunrise_game_data::gamerules::GameRules::default();
    let load_env = |id: &str| {
        if id.contains(':') {
            env_jsons.get(id).cloned()
        } else {
            env_jsons.get(&format!("minecraft:{id}")).cloned()
        }
    };
    let mut test_environments = std::collections::HashMap::with_capacity(env_jsons.len());
    for (id, json) in &env_jsons {
        if let Some(env) = crate::test_environments::parse(json, &load_env, 0, &rules) {
            test_environments.insert(id.clone(), env);
        }
    }
    let mut test_instances = std::collections::HashMap::new();
    for (id, json) in packed.validated("minecraft:test_instance") {
        if let Some(mut def) = parse_test_def_with_env(json, &load_env, &rules) {
            def.callback = def
                .function
                .as_deref()
                .and_then(|fn_id| functions.resolve(fn_id));
            test_instances.insert(id, def);
        }
    }
    for (id, definition) in functions.definitions() {
        let mut def = parse_test_def(&serde_json::json!({
            "type": "minecraft:function",
            "structure": definition.structure,
            "function": definition.function,
            "environment": definition.environment,
            "dimension": definition.dimension,
            "max_ticks": definition.max_ticks,
            "setup_ticks": definition.setup_ticks,
            "max_attempts": definition.max_attempts,
            "required_successes": definition.required_successes,
            "required": definition.required,
        }))
        .expect("validated registered test definition");
        def.callback = def
            .function
            .as_deref()
            .and_then(|fn_id| functions.resolve(fn_id));
        test_instances.entry(id.to_owned()).or_insert(def);
    }
    (
        std::sync::Arc::new(test_environments),
        std::sync::Arc::new(test_instances),
    )
}
