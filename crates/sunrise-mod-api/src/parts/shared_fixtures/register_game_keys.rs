/// Register the game fixture's final block registry through the actual registrar, retaining
/// its installed test mods and Parts. Production mods request only the entries they use.
/// `mod_fluids` names the world fluids the loader admitted from mod files, source and flowing,
/// which the game resolves after the engine's.
pub fn register_game_keys(
    ids: impl IntoIterator<Item = String>,
    mod_fluids: &[&str],
    schema: &mod_vocab::__private::RuleSchema<'_>,
    dimension_shape: fn(&crate::DimensionSettings) -> bool,
) {
    crate::__private::initialize().expect("fixture registration requires the mod API runtime");
    let ids: std::collections::BTreeSet<String> = ids.into_iter().collect();
    mod_vocab::testing::rebuild_parts(|| {
        let mut registrar = super::with_active_rule_schema(schema, || {
            super::register_all(crate::__private::mods(), dimension_shape)
        });
        registrar.mod_id = "sunrise";
        registrar.declare_fluid("definition_test");
        // Request in one order: callers hand over a `HashSet`, whose iteration order differs
        // per game, and a registry whose slots differ by order cannot share its payloads.
        for id in &ids {
            registrar.block_key(id);
        }
        super::install_registrar(registrar, schema)?;
        let mut fluids = vec![
            "minecraft:empty",
            "minecraft:water",
            "minecraft:flowing_water",
            "minecraft:lava",
            "minecraft:flowing_lava",
        ];
        fluids.extend(mod_fluids);
        mod_vocab::__private::resolve_fluids(
            &fluids,
            // The engine's world fluids in vanilla registry order, as the game resolves them.
            &[
                "minecraft:empty",
                "minecraft:flowing_water",
                "minecraft:water",
                "minecraft:flowing_lava",
                "minecraft:lava",
            ],
        )
        .unwrap();
        Ok::<(), Vec<String>>(())
    })
    .unwrap();
}
