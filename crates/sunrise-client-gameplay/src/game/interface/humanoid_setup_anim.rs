use super::{stock_layer_mask::StockLayerMask, *};
use sunrise_client::mobs::render;
use sunrise_sim::mobs::anim;

impl render::RenderDataSource for crate::game::ClientGame<'_> {
    fn mob_tables(&self) -> sunrise_client::mobs_render::ClientMobRender<'_> {
        sunrise_client::mobs_render::ClientMobRender {
            models: self.client_res().client_mob_models(),
            data: self.client_res().client_mob_data(),
        }
    }

    fn equipment_layers(&self, mob: &Mob) -> Vec<render::Layer> {
        self.mob_equipment_layers(
            mob,
            super::equipment_piece::EquipmentPass::Draw,
            StockLayerMask::EMPTY,
        )
    }

    fn equipment_inventory(&self, mob: &Mob) -> Vec<render::Layer> {
        self.mob_equipment_layers(
            mob,
            super::equipment_piece::EquipmentPass::TextureInventory,
            StockLayerMask::EMPTY,
        )
    }

    fn humanoid_setup(&self) -> Option<&dyn anim::HumanoidSetup> {
        mod_api::__private::listened(mod_api::HookId::HumanoidSetupAnim)
            .then_some(self as &dyn anim::HumanoidSetup)
    }

    fn variants(&self) -> Option<&sunrise_sim::mobs::variants::VariantTables> {
        Some(self.world_variants())
    }

    fn model_posed(&self) -> Option<&dyn render::ModelPosed> {
        mod_api::__private::listened(mod_api::HookId::LivingModelPosed)
            .then_some(self as &dyn render::ModelPosed)
    }

    fn piglin_loves(&self, item: &str) -> bool {
        const TAG: &str = "minecraft:piglin_loved";
        if let Some(remote) = self.client.remote.as_ref()
            && let Some(tags) = remote.registries.tags.get("minecraft:item")
            && let Some(ids) = remote.builtin.get("minecraft:item")
        {
            return tags
                .get(TAG)
                .is_some_and(|members| members.iter().any(|&id| ids.name(id) == Some(item)));
        }
        self.items()
            .item_tags
            .get(TAG)
            .is_some_and(|members| members.contains(item))
    }

    fn has_tool(&self, stack: &sunrise_items::items::ItemStack) -> bool {
        self.items().has_tool(stack)
    }
}

pub(in crate::game) struct HiddenLayersData<'a, 'game> {
    pub(in crate::game) game: &'a crate::game::ClientGame<'game>,
    pub(in crate::game) hidden_layers: StockLayerMask,
}

impl render::RenderDataSource for HiddenLayersData<'_, '_> {
    fn mob_tables(&self) -> sunrise_client::mobs_render::ClientMobRender<'_> {
        render::RenderDataSource::mob_tables(self.game)
    }

    fn equipment_layers(&self, mob: &Mob) -> Vec<render::Layer> {
        self.game.mob_equipment_layers(
            mob,
            super::equipment_piece::EquipmentPass::Draw,
            self.hidden_layers,
        )
    }

    fn equipment_inventory(&self, mob: &Mob) -> Vec<render::Layer> {
        render::RenderDataSource::equipment_inventory(self.game, mob)
    }

    fn humanoid_setup(&self) -> Option<&dyn anim::HumanoidSetup> {
        render::RenderDataSource::humanoid_setup(self.game)
    }

    fn variants(&self) -> Option<&sunrise_sim::mobs::variants::VariantTables> {
        render::RenderDataSource::variants(self.game)
    }

    fn model_posed(&self) -> Option<&dyn render::ModelPosed> {
        render::RenderDataSource::model_posed(self.game)
    }

    fn piglin_loves(&self, item: &str) -> bool {
        render::RenderDataSource::piglin_loves(self.game, item)
    }

    fn has_tool(&self, stack: &sunrise_items::items::ItemStack) -> bool {
        render::RenderDataSource::has_tool(self.game, stack)
    }
}

#[cfg(test)]
#[path = "tests/hidden_layers_variants_tests.rs"]
mod hidden_layers_variants_tests;

impl anim::HumanoidSetup for crate::game::ClientGame<'_> {
    fn before_arm_pose(&self, mob: &Mob, state: &anim::State<'_>, poses: &mut anim::Poses) {
        use sunrise_gameplay_common::game::api_values::TryIntoApi;
        let stacks = [false, true].map(|off| {
            render::held_item_stack(&mob.extra, off).and_then(|stack| stack.try_into_api().ok())
        });
        let [main, off] = anim::humanoid_arm_poses(mob, mob.extra.held_arm_pose, state.swing);
        let humanoid = mod_api::HumanoidRenderState {
            main_hand: stacks[0].as_ref(),
            off_hand: stacks[1].as_ref(),
            main_arm_left: mob.main_arm_left(),
            is_using_item: mob.extra.using_item > 0,
            use_hand: (mob.extra.using_item > 0).then_some(if mob.extra.using_off_hand {
                Hand::Off
            } else {
                Hand::Main
            }),
            ticks_using_item: mob.extra.using_item,
            use_animation: mob.extra.use_animation,
            is_swinging: mob.is_swinging(),
            main_arm_pose: main.try_into_api().unwrap(),
            off_arm_pose: off.try_into_api().unwrap(),
        };
        self.setup_humanoid_parts(mob.id, &mob.kind.id, &humanoid, state.partial, poses);
    }
}

impl crate::game::ClientGame<'_> {
    pub(in crate::game) fn world_variants(&self) -> &sunrise_sim::mobs::variants::VariantTables {
        self.client
            .remote
            .as_ref()
            .filter(|remote| !remote.variants.is_empty())
            .map_or_else(
                || self.common.res.variants(),
                |remote| remote.variants.as_ref(),
            )
    }

    /// Lends the live pose to listeners at the shared humanoid seam.
    pub(in crate::game) fn setup_humanoid_parts(
        &self,
        entity_id: u32,
        entity_type: &str,
        humanoid: &mod_api::HumanoidRenderState<'_>,
        partial_tick: f32,
        poses: &mut anim::Poses,
    ) {
        self.native_humanoid_setup_anim(mod_api::__private::HumanoidSetupAnimCall {
            entity_id,
            entity_type,
            humanoid,
            partial_tick,
            game_time: self
                .client
                .title
                .0
                .is_none()
                .then_some(self.common.world.game_time),
            parts: poses,
        });
    }
}

#[cfg(test)]
mod piglin_loved_tests {
    use crate::game::native_entity_refs::tests::with_client_game;
    use std::sync::Arc;
    use sunrise_client::mobs::render::RenderDataSource;
    use sunrise_protocol::packets::configuration::{RegistryTags, TagEntry, UpdateTags};

    /// A client reads `#minecraft:piglin_loved` from the server's tags (#5132): a server whose
    /// tag holds only a stick loves the stick and not gold. With no server, the game's own
    /// tags answer, vanilla's gold, so nothing of the server's outlives its connection.
    #[test]
    #[ignore = "daily"]
    fn a_client_reads_piglin_loved_from_the_servers_tags() {
        with_client_game(|game| {
            let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(pipe, "PiglinLoved").unwrap();
            let stick = game
                .client
                .remote
                .as_ref()
                .unwrap()
                .builtin
                .get("minecraft:item")
                .unwrap()
                .id("minecraft:stick")
                .unwrap();
            let registries = &mut game.client.remote.as_mut().unwrap().registries;
            Arc::make_mut(registries).add_tags(&UpdateTags {
                registries: vec![RegistryTags {
                    registry: "minecraft:item".into(),
                    tags: vec![TagEntry {
                        name: "minecraft:piglin_loved".into(),
                        ids: vec![stick],
                    }],
                }],
            });
            assert!(game.piglin_loves("minecraft:stick"));
            assert!(!game.piglin_loves("minecraft:gold_ingot"));
            game.client.remote = None;
            assert!(game.piglin_loves("minecraft:gold_ingot"));
            assert!(!game.piglin_loves("minecraft:stick"));
        });
    }
}
