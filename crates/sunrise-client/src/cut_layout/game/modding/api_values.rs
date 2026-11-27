#[path = "api_values/small_values.rs"]
pub(crate) mod small_values;

#[cfg(feature = "client")]
#[cfg(test)]
pub(crate) fn resolve_cached_block_definitions(
    cached_registry: &'static sunrise_world::registry::BlockRegistry,
) -> Result<&'static sunrise_world::registry::BlockRegistry, String> {
    if !mod_api::__private::parts_installation::parts_installed() {
        mod_platform::__private::part_tests::install_parts_fixture(&[])
            .map_err(|errors| errors.join("\n"))?;
    }
    sunrise_gameplay_common::game::api_values::resolve_cached_block_definitions(cached_registry)
}

#[cfg(test)]
pub(crate) fn install_test_platform() {
    mod_platform::__private::test_platform::install();
}

#[cfg(test)]
pub(crate) fn register_game_keys(ids: impl IntoIterator<Item = String>) {
    sunrise_gameplay_common::game::test_fixtures::register_game_keys(
        ids,
        mod_platform::__private::part_tests::register_game_keys,
    );
}
