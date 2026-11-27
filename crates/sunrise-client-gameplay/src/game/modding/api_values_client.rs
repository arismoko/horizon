use sunrise_gameplay_common::game::api_values::TryIntoApi;
#[cfg(feature = "client")]
use sunrise_gameplay_common::game::native_hooks::ResidentReadBlockEntity;
#[path = "api_values/aggregates_client.rs"]
pub mod aggregates;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    pub(in crate::game) fn api_block_entity_submit(
        &self,
        pos: [i32; 3],
        block: &str,
        state: sunrise_world::registry::BlockStateId,
        entity: &sunrise_level::containers::ModBlockEntity,
        draw: &mut super::native_block_entity_draw::BlockEntityDraw,
    ) {
        if !mod_api::__private::has_part_hook(mod_api::HookId::BlockEntitySubmit) {
            return;
        }
        let data = ResidentReadBlockEntity::stored(entity);
        self.native_block_entity_submit(
            pos,
            block,
            state,
            mod_api::BlockEntityDataView::new(&data),
            draw,
        );
    }
}

pub(in crate::game) fn mod_bar_value(
    bar: &sunrise_client::boss_overlay::ModBar,
) -> Result<mod_api::ClientBossBar, mod_api::text::TextFormError> {
    Ok(mod_api::ClientBossBar {
        uuid: bar.uuid,
        name: bar.name.try_into_api()?,
        progress: bar.progress,
        color: bar.color,
        overlay: bar.overlay,
        darken_screen: bar.darken_screen,
        play_boss_music: bar.play_boss_music,
        create_world_fog: bar.create_world_fog,
    })
}

/// Prepare callback data before mesh construction; workers never convert engine records.
pub(in crate::game) fn mod_tint_values(
    _registry: &sunrise_world::registry::BlockRegistry,
) -> Result<Option<sunrise_client_render::mesher::ModTintValues>, String> {
    if !sunrise_client_render::mesher::has_custom_block_tint_sources()
        && !sunrise_client_render::mesher::has_section_geometry()
    {
        return Ok(None);
    }
    let metadata = sunrise_gameplay_common::game::api_values::block_metadata_table()
        .map_err(|error| format!("block tint metadata: {error:?}"))?;
    let states = sunrise_gameplay_common::game::api_values::block_state::snapshot()
        .map_err(|error| format!("block tint state: {error:?}"))?;
    Ok(Some(sunrise_client_render::mesher::ModTintValues {
        states,
        metadata,
    }))
}
