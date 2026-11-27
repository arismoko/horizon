use mod_api::{__private::game::CallbackOwner, *};
use sunrise_client_render::mod_pipelines::BuiltinRenderType;
use sunrise_gameplay_common::game::api_values::TryIntoEngine;

impl RenderAssets for crate::game::ClientGame<'_> {
    fn model(&self, id: &Identifier) -> Result<ModelRef, ModelError> {
        CallbackOwner::call_host(|| self.native_model(id))
    }
    fn block_state_models(&self, state: BlockState) -> Result<BakedModelKeys<'_>, ModelError> {
        let state = state
            .try_into_engine(&())
            .map_err(|_| ModelError::UnknownModel)?;
        Ok(CallbackOwner::call_host(|| {
            self.client.client_pack.render.block_state_models(state)
        }))
    }
    fn block_state_model(&self, state: BlockState) -> Result<ModelRef, ModelError> {
        let state = state
            .try_into_engine(&())
            .map_err(|_| ModelError::UnknownModel)?;
        CallbackOwner::call_host(|| self.native_block_state_model(state))
    }
    fn baked_model(&self, key: BakedModelKey) -> Result<BakedModelView<'_>, ModelError> {
        CallbackOwner::call_host(|| self.client.client_pack.render.baked_model(key))
    }
    fn atlas_sprite_pixels(
        &self,
        sprite: SpriteRef,
    ) -> Result<AtlasSpritePixels, AtlasSpriteError> {
        CallbackOwner::call_host(|| {
            let (atlas, split) = self.sprite_atlas_and_split();
            atlas.atlas_sprite_pixels(sprite, split)
        })
    }
    fn render_pipeline(&self, id: &Identifier) -> Result<RenderPipelineRef, RenderPipelineError> {
        CallbackOwner::call_host(|| self.client.client_pack.render.render_pipeline(id))
    }

    fn render_type(&self, id: &Identifier) -> Result<RenderTypeRef, RenderTypeError> {
        CallbackOwner::call_host(|| self.client.client_pack.render.render_type(id))
    }

    fn entity_solid(&self, texture: &Identifier) -> Result<RenderTypeRef, RenderTypeError> {
        CallbackOwner::call_host(|| {
            self.client
                .client_pack
                .render
                .textured_render_type(BuiltinRenderType::EntitySolid, texture)
        })
    }

    fn entity_cutout(&self, texture: &Identifier) -> Result<RenderTypeRef, RenderTypeError> {
        CallbackOwner::call_host(|| {
            self.client
                .client_pack
                .render
                .textured_render_type(BuiltinRenderType::EntityCutout, texture)
        })
    }

    fn entity_translucent(&self, texture: &Identifier) -> Result<RenderTypeRef, RenderTypeError> {
        CallbackOwner::call_host(|| {
            self.client
                .client_pack
                .render
                .textured_render_type(BuiltinRenderType::EntityTranslucent, texture)
        })
    }

    fn compute_pipeline(
        &self,
        id: &Identifier,
    ) -> Result<ComputePipelineRef, ComputePipelineError> {
        CallbackOwner::call_host(|| self.client.client_pack.render.compute_pipeline(id))
    }
    fn pipeline_modifier(
        &self,
        id: &Identifier,
    ) -> Result<PipelineModifierRef, RenderPipelineError> {
        CallbackOwner::call_host(|| self.client.client_pack.render.pipeline_modifier(id))
    }
    fn animation(&self, id: &Identifier) -> Option<AnimationHolder> {
        CallbackOwner::call_host(|| self.client.client_pack.render.animation(id))
    }
    fn shader_pack_block_id(&self, state: BlockState) -> Option<i32> {
        let state = state.try_into_engine(&()).ok()?;
        CallbackOwner::call_host(|| self.active_block_state_id(state.0 as usize))
    }
    fn pipeline_pack_bound(&self, pipeline: RenderPipelineRef) -> bool {
        CallbackOwner::call_host(|| {
            self.client
                .client_pack
                .render
                .mod_pipelines
                .slot(pipeline)
                .is_ok_and(|slot| self.pack_slot_bound(slot as usize))
        })
    }
}
