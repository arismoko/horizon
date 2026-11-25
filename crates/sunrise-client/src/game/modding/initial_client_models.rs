//! First client reload: the initial game owns no published model generation until apply.
use super::{Game, PackResources, Renderer, UiResources, client_reload};
use std::sync::Arc;
use sunrise_client::gfx::PreparedModels;

pub(super) struct InitialModelGeneration {
    pub(super) pack: PackResources,
    pub(super) gui: UiResources,
}

pub(super) struct PendingClientStart {
    pub(super) models: InitialModelGeneration,
    pub(super) applies: Vec<client_reload::PreparedClientReload>,
    pub(super) pack_error: Option<String>,
    pub(super) handoff: bool,
}

impl Game<'_> {
    /// AtlasManager.apply: adopt the uploaded textures and the GUI sprites, without
    /// admitting the new model store or its GPU buffers.
    pub(super) fn publish_client_atlases(
        &mut self,
        atlas: &Arc<super::RenderAssets>,
        gui: UiResources,
        renderer: Option<Renderer>,
    ) -> Option<PreparedModels> {
        let mut prepared = None;
        if let Some(mut renderer) = renderer {
            prepared = renderer.take_prepared_models();
            if let Some(old) = &self.renderer {
                renderer.resize(old.config.width, old.config.height);
            }
            self.set_renderer(renderer);
        }
        self.stitching_block_atlas = Some(atlas.clone());
        self.native_texture_atlas_stitched(mod_api::TextureAtlas::Blocks);
        self.gui = gui;
        sunrise_client::ui_theme::reset_atlas();
        self.native_texture_atlas_stitched(mod_api::TextureAtlas::Gui);
        prepared
    }

    pub(super) fn install_model_upload(&mut self, prepared: Option<PreparedModels>) {
        self.stitching_block_atlas = None;
        if let Some(renderer) = &mut self.renderer {
            renderer.publish_prepared_models(prepared);
        }
    }

    pub(super) fn apply_initial_model_generation(
        &mut self,
        models: InitialModelGeneration,
        client_applies: Vec<client_reload::PreparedClientReload>,
        renderer: Option<Renderer>,
    ) -> sunrise_client::Result<()> {
        self.join_world_rest()?;
        let InitialModelGeneration { pack, gui } = models;
        let item_extensions =
            Self::prepare_client_item_extensions(&gui, &mut self.client_item_extension_owners);
        let geometry = super::native_block_entity_draw::StandaloneGeometry::new(
            &pack,
            &gui,
            self.res.try_block_entity_render(),
        );
        // The atlas listener applies before the model manager. Both use the same
        // prepared atlas/model bundle, but each publishes only its own store.
        let upload = self.publish_client_atlases(&pack.render, gui, renderer);
        self.apply_client_reload(client_applies, |game| {
            game.pack = pack;
            game.standalone_geometry = geometry;
            let context = game.worker_context();
            game.streaming.reload_resources(&mut game.world, context);
            game.install_model_upload(upload);
            game.armor_stand_assets.extensions = item_extensions;
            game.resolve_entity_renderers();
            game.native_model_baking_completed();
        });
        self.native_client_setup();
        Ok(())
    }

    /// Fixtures with an already constructed Game drive the same application path.
    #[cfg(test)]
    pub(in crate::game) fn publish_initial_client_models(
        &mut self,
        client_applies: Vec<client_reload::PreparedClientReload>,
    ) -> sunrise_client::Result<()> {
        let pack = self.pack.clone();
        let metrics = UiResources::server(&pack.source)?;
        let gui = std::mem::replace(&mut self.gui, metrics);
        self.apply_initial_model_generation(
            InitialModelGeneration { pack, gui },
            client_applies,
            None,
        )
    }
}
