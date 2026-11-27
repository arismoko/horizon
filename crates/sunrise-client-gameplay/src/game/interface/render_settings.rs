//! The renderer and its settings: building it for the window, applying the video settings and a
//! shader pack's translucent layers, and the far terrain (LOD) distances.
use super::*;

#[derive(Default)]
pub(in crate::game) struct FrameAttempt<K = FrameKey>(Option<K>);

/// Requested frame, pick path, load outcome and HDR tonemap.
type FrameKey = (
    Option<sunrise_client::gfx::FrameConfiguration>,
    PathBuf,
    Option<bool>,
    Option<sunrise_shaders::Tonemap>,
);

fn look_again<C: PartialEq>(
    attempts: &mut FrameAttempt<(
        Option<C>,
        PathBuf,
        Option<bool>,
        Option<sunrise_shaders::Tonemap>,
    )>,
    wanted: Option<C>,
    path: PathBuf,
    outcome: Option<bool>,
    hdr: Option<sunrise_shaders::Tonemap>,
) -> bool {
    attempts.changed((wanted, path, outcome, hdr))
}

impl<K: PartialEq> FrameAttempt<K> {
    // Remember attempts, including failures, until the requested configuration changes.
    fn changed(&mut self, key: K) -> bool {
        let changed = self.0.as_ref() != Some(&key);
        self.0 = Some(key);
        changed
    }
}

impl crate::game::ClientGame<'_> {
    pub(in crate::game::client_side) fn renderer_for(
        &self,
        window: Arc<Window>,
        pack: &PackResources,
        client_pack: &crate::game::client_side::resource_tables_client::ClientPackResources,
        gui: &UiResources,
        opened: Option<sunrise_client::gfx::Opened>,
    ) -> Result<Box<Renderer>> {
        build_renderer(
            window,
            self.common.options.offscreen,
            pack,
            client_pack,
            gui,
            opened.map_or_else(
                || {
                    self.client.renderer.0.as_deref().map_or(
                        sunrise_client::gfx::RendererDevice::New,
                        sunrise_client::gfx::RendererDevice::Reuse,
                    )
                },
                |device| sunrise_client::gfx::RendererDevice::Opened(Box::new(device)),
            ),
            None,
            self.client.settings.sunrise.api,
            self.client.settings.sunrise.translucent_batching,
            self.effective_vsync(),
            &self.client.settings.sunrise,
            self.picked_pack_shape(),
            self.client
                .renderer
                .0
                .as_ref()
                .map(|r| r.display_reading())
                .unwrap_or_default(),
            self.client.settings.texture_atlas_settings(),
        )
        .and_then(|(renderer, clouds)| {
            clouds?;
            Ok(Box::new(renderer))
        })
    }
    /// Under `SUNRISE_HDR_FRAME=float` the frame follows what draws the picture: when HDR,
    /// Smooth Gradients or the pick (unless refused) call for another frame than the renderer's,
    /// it is made again before this frame draws. Without the switch a frame stays until the next
    /// start, as before.
    pub(in crate::game::client_side) fn follow_frame(&mut self) {
        if !native_shaders::linear_frame_switch() {
            return;
        }
        let picked = shader_packs::picked_pack(&self.client.settings);
        let dir = Self::shader_packs_dir();
        let path = dir.join(picked);
        let available = path.exists()
            && !sunrise_client::shader_pack_list::unsupported(&dir, picked)
            && path.file_name().is_none_or(|f| {
                !shader_packs::active_pack(&f.to_string_lossy(), &self.client.settings.mods)
                    .is_empty()
            });
        let outcome = if available {
            native_shaders::outcome(&path)
        } else {
            Some(false)
        };
        // The renderer's offer can change while the game runs (Vulkan's formats follow desktop
        // HDR and the window's output), and the frame with it.
        let shape = self.picked_pack_shape();
        let request = shape.frame();
        let wanted = self
            .client
            .renderer
            .0
            .as_ref()
            .map(|r| r.requested_frame(&self.client.settings.sunrise, request));
        if !look_again(
            &mut self.client.frame_attempted,
            wanted,
            path,
            outcome,
            shape.hdr,
        ) {
            return;
        }
        let Some(renderer) = &mut self.client.renderer.0 else {
            return;
        };
        let follows = renderer.frame_follows(&self.client.settings.sunrise, request);
        if !shader_packs::reconcile_pack_hdr(follows, &mut renderer.pack_hdr, shape.hdr) {
            return;
        }
        if let Err(e) = self.rebuild_renderer() {
            log::warn!("Sunrise: the renderer for the new frame could not be made: {e}");
        }
    }

    /// The renderer made again on the same device from the resources loaded, keeping the
    /// pipelines that come out the same and the world the old one holds (`Renderer::take_world`):
    /// only what draws in the frame is made again, and no section is meshed again.
    fn rebuild_renderer(&mut self) -> Result<()> {
        let Some(window) = self.client.window.0.clone() else {
            return Ok(());
        };
        let started = std::time::Instant::now();
        if let Some(renderer) = &mut self.client.renderer.0 {
            renderer.release_surface();
        }
        let mut renderer = match self.renderer_for(
            window,
            &self.common.pack,
            &self.client.client_pack,
            &self.client.gui,
            None,
        ) {
            Ok(renderer) => renderer,
            Err(error) => {
                if let Some(renderer) = &mut self.client.renderer.0 {
                    renderer.recreate_surface()?;
                }
                return Err(error);
            }
        };
        let mut world_kept = false;
        if let Some(old) = &mut self.client.renderer.0 {
            renderer.resize(old.config.width, old.config.height);
            renderer.screenshot = old.screenshot.take();
            renderer.screenshot_result = old.screenshot_result.take();
            world_kept = renderer.take_world(old);
        }
        self.adopt(renderer, world_kept);
        log::info!(
            "Sunrise: the renderer was made again for the frame the settings call for in {} ms",
            started.elapsed().as_millis()
        );
        Ok(())
    }

    pub fn apply_settings(&mut self) {
        self.apply_client_settings();
    }

    fn apply_streaming_settings(&mut self) {
        let streaming_target =
            sunrise_gameplay_common::game::streaming_radius::streaming_radius_for(
                self.client.settings.render_distance,
                self.client.remote.as_ref().map(|r| r.view_distance),
            );
        let render_distance_changed = self.client.streaming.radius != streaming_target;
        self.client.streaming.radius = streaming_target;
        // Options.renderDistance, biomeBlendRadius, ambientOcclusion,
        // cutoutLeaves, Opaque Ice and the leaf-culling mode:
        // LevelExtractor.allChanged when they change.
        let leaf_culling = self.effective_leaf_culling();
        let (s, st) = (&self.client.settings, &self.client.streaming);
        let reload = render_distance_changed
            || (
                s.biome_blend,
                s.smooth_lighting,
                s.cutout_leaves,
                s.sunrise.opaque_ice,
                leaf_culling,
            ) != (
                st.biome_blend,
                st.smooth_lighting,
                st.cutout_leaves,
                st.opaque_ice,
                st.leaf_culling,
            );
        self.client.streaming.biome_blend = self.client.settings.biome_blend;
        self.client.streaming.smooth_lighting = self.client.settings.smooth_lighting;
        self.client.streaming.cutout_leaves = self.client.settings.cutout_leaves;
        self.client.streaming.opaque_ice = self.client.settings.sunrise.opaque_ice;
        self.client.streaming.leaf_culling = leaf_culling;
        if reload {
            self.all_sections_changed();
        }
    }

    fn apply_client_settings(&mut self) {
        self.bind_mod_mappings();
        // LocalPlayer.refreshChatAbilities: the chat shows what the Chat option allows.
        let abilities = self.chat_abilities();
        self.client
            .chat_ui
            .chat
            .set_abilities(&self.client.gui.atlas.font, abilities);
        self.apply_streaming_settings();
        self.client.audio.0.master = self.client.settings.master_volume as f32;
        if self.client.settings.sound_device != self.client.audio.0.device() {
            let _ = self
                .client
                .audio
                .0
                .set_device(&self.client.settings.sound_device);
        }
        self.apply_ui_settings();
    }

    /// The sprite layers a shader pack draws in the translucent pass: every section is meshed
    /// again when they change.
    pub(in crate::game::client_side) fn set_translucent_layers(&mut self, layers: Vec<bool>) {
        if *layers != *self.client.streaming.translucent_layers {
            self.client.streaming.translucent_layers = layers.into();
            self.all_sections_changed();
        }
    }

    /// LOD Distance and Far Detail Reduced as the player chose them, the
    /// distance no farther than a server sampling far terrain for us allows.
    pub(in crate::game::client_side) fn far_terrain_choice(&self) -> (u32, bool) {
        let g = &self.client.settings.sunrise;
        let reduced = g.far_detail == sunrise_client::graphics::FarDetail::Reduced;
        let distance = match self.server_far_terrain() {
            Some(offer) => g.lod_distance.min(offer.distance),
            None => g.lod_distance,
        };
        (distance, reduced)
    }

    /// The far terrain another machine's server samples for us
    /// (`lod_net`): a client of a dedicated server or of a LAN world, not of
    /// its own server thread, which hands its generator over directly.
    pub(in crate::game::client_side) fn server_far_terrain(
        &self,
    ) -> Option<sunrise_sim::lod_net::Offer> {
        if self.client.integrated.is_some() {
            return None;
        }
        self.client.remote.as_ref()?.far_terrain
    }

    /// LOD Distance as far terrain uses it (`graphics::lod_distance_capped`).
    pub(in crate::game::client_side) fn lod_distance(&self) -> u32 {
        let (distance, _) = self.far_terrain_choice();
        match &self.client.renderer.0 {
            Some(renderer) => {
                sunrise_client::graphics::lod_distance_capped(distance, &renderer.info)
            }
            None => distance.min(sunrise_client::lod::MAX_CHUNKS),
        }
    }

    /// Only enabled LOD worlds extend the renderer's range, never simulation.
    pub fn terrain_distance(&self) -> i32 {
        let distance = self.effective_render_distance();
        if self.client.lod.0.is_some() {
            distance.max(self.lod_distance() as i32)
        } else {
            distance
        }
    }

    /// The renderer for the window, with the current block sprites, GUI
    /// atlas and world border texture.
    /// A new renderer's atlas holds only the pack's sprites: what was drawn into it at run time
    /// (skins, maps, mods' dynamic textures, the themed UI's glyphs and corners) has to be drawn
    /// again.
    pub(in crate::game::client_side) fn forget_atlas_slots(&mut self) {
        self.client.skin_slots.forget();
        self.client.map_slots.forget();
        self.client.trim_slots.forget();
        self.client.map_patches.clear();
        self.client.text_patches.get_mut().clear();
        self.client.kept_block_entities.get_mut().clear();
        sunrise_client::ui_theme::reset_atlas();
    }

    pub fn new_renderer(&self, window: Arc<Window>) -> Result<Box<Renderer>> {
        self.renderer_for(
            window,
            &self.common.pack,
            &self.client.client_pack,
            &self.client.gui,
            None,
        )
    }

    /// A renderer made again from the resources loaded takes over: what named the old one's
    /// atlas slots is forgotten, the world is meshed again, and the shader pack and the
    /// settings are applied again.
    pub(in crate::game::client_side) fn adopt_renderer(&mut self, renderer: Box<Renderer>) {
        self.adopt(renderer, false);
    }

    /// `adopt_renderer`; with `world_kept`, a renderer that took the old one's sections and far
    /// tiles (`Renderer::take_world`): they and the meshes on their way stay.
    fn adopt(&mut self, mut renderer: Box<Renderer>, world_kept: bool) {
        if !world_kept {
            // A new renderer starts with the vanilla column.
            renderer.clear_sections(self.common.world.layout());
        }
        // And has no wall for the world border. Its sections' sources are kept as the meshes
        // on their way carry them.
        self.client.border_renderer.invalidate();
        renderer.keep_pack_sources(self.client.streaming.shader_pack);
        self.set_renderer(renderer);
        self.bind_dynamic_textures();
        self.forget_atlas_slots();
        if !world_kept {
            self.clear_lod();
            self.client.pending_uploads.0.clear();
            let context = self.worker_context();
            self.client
                .streaming
                .reload_resources(&mut self.common.world, context);
        }
        self.apply_ui_settings();
    }

    /// `LevelExtractor.allChanged`: displayed sections and mesh work are
    /// discarded, then every loaded section is rebuilt.
    pub(in crate::game::client_side) fn all_sections_changed(&mut self) {
        self.clear_lod();
        self.client.pending_uploads.0.clear();
        if let Some(renderer) = self.client.renderer.0.as_mut() {
            renderer.clear_sections(self.common.world.layout());
        }
        self.client
            .streaming
            .reload_sections(&mut self.common.world);
    }
}

#[cfg(test)]
mod tests {
    use super::{FrameAttempt, look_again};
    use crate::game::shader_packs::reconcile_pack_hdr;
    use std::path::PathBuf;
    use sunrise_shaders::Tonemap;

    #[test]
    fn a_new_hdr_request_alone_is_looked_at_again() {
        let mut attempts = FrameAttempt::default();
        let path = PathBuf::from("pack");
        let mut installed = Some(Tonemap::Pack);
        let mut look =
            |hdr| look_again(&mut attempts, Some("8-bit"), path.clone(), Some(true), hdr);
        assert!(look(Some(Tonemap::Pack)));
        assert!(!look(Some(Tonemap::Pack)));
        assert!(look(Some(Tonemap::Sunrise)));
        assert!(!reconcile_pack_hdr(
            true,
            &mut installed,
            Some(Tonemap::Sunrise)
        ));
        assert_eq!(installed, Some(Tonemap::Sunrise));
        assert!(!look(Some(Tonemap::Sunrise)));
        assert!(look(None));
    }

    #[test]
    fn failed_frame_attempt_waits_for_a_policy_or_setting_change() {
        let mut attempts = FrameAttempt::default();
        let mut rebuilds = 0;
        // The requested frame stands for itself here: the 8-bit one, then the float one.
        for key in [
            ("8-bit", "p", None),
            ("8-bit", "p", None),
            // HDR output offered later, with HDR On: the frame is looked at again.
            ("float", "p", None),
            ("float", "p", Some(true)),
            ("float", "p", Some(true)),
            ("float", "p", Some(false)),
            ("float", "p", Some(false)),
            ("float", "q", None),
            ("8-bit", "q", None),
        ] {
            if attempts.changed(key) {
                // A replacement fails: no success is recorded, just the attempted key.
                rebuilds += 1;
            }
        }
        assert_eq!(rebuilds, 6);
    }
}
