use super::item_draw::{BlockItemDraw, block_entity_pose};
use sunrise_assets::item_model::DisplayContext;
use sunrise_client::{
    block_entity_render::vertex_light, camera::rotate_degrees, item_render, mesher::Vertex,
};
use sunrise_client_render::block_entity_reach::Reach;
use sunrise_items::items::ItemStack;
use sunrise_level::containers::BlockEntity;

impl crate::game::ClientGame<'_> {
    /// `VaultRenderer`: the display item of every vault near the camera that
    /// shows one, turning above its floor.
    pub(in crate::game) fn vault_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
    ) {
        self.vault_items(camera, alpha, |d| {
            self.submit_item_world(d.state, d.pose, d.light, out);
        });
    }
    /// The items [`Self::vault_geometry`] draws: each copy of a display item's
    /// cluster (`ItemEntityRenderer.renderMultipleFromCount`).
    pub fn vault_items(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        mut draw: impl FnMut(BlockItemDraw),
    ) {
        let _items = sunrise_level::spans::raw_flight_guard("block_entity_items");
        let reach = Reach::Default;
        sunrise_level::spans::note_block_entity_item_visits(|| {
            self.common.world.block_entities.len() as u64
        });
        for (key, entity) in &self.common.world.block_entities {
            let BlockEntity::Vault(vault) = entity else {
                continue;
            };
            let Some(stack) = &vault.display_item else {
                continue;
            };
            if !reach.should_render([key.0, key.1, key.2], camera, 0) {
                continue;
            }
            let state = self.resolve_item(stack, DisplayContext::Ground, None, 0);
            if state.is_empty() {
                continue;
            }
            let mut pose = block_entity_pose(*key, camera);
            pose.translate(0.5, 0.4, 0.5);
            rotate_degrees(&mut pose, 1, vault.display_spin(alpha as f32));
            let light = vertex_light(self.common.world.light(key.0, key.1, key.2));
            let seed = i64::from(self.common.item_number(&stack.item).unwrap_or(0))
                + i64::from(stack.damage);
            for copy in item_render::cluster_poses(
                &state,
                pose.to_glam(),
                item_render::rendered_amount(stack.count),
                seed,
            ) {
                draw(BlockItemDraw {
                    #[cfg(any(test, feature = "test-support"))]
                    stack,
                    #[cfg(any(test, feature = "test-support"))]
                    display: DisplayContext::Ground,
                    #[cfg(any(test, feature = "test-support"))]
                    seed: 0,
                    state: &state,
                    pose: copy,
                    light,
                });
            }
        }
    }
    /// VaultBlock.useItemOn on a client level: something in the hand on an ACTIVE vault is
    /// SUCCESS_SERVER, so the click ends there (no UseItem, no other hand, no swing); only the
    /// server tries the key.
    pub(in crate::game::client_side) fn vault_takes_click(
        &self,
        pos: [i32; 3],
        held: Option<&ItemStack>,
    ) -> bool {
        let state = self.common.world.block(pos[0], pos[1], pos[2]);
        held.is_some()
            && self
                .common
                .pack
                .assets
                .registry
                .state(state)
                .is_some_and(|block| {
                    sunrise_game_data::block_entity_types::factory_block(&block.name)
                        == "minecraft:vault"
                        && block.properties.get("vault_state").map(String::as_str) == Some("active")
                })
    }
}
