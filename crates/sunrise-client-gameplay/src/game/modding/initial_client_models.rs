//! First client reload: the initial game owns no published model generation until apply.
use super::{PackResources, Renderer, UiResources, client_reload};
use std::sync::Arc;
use sunrise_client::gfx::PreparedModels;

pub struct InitialModelGeneration {
    pub(in crate::game) pack: PackResources,
    pub client_pack: crate::game::client_side::resource_tables_client::ClientPackResources,
    pub(in crate::game) gui: UiResources,
}

pub struct PendingClientStart {
    pub models: InitialModelGeneration,
    pub(in crate::game) applies: Vec<client_reload::PreparedClientReload>,
    pub(in crate::game) pack_error: Option<String>,
    pub(in crate::game) handoff: bool,
}

impl crate::game::ClientGame<'_> {
    /// AtlasManager.apply: adopt the uploaded textures and the GUI sprites, without
    /// admitting the new model store or its GPU buffers.
    pub(in crate::game::client_side) fn publish_client_atlases(
        &mut self,
        atlas: &Arc<super::RenderAssets>,
        gui: UiResources,
        renderer: Option<Box<Renderer>>,
    ) -> Option<PreparedModels> {
        let mut prepared = None;
        if let Some(mut renderer) = renderer {
            prepared = renderer.take_prepared_models();
            if let Some(old) = &self.client.renderer.0 {
                renderer.resize(old.config.width, old.config.height);
            }
            self.set_renderer(renderer);
        }
        self.client.stitching_block_atlas.0 = Some(atlas.clone());
        self.native_texture_atlas_stitched(mod_api::TextureAtlas::Blocks);
        self.client.gui = gui;
        sunrise_client::ui_theme::reset_atlas();
        self.native_texture_atlas_stitched(mod_api::TextureAtlas::Gui);
        prepared
    }

    pub(in crate::game::client_side) fn install_model_upload(
        &mut self,
        prepared: Option<PreparedModels>,
    ) {
        self.client.stitching_block_atlas.0 = None;
        if let Some(renderer) = &mut self.client.renderer.0 {
            renderer.publish_prepared_models(prepared);
        }
    }

    pub(in crate::game::client_side) fn apply_initial_model_generation(
        &mut self,
        models: InitialModelGeneration,
        client_applies: Vec<client_reload::PreparedClientReload>,
        renderer: Option<Box<Renderer>>,
    ) -> sunrise_game_data::Result<()> {
        self.common.join_world_rest()?;
        let InitialModelGeneration {
            pack,
            client_pack,
            gui,
        } = models;
        let item_extensions = Self::prepare_client_item_extensions(
            &gui,
            &mut self.client.client_item_extension_owners,
        );
        let geometry = super::native_block_entity_draw::StandaloneGeometry::new(
            &pack,
            &client_pack,
            &gui,
            self.try_block_entity_render(),
        );
        // The atlas listener applies before the model manager. Both use the same
        // prepared atlas/model bundle, but each publishes only its own store.
        let upload = self.publish_client_atlases(&client_pack.render, gui, renderer);
        self.apply_client_reload(client_applies, |game| {
            game.pack = pack;
            game.client.client_pack = client_pack;
            game.client.standalone_geometry = geometry;
            let context = game.worker_context();
            game.client
                .streaming
                .reload_resources(&mut game.common.world, context);
            game.install_model_upload(upload);
            game.client.armor_stand_assets.0.extensions = item_extensions;
            game.resolve_entity_renderers();
            game.native_model_baking_completed();
        });
        self.native_client_setup();
        Ok(())
    }

    /// Fixtures with an already constructed Game drive the same application path.
    #[cfg(any(test, feature = "test-support"))]
    pub fn publish_initial_client_models(
        &mut self,
        client_applies: Vec<client_reload::PreparedClientReload>,
    ) -> sunrise_game_data::Result<()> {
        let pack = self.common.pack.clone();
        let client_pack = self.client.client_pack.clone();
        let metrics = UiResources::fonts_only(&pack.source)?;
        let gui = std::mem::replace(&mut self.client.gui, metrics);
        self.apply_initial_model_generation(
            InitialModelGeneration {
                pack,
                client_pack,
                gui,
            },
            client_applies,
            None,
        )
    }
}
