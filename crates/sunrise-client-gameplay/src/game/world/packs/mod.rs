//! Resource pack selection bound to the game: the pack screen lists, and a
//! full resource reload after a change, while playing.
#[cfg(feature = "client")]
use super::{PackResources, Resources, build_renderer, prepare_renderer_textures};
#[cfg(feature = "client")]
use crate::game::client_side::pack_discovery_client::{data_packs_with_icons, into_resource_pack};
#[cfg(feature = "client")]
use crate::game::client_side::resource_tables_client::{
    ClientPackResources, ClientResources, UiResources,
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use sunrise_assets::resource_source::ResourceRoot;
#[cfg(feature = "client")]
use sunrise_client::gfx::PreparedTextures;
#[cfg(feature = "client")]
use sunrise_client::{hud::HudAtlas, level_loading::OverlayFade, pack_browser};
use sunrise_client::{
    pack_screen::{self, Compat, PackInfo, Source as PackSource},
    packs::{self, Pack, Position},
};
#[cfg(feature = "client")]
use sunrise_client_render::streaming::WorkerContext;
#[cfg(feature = "client")]
use sunrise_game_data::lang;
use sunrise_game_data::text::{Style, Text};
use sunrise_gameplay_common::game::packs_common::cache_dir;
#[cfg(feature = "client")]
use sunrise_level::assets::Assets;

pub fn packs_dir() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_default()
        .join("resourcepacks")
}
/// `Blaze3D.openPath`: the desktop's handler on the folder (the default file manager).
pub(in crate::game::client_side) fn open_folder(dir: &Path) {
    if let Err(e) = sunrise_client::desktop::open_path(dir) {
        log::warn!("Sunrise: could not open {}: {e}", dir.display());
    }
}
/// `Blaze3D.openUri`: the system browser on the link.
pub fn open_uri(uri: &str) {
    if let Err(error) = sunrise_client::desktop::open_uri(uri) {
        log::warn!("Sunrise: could not open URI {uri}: {error}");
    }
}
/// The accepted content root and the result of layering selected client packs over it.
/// Recovery keeps the required packs and uses the same content root.
pub struct PackRoots {
    pub base: ResourceRoot,
    pub selected: std::io::Result<ResourceRoot>,
    pub cleared: Option<Vec<Pack>>,
}

impl PackRoots {
    pub fn for_selection(
        base: ResourceRoot,
        all: &[Pack],
        ids: &[String],
        server: &[Pack],
    ) -> Self {
        let mut enabled = packs::enabled(all, ids);
        let cleared = packs::has_optional_pack(&enabled).then(|| packs::enabled(all, &[]));
        enabled.extend_from_slice(server);
        let selected = layer_client_packs(&base, &enabled);
        Self {
            base,
            selected,
            cleared,
        }
    }
}

impl From<ResourceRoot> for PackRoots {
    fn from(base: ResourceRoot) -> Self {
        Self {
            selected: Ok(base.clone()),
            base,
            cleared: None,
        }
    }
}

/// The selection and a server's packs (fixed at the top, `Pack.Position.TOP`) are laid over
/// [`crate::game::client_side::mods_client::content_root`], so required mod resources sit below the player's packs.
/// Built-in packs are found in `vanilla`.
#[cfg(feature = "client")]
pub fn startup_root(
    vanilla: &Path,
    selected: &[String],
    server: &[Pack],
    mod_sources: &super::mods::ModsState,
) -> sunrise_game_data::Result<PackRoots> {
    let mut all = packs::discover(vanilla, &packs_dir());
    all.extend(
        sunrise_gameplay_common::game::pack_discovery::discover(
            mod_api::PackType::ClientResources,
            mod_sources,
        )
        .into_iter()
        .map(into_resource_pack),
    );
    let base = crate::game::client_side::mods_client::content_root(vanilla, mod_sources)?;
    Ok(PackRoots::for_selection(base, &all, selected, server))
}

/// `Minecraft.clearResourcePacksOnError`: save the empty selection before rebuilding its layers.
pub(in crate::game) fn clear_selection(settings: &mut super::Settings) {
    log::info!("Caught error loading resourcepacks, removing all selected resourcepacks");
    settings.resource_packs.clear();
    settings.file.set(
        "incompatibleResourcePacks",
        sunrise_client::options_file::V::List(Vec::new()),
    );
    settings.save(&sunrise_gameplay_common::game::options_path());
}

/// Client packs overlay assets only, never the prepared tables or passive server data.
pub(in crate::game) fn layer_client_packs(
    base: &ResourceRoot,
    enabled: &[Pack],
) -> std::io::Result<ResourceRoot> {
    let source = packs::resources(base.source().clone(), enabled)?;
    Ok(
        ResourceRoot::new(base.prepared().to_path_buf(), cache_dir(), source)
            .with_data(base.data().clone(), base.data_changes().clone())
            .with_mod_layers(base.mod_layers().clone()),
    )
}
pub fn info(p: &Pack) -> PackInfo {
    if let Some(info) = &p.selection {
        return info.clone();
    }
    let built_in = !p.id.starts_with("file/");
    PackInfo {
        id: p.id.clone(),
        title: p.title.clone(),
        description: p.raw_description.clone(),
        description_style: p.description_style.clone(),
        source: if built_in {
            PackSource::BuiltIn
        } else {
            PackSource::Default
        },
        compat: p.compatibility,
        icon: p.icon_png().as_deref().and_then(pack_screen::load_icon),
        required: false,
        position: Position::Top,
        fixed: false,
        hidden: false,
    }
}
/// A pack from `DownloadedPackSource.loadRequestedPacks`: fixed at the top of
/// the selected list (`DOWNLOADED_PACK_SELECTION`), labelled as a server pack
/// (or a world's own pack when its id is the local-world uuid).
pub(in crate::game::client_side) fn downloaded_info(p: &Pack) -> PackInfo {
    // `WorldOpenFlows.WORLD_PACK_ID` as hex in `server/{uuid}/…`.
    const WORLD: &str = "server/640a6a92b6cb48a0b391831586500359/";
    let world = p.id.starts_with(WORLD);
    PackInfo {
        id: p.id.clone(),
        title: p.title.clone(),
        description: p.raw_description.clone(),
        description_style: p.description_style.clone(),
        source: if world {
            PackSource::World
        } else {
            PackSource::Server
        },
        compat: p.compatibility,
        icon: p.icon_png().as_deref().and_then(pack_screen::load_icon),
        required: true,
        position: Position::Top,
        fixed: true,
        hidden: false,
    }
}
/// The vanilla pack: required, at the bottom (`BuiltInPackSource`), with the
/// client JAR's `pack.png` that the prepared resources hold.
pub(in crate::game::client_side) fn vanilla_info(vanilla: &Path) -> PackInfo {
    PackInfo {
        id: "vanilla".into(),
        title: Text::translate("resourcePack.vanilla.name", &[]),
        description: Text::translate("resourcePack.vanilla.description", &[]),
        description_style: Style::default(),
        source: PackSource::BuiltIn,
        compat: Compat::Compatible,
        icon: std::fs::read(vanilla.join("pack.png"))
            .ok()
            .as_deref()
            .and_then(pack_screen::load_icon),
        required: true,
        position: Position::Bottom,
        fixed: false,
        hidden: false,
    }
}
/// Packs the Resource Packs screen lists: folder and built-in packs, then any
/// active downloaded server (or world) packs, then vanilla.
pub fn pack_infos(vanilla: &Path, server: &[Pack], additional: &[Pack]) -> Vec<PackInfo> {
    let mut infos: Vec<PackInfo> = packs::discover(vanilla, &packs_dir())
        .iter()
        .map(info)
        .collect();
    infos.extend(additional.iter().map(info));
    infos.extend(server.iter().map(downloaded_info));
    infos.push(vanilla_info(vanilla));
    infos
}
/// The folder packs' Polytone config entries (#11486), by pack id, for those that have some:
/// what the Pack Options page edits.
pub(in crate::game::client_side) fn pack_configs(
    vanilla: &Path,
) -> std::collections::BTreeMap<String, packs::Configs> {
    packs::discover(vanilla, &packs_dir())
        .into_iter()
        .filter(|p| !p.polytone_configs.entries().is_empty())
        .map(|p| (p.id, p.polytone_configs))
        .collect()
}

/// The confirmation's callback, shared by the startup menu and in-game options.
pub(in crate::game::client_side) fn import_resource_packs(
    ui: &mut sunrise_client::options_ui::OptionsUi,
    vanilla: &Path,
    server: &[Pack],
    paths: &[PathBuf],
    additional: &[Pack],
) {
    let dir = packs_dir();
    let mut failed = false;
    for path in paths {
        match packs::copy_pack(path, &dir) {
            Ok(()) => log::info!("Sunrise: added resource pack {}", path.display()),
            Err(error) => {
                log::warn!(
                    "Sunrise: could not copy resource pack {}: {error}",
                    path.display()
                );
                failed = true;
            }
        }
    }
    ui.update_pack_infos(pack_infos(vanilla, server, additional));
    ui.pack_configs = pack_configs(vanilla);
    if failed {
        ui.resource_error = Some(Text::translate("pack.copyFailure", &[]).plain());
    }
}
/// The player changed Pack Options: `chosen` replaces their choices for `entries`, kept in
/// `polytone-configs.json`.
pub(in crate::game::client_side) fn save_pack_options(
    entries: &[String],
    chosen: std::collections::BTreeMap<String, packs::ConfigValue>,
) {
    let mut all = packs::polytone_choices();
    all.retain(|id, _| !entries.contains(id));
    all.extend(chosen);
    if let Err(e) = packs::save_polytone_choices(&packs::polytone_choices_file(), &all) {
        log::warn!("could not save the pack options: {e}");
    }
    packs::set_polytone_choices(all);
}
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    pub(in crate::game::client_side) fn drop_pack_file(&mut self, path: &Path) {
        let size = self.gui_size();
        let handled = self.client.screen == super::Screen::Options
            && self.client.chat_ui.options.drop_resource_pack(
                path.to_path_buf(),
                &self.client.gui.atlas,
                size,
            );
        if !handled
            && self.title_input_active()
            && let Some(title) = &mut self.client.title.0
        {
            title.drop_file(path.to_path_buf());
        }
    }
    pub(in crate::game::client_side) fn resource_pack_infos(&mut self) -> Vec<PackInfo> {
        let additional: Vec<_> = self
            .discover_mod_packs(mod_api::PackType::ClientResources)
            .into_iter()
            .map(into_resource_pack)
            .collect();
        pack_infos(
            &self.common.vanilla_root(),
            &self.client.server_packs.0.active,
            &additional,
        )
    }
    /// Whether all active packs preserve vanilla's block models. The server
    /// overlay sits above the local selection and must be included too.
    pub(in crate::game::client_side) fn model_packs_are_vanilla(&self) -> bool {
        self.client
            .settings
            .sunrise
            .vanilla_models_with_server_packs(
                self.client
                    .settings
                    .resource_packs
                    .iter()
                    .map(String::as_str),
                self.client
                    .server_packs
                    .0
                    .active
                    .iter()
                    .map(|pack| pack.id.as_str()),
            )
    }
    /// The leaf mode that is safe for the current local and server packs.
    pub(in crate::game::client_side) fn effective_leaf_culling(
        &self,
    ) -> sunrise_client::graphics::LeafCullingMode {
        self.client
            .settings
            .sunrise
            .leaf_culling_effective_with_server_packs(
                self.client
                    .settings
                    .resource_packs
                    .iter()
                    .map(String::as_str),
                self.client
                    .server_packs
                    .0
                    .active
                    .iter()
                    .map(|pack| pack.id.as_str()),
            )
    }
    /// The packs the pack screen offers and the current selection
    /// (`PackSelectionScreen` makes its `PackSelectionModel` from them).
    pub fn refresh_pack_lists(&mut self) {
        let infos = self.resource_pack_infos();
        let configs = pack_configs(&self.common.vanilla_root());
        let browsing = self.client.settings.extras.pack_browser;
        let o = &mut self.client.chat_ui.options;
        o.pack_browser = browsing;
        o.pack_infos = infos;
        o.pack_configs = configs;
        o.pack_selection = self.client.settings.resource_packs.clone();
        o.pack_screen = None;
    }
    /// `Minecraft.reloadResourcePacks`: rebuild the overlay and reload every
    /// client resource from it, as vanilla's reload listeners do: block
    /// models and sprites (every section is meshed again,
    /// `LevelRenderer.allChanged`), the shared atlas (GUI, item, entity,
    /// particle and font textures), item models, sounds, languages, GUI
    /// metadata and the cloud and world border textures. The loading runs on
    /// a worker under the progress overlay (`LoadingOverlay` over
    /// `ReloadableResourceManager.createReload`'s background executor), so
    /// the window keeps drawing; `drive_pack_reload` applies it.
    /// Sunrise's pack browser: what its worker finished since the last
    /// frame. A pack that landed becomes an entry in the available list.
    pub(in crate::game::client_side) fn drive_pack_browser(&mut self) {
        let Some(screen) = &mut self.client.chat_ui.options.pack_screen else {
            return;
        };
        let Some(browser) = &mut screen.browser else {
            return;
        };
        let Some(message) = browser.poll() else {
            return;
        };
        if let pack_browser::Message::Results(_) = &message {
            // The rows draw their icons as they land.
            browser.fetch_icons();
        }
        if let pack_browser::Message::Icon(id, png) = &message {
            let key = pack_screen::icon_texture(id);
            if let Some(icon) = pack_screen::load_fetched_icon(png) {
                // Tracked, so closing the browser frees it again.
                screen.keep_browser_icon(key, icon);
            }
            return;
        }
        if let pack_browser::Message::Downloaded(name) = message {
            log::info!("Sunrise: added the resource pack {name}");
            // Rebuilding the screen would shut the list; keep it and only
            // refresh what the lists read.
            let selection = self.client.settings.resource_packs.clone();
            let infos = self.resource_pack_infos();
            self.client.chat_ui.options.pack_infos = infos;
            self.client.chat_ui.options.pack_configs = pack_configs(&self.common.vanilla_root());
            self.client.chat_ui.options.pack_selection = selection;
            self.client.chat_ui.options.pack_screen = None;
        }
    }
    pub(in crate::game) fn reload_packs(&mut self) {
        if let Err(e) = self.spawn_pack_reload(ReloadKind::Selection) {
            self.selection_reload_failed(&format!("could not start the pack reload: {e}"));
        }
    }
    /// NeoForge's `Minecraft.rollbackResourcePacks`: reset the selection and
    /// reload only if an optional pack was selected; otherwise show the error.
    pub fn selection_reload_failed(&mut self, error: &str) {
        log::warn!("Sunrise: resource reload failed: {error}");
        let mut all = packs::discover(&self.common.vanilla_root(), &packs_dir());
        all.extend(
            self.discover_mod_packs(mod_api::PackType::ClientResources)
                .into_iter()
                .map(into_resource_pack),
        );
        if packs::has_optional_pack(&packs::enabled(&all, &self.client.settings.resource_packs)) {
            clear_selection(&mut self.client.settings);
            if self.spawn_pack_reload(ReloadKind::Selection).is_ok() {
                // addResourcePackLoadFailToast(null), SystemToast.addOrUpdate.
                let title = sunrise_game_data::text::Text::translate("resourcePack.load_fail", &[]);
                self.add_or_update_system_toast(
                    super::toast_manager::SystemToastId::PackLoadFailure,
                    sunrise_client::toasts::SystemToast::new(title, None),
                );
                return;
            }
        }
        if self.client.screen != super::Screen::Options {
            self.open_options_at(
                self.client.title.0.is_some(),
                sunrise_client::options_ui::Page::Packs,
            );
        }
        self.client.chat_ui.options.resource_error = Some(error.to_string());
    }
    /// Stages the selection with the server's active packs on top on a
    /// worker (`createReload`'s background executor); the frame draws its
    /// progress meanwhile, and the overlay takes no input (`LoadingOverlay`),
    /// so what is held is released (`KeyMapping.releaseAll`).
    pub fn spawn_pack_reload(&mut self, kind: ReloadKind) -> std::io::Result<()> {
        // A selection reload, including startup rollback, needs tail-owned mob metadata.
        // An in-flight reload finishes first (vanilla's `pendingReload`).
        if kind == ReloadKind::Selection
            && reload_waits(
                self.common.rest_loader.is_some(),
                self.client.pack_reload.0.is_some(),
            )
        {
            self.client.pending_pack_reload = true;
            return Ok(());
        }
        self.common
            .join_world_rest()
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        self.clear_device_keys();
        self.client.physical_keys.clear();
        self.set_mapping_held("key.attack", false);
        self.set_mapping_held("key.use", false);
        let vanilla = self.common.vanilla_root();
        if self.common.ui.vanilla_root.as_os_str().is_empty() {
            self.common.ui.vanilla_root = vanilla.clone();
        }
        let enabled = self.applied_mods();
        // Only content-only packages are selectable on reload. Keep code packages' validated
        // startup manifests: changing their content must not install code or registries live.
        self.common.mods.selected = sunrise_mod_package::selection::content_packages(
            &sunrise_gameplay_common::game::game_dir(),
            &enabled,
            &self.common.mods.dev,
        )
        .filter(|p| {
            !self
                .common
                .mods
                .native
                .built()
                .iter()
                .any(|built| built.id == p.package.manifest.id)
        })
        .collect();
        self.common.mods.refused = Default::default();
        let mod_sources = self.common.mods.clone();
        let (selected, server) = (
            self.client.settings.resource_packs.clone(),
            self.client.server_packs.0.active.clone(),
        );
        let (assets, res, context) = (
            self.common.pack.assets.clone(),
            self.common.res.clone(),
            self.client.client_pack.context.clone(),
        );
        let client_res_slot = self.client.client_res.clone();
        let (unicode, japanese) = (
            self.client.settings.force_unicode,
            self.client.settings.japanese_glyphs,
        );
        // The reload's mod listeners register here, on the thread that owns
        // the game, before the staging worker starts: each listener keeps its
        // registering mod for crash attribution.
        let max_atlas_size = self
            .client
            .renderer
            .0
            .as_ref()
            .map_or(sunrise_client::hud::MAX_ATLAS_SIZE, |renderer| {
                renderer.device.limits().max_texture_dimension_2d
            });
        let captured = self
            .common
            .capture_candidate_settings(&self.common.options.resources)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        let client_listeners = self.register_client_reload_listeners(&captured);
        let texture_atlases = super::client_reload::collect_texture_atlases();
        let emissive_sprites = super::client_reload::collect_emissive_sprites();
        crate::game::client_side::mods_client::register_fluid_models(
            self.common.options.execution_role,
        );
        let stage = move |progress: &sunrise_gameplay_common::game::Progress| {
            let Some(client_res) = client_res_slot.get() else {
                return Err("client resources not loaded".to_owned());
            };
            stage_pack_with_emissive(
                &vanilla,
                &selected,
                &server,
                &mod_sources,
                client_listeners,
                &texture_atlases,
                &emissive_sprites,
                &assets,
                &res,
                client_res,
                &context,
                unicode,
                japanese,
                max_atlas_size,
                progress,
            )
            .map(|mut staged| {
                staged.settings = Some(captured);
                staged
            })
            .map_err(|e| e.to_string())
        };
        #[cfg(test)]
        let stage = mod_api::testing::inherit_mods(stage);
        self.client.pack_reload.0 = Some(PackReload::spawn(kind, stage)?);
        self.client.reload_overlay = Some(ReloadOverlay::default());
        Ok(())
    }
    pub(in crate::game) fn reload_overlay_visible(&self) -> bool {
        self.client.pack_reload.0.is_some() || self.client.reload_overlay.is_some()
    }

    /// One frame of the pack reload's `LoadingOverlay`: its bar eased toward
    /// the reload's progress (all of it once applied) and how far it has
    /// faded. `None`, and the overlay dropped, once it has faded out.
    pub(in crate::game::client_side) fn reload_overlay_frame(
        &mut self,
    ) -> Option<(f32, OverlayFade)> {
        let overlay = self.client.reload_overlay.as_mut()?;
        let now = Instant::now();
        let since = |at: Instant| now.duration_since(at).as_millis() as u64;
        let fade_in = since(*overlay.fade_in.get_or_insert(now));
        let Some(fade) = OverlayFade::at(fade_in, overlay.fade_out.map(since)) else {
            self.client.reload_overlay = None;
            return None;
        };
        let actual = self
            .client
            .pack_reload
            .0
            .as_ref()
            .map_or(1.0, |reload| reload.progress.fraction());
        overlay.bar = sunrise_client::level_loading::ease_overlay(overlay.bar, actual);
        Some((overlay.bar, fade))
    }
    /// `LoadingOverlay.tick` once the reload is done: the staged pack is
    /// applied on the render thread, and its outcome goes to whoever asked
    /// (`reloadResourcePacks`' callback, or `PackReloadConfig.Callbacks`).
    pub fn drive_pack_reload(&mut self) {
        let Some((kind, staged)) = self
            .client
            .pack_reload
            .0
            .as_mut()
            .and_then(PackReload::finished)
        else {
            return;
        };
        self.client.pack_reload.0 = None;
        let result =
            staged.and_then(|staged| self.apply_staged_pack(staged).map_err(|e| e.to_string()));
        if let Some(overlay) = &mut self.client.reload_overlay {
            overlay.fade_out = Some(Instant::now());
        }
        match kind {
            ReloadKind::Selection => {
                self.mods_reloaded(result.is_ok());
                if let Err(e) = result {
                    self.selection_reload_failed(&e);
                }
            }
            ReloadKind::Server => {
                if let Err(e) = &result {
                    log::warn!("Sunrise: server resource pack reload failed: {e}");
                }
                self.finish_pack_reload(result.is_ok());
            }
        }
    }
    /// The render-thread half: the renderer on the new pack's textures, then
    /// the swap (`ReloadInstance`'s application listeners).
    pub fn apply_staged_pack(&mut self, staged: StagedPack) -> sunrise_game_data::Result<()> {
        let StagedPack {
            root,
            client_applies,
            gui,
            pack,
            client_pack,
            textures,
            cem,
            block_entity_shapes,
            waypoint_styles,
            armor_stand_assets,
            entity_models,
            frame_models,
            sounds,
            language,
            settings,
        } = staged;
        // All fallible preflight finishes before any listener or the game
        // publishes the candidate. In particular, registration can refuse it.
        let loop_context = match settings {
            Some(captured) => self.common.prepare_loop_context_from(captured),
            None => self.common.prepare_loop_context(&root)?,
        };
        let professions = self.prepare_client_professions()?;
        let item_extensions = Self::prepare_client_item_extensions(
            &gui,
            &mut self.client.client_item_extension_owners,
        );
        // A reload from the title can apply before the start's world-rest worker is done.
        self.common.join_world_rest()?;
        let tab_modifiers = sunrise_client::polytone_creative_tabs::TabModifier::read_all(&root);
        let standalone_geometry = super::native_block_entity_draw::StandaloneGeometry::new(
            &pack,
            &client_pack,
            &gui,
            self.try_block_entity_render(),
        );
        self.client.native_shaders.pick_hdr.get_mut().forget();
        // Keep the working GPU resources, and hand its window surface over.
        if let Some(renderer) = &mut self.client.renderer.0 {
            renderer.release_surface();
        }
        let renderer = match self
            .client
            .window
            .0
            .clone()
            .map(|window| {
                build_renderer(
                    window,
                    self.common.options.offscreen,
                    &pack,
                    &client_pack,
                    &gui,
                    self.client.renderer.0.as_deref().map_or(
                        sunrise_client::gfx::RendererDevice::New,
                        sunrise_client::gfx::RendererDevice::Reuse,
                    ),
                    Some(textures),
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
                .and_then(|(renderer, required)| {
                    required?;
                    Ok(Box::new(renderer))
                })
            })
            .transpose()
        {
            Ok(renderer) => renderer,
            Err(error) => {
                if let Some(renderer) = &mut self.client.renderer.0 {
                    renderer.recreate_surface().map_err(|restore| {
                        format!(
                            "{error}; could not restore the previous rendering surface: {restore}"
                        )
                    })?;
                }
                return Err(error);
            }
        };
        let skip = self
            .client
            .settings
            .extras
            .skip_modded_splashes_if_vanilla_modified;
        let gui = gui.with_splashes(&pack.source, &self.common.vanilla_root(), skip);
        let has_renderer = renderer.is_some();
        let upload = self.publish_client_atlases(&client_pack.render, gui, renderer);
        // With the atlas visible, admit the models at the model manager's point
        // in the listener order. All renderer and model validation is finished.
        self.apply_client_reload(client_applies, |game: &mut super::ClientGame<'_>| {
            // Refresh modifiers from the newly staged packs, including removals.
            game.client
                .creative_screen
                .0
                .set_pack_modifiers(tab_modifiers);
            game.refresh_creative_search_names();
            [
                game.client.frame_model,
                game.client.glow_frame_model,
                game.client.map_frame_model,
                game.client.glow_map_frame_model,
            ] = frame_models;
            game.client.waypoint_styles = waypoint_styles;
            game.client
                .armor_stand_assets
                .0
                .apply_resources(armor_stand_assets);
            game.client.armor_stand_assets.0.extensions = item_extensions;
            game.client.entity_models = entity_models;
            game.side_clear_random_tick_sections();
            game.portal_sections.clear();
            game.options.resources = root.clone();
            game.client.falling_vertices.get_mut().clear();
            game.pack = pack;
            game.client.client_pack = client_pack;
            // The joined level's environment for the frame has the packs' layers over its type.
            game.build_level_environments();
            game.client.standalone_geometry = standalone_geometry;
            game.client.instance_groups.borrow_mut().clear();
            game.client.prepared_items.borrow_mut().clear();
            if let Some(renderer) = &game.client.renderer.0 {
                renderer.clear_mod_meshes();
                renderer.clear_mod_uniforms();
            }
            // The new atlas's skin and map slots are blank: players and maps
            // upload into it again instead of naming sprites it does not have.
            game.client.skin_slots.forget();
            game.client
                .skin_downloads
                .0
                .borrow_mut()
                .set_resource_root(game.pack.source.clone());
            game.client.map_slots.forget();
            game.client.map_patches.clear();
            game.client.text_patches.get_mut().clear();
            // Kept mod block entities name the old atlas's sprites, and their view distances the old
            // registry's states.
            game.client.kept_block_entities.get_mut().clear();
            game.client.live_scenes.0.get_mut().clear();
            game.client.mod_view_distances.take();
            // The atlas's sprites moved: the particle engine re-reads every rect
            // and the packs' sprite sets, or every particle draws a stale region.
            if !game.is_logical_server() {
                game.client.particles.0.reload(
                    &game.common.pack.source,
                    &game.client.gui.atlas.sprites,
                    &game.client.gui.atlas.translucent,
                );
            }
            // Villager hats come from the pack's texture metadata (shared data, swapped in place).
            *game
                .client_res()
                .client_mob_data()
                .villager_hats
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                sunrise_client::mobs::villager_hats::VillagerHats::load(&game.pack.source);
            game.install_client_professions(&professions);
            *game
                .client_res()
                .client_mob_data()
                .cem
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = cem;
            game.client_res()
                .block_entity_render()
                .set_shapes(block_entity_shapes);
            // The random entity textures and emissive overlays the new atlas packs. A new scan is
            // a new generation: every mob chooses again on its next tick.
            *game
                .client_res()
                .client_mob_data()
                .random_textures
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                std::mem::take(&mut game.client.gui.atlas.random_textures);
            *game
                .client_res()
                .client_mob_data()
                .emissive
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                game.client.gui.atlas.emissive.clone();
            *game
                .client_res()
                .client_mob_data()
                .blinks
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                std::mem::take(&mut game.client.gui.atlas.blinks);
            // Read with the language in use when the reload started: one picked since then wins.
            let code = lang::code_snapshot();
            lang::set(if language.code == code {
                language
            } else {
                lang::Language::load(&[&game.pack.source], &code)
            });
            // Book search terms and resolved result icons belong to these resources.
            game.client.recipe_ui.0.reset_screen();
            game.refresh_remote_language();
            game.client
                .audio
                .0
                .reload_with(&game.common.pack.source, sounds);
            game.clear_lod();
            // A pack that may replace leaf models falls back to `Off` before the
            // reload marks every section dirty.
            game.client.streaming.leaf_culling = game.effective_leaf_culling();
            let context = game.worker_context();
            game.client
                .streaming
                .reload_resources(&mut game.common.world, context);
            game.client.pending_uploads.0.clear();
            game.install_model_upload(upload);
            if has_renderer {
                game.apply_ui_settings();
            }
            game.client.chat_ui.chat.rescale(&game.client.gui.atlas);
            game.install_loop_context(loop_context);
            game.resolve_entity_renderers();
            game.native_model_baking_completed();
        });
        self.native_client_resources_reloaded();
        Ok(())
    }
}

/// The CPU half of a pack reload (`PackReload` runs it on a worker;
/// `apply_staged_pack` does the GPU half and the swap): the overlay
/// root, the models and sprites, the menu bundle, the pack's tables and its
/// CPU mip chains.
#[cfg(all(feature = "client", any(test, feature = "test-support")))]
pub fn stage_pack(
    vanilla_root: &Path,
    selected: &[String],
    server: &[Pack],
    mod_sources: &super::mods::ModsState,
    client_listeners: mod_api::ClientListenerRegistry,
    texture_atlases: &[mod_api::Answered<mod_api::TextureAtlasRegistration>],
    assets: &Arc<Assets>,
    res: &Resources,
    client_res: &ClientResources,
    context: &WorkerContext,
    force_unicode: bool,
    japanese: bool,
    progress: &super::Progress,
) -> sunrise_game_data::Result<StagedPack> {
    stage_pack_with_emissive(
        vanilla_root,
        selected,
        server,
        mod_sources,
        client_listeners,
        texture_atlases,
        &[],
        assets,
        res,
        client_res,
        context,
        force_unicode,
        japanese,
        sunrise_client::hud::MAX_ATLAS_SIZE,
        progress,
    )
}

#[cfg(feature = "client")]
fn stage_pack_with_emissive(
    vanilla_root: &Path,
    selected: &[String],
    server: &[Pack],
    mod_sources: &super::mods::ModsState,
    client_listeners: mod_api::ClientListenerRegistry,
    texture_atlases: &[mod_api::Answered<mod_api::TextureAtlasRegistration>],
    emissive_sprites: &[mod_api::Answered<mod_api::EmissiveSpriteRegistration>],
    assets: &Arc<Assets>,
    res: &Resources,
    client_res: &ClientResources,
    context: &WorkerContext,
    force_unicode: bool,
    japanese: bool,
    max_atlas_size: u32,
    progress: &super::Progress,
) -> sunrise_game_data::Result<StagedPack> {
    let entity_models_doc = sunrise_client::mobs::EntityModelDocument::load(vanilla_root)
        .map_err(|error| format!("loading vanilla entity model definitions: {error}"))?;
    let resource_root = startup_root(vanilla_root, selected, server, mod_sources)?.selected?;
    let resource_root = super::client_reload::stage_texture_atlases(resource_root, texture_atlases);
    let root = resource_root.prepared().to_path_buf();
    let source = resource_root.source().clone();
    progress.set(1);
    let (staged, client_applies) = super::client_reload::stage_client_reload(
        &source,
        client_listeners,
        || -> sunrise_game_data::Result<_> {
            // The world's models and sprites and the GUI atlas read the same root and nothing of
            // each other: about a second each, side by side.
            // The sound events and the language are read here too, not in
            // the frame that applies the reload (about 20 ms of it).
            let (render, atlas, panorama, (sounds, language)) = std::thread::scope(|s| {
                let atlas = sunrise_gameplay_common::game::resource_load::named_scope(
                    s,
                    "pack-assets",
                    || {
                        let game = sunrise_assets::resource_source::ResourceSource::directory(
                            vanilla_root.to_path_buf(),
                        );
                        HudAtlas::build_over_and_limit(&source, &source, &game, max_atlas_size)
                            .map_err(|e| e.to_string())
                    },
                );
                let panorama = sunrise_gameplay_common::game::resource_load::named_scope(
                    s,
                    "pack-assets",
                    || sunrise_client::gfx::panorama::Faces::load(&source, &source),
                );
                let small = sunrise_gameplay_common::game::resource_load::named_scope(
                    s,
                    "pack-assets",
                    || {
                        (
                            sunrise_level::sound_events::SoundRegistry::load(&source),
                            lang::Language::load(&[&source], &lang::code()),
                        )
                    },
                );
                (
                    sunrise_client::assets::RenderAssets::load_with_emissive(
                        &resource_root,
                        assets,
                        emissive_sprites,
                    ),
                    atlas.join(),
                    panorama.join(),
                    small
                        .join()
                        .unwrap_or_else(|p| std::panic::resume_unwind(p)),
                )
            });
            let render = Arc::new(render?);
            let mut atlas = atlas.unwrap_or_else(|p| std::panic::resume_unwind(p))?;
            atlas.font.uniform = force_unicode;
            atlas.font.japanese = japanese;
            let gui = UiResources {
                atlas,
                panorama: Arc::new(panorama.unwrap_or_else(|p| std::panic::resume_unwind(p))),
                // `Game::apply_staged_pack` fills it with the client's setting.
                splashes: Arc::from(""),
                plain_texts: Default::default(),
            };
            progress.set(2);
            let (mut pack, client_pack) = PackResources::load(
                &resource_root,
                assets.clone(),
                render,
                res.facts(),
                context.chunk.biomes.clone(),
                context.chunk.light.clone(),
                &res.core.item_data,
            )?;
            pack.additional_data = data_packs_with_icons(mod_sources).into();
            let textures = prepare_renderer_textures(&client_pack);
            let cem = Arc::new(sunrise_client::mobs::cem::CemModels::load(
                &source,
                client_res.client_mob_models(),
            ));
            let block_entity_shapes = client_res.block_entity_render().shapes_for(&source);
            let waypoint_styles = sunrise_client::locator_bar::WaypointStyles::load(&source);
            let armor_stand_assets =
                crate::game::client_side::armor_stands_client::ArmorStandAssets::load(
                    &source, None,
                );
            let entity_models = super::EntityModels::load(Some(&entity_models_doc));
            let frame_models = crate::game::client_side::frames_client::bake_frames(
                client_pack.render.model_library.as_deref().ok(),
            );
            progress.set(3);
            Ok(move |client_applies| StagedPack {
                root,
                client_applies,
                gui,
                pack,
                client_pack,
                textures,
                cem,
                block_entity_shapes,
                waypoint_styles,
                armor_stand_assets,
                entity_models,
                frame_models,
                sounds,
                language,
                settings: None,
            })
        },
    )
    .map_err(|error| {
        error
            .downcast::<mod_api::__private::reload::ReloadStageError>()
            .map_or_else(|error| error.to_string(), |error| error.diagnostic())
    })?;
    Ok(staged(client_applies))
}

/// Everything a pack reload builds on the CPU (`apply_staged_pack` finishes
/// it on the render thread).
#[cfg(feature = "client")]
pub struct StagedPack {
    pub root: PathBuf,
    pub waypoint_styles: sunrise_client::locator_bar::WaypointStyles,
    pub armor_stand_assets: crate::game::client_side::armor_stands_client::ArmorStandAssets,
    pub frame_models: [crate::game::client_side::frames_client::FrameModel; 4],
    /// The mod reload listeners' prepared applies, ordered around publication
    /// of the staged models.
    pub client_applies: Vec<super::client_reload::PreparedClientReload>,
    pub gui: UiResources,
    pub pack: PackResources,
    pub client_pack: ClientPackResources,
    pub textures: PreparedTextures,
    /// The packs' custom entity models (OptiFine CEM).
    pub cem: Arc<sunrise_client::mobs::cem::CemModels>,
    /// The block entities' models, the packs' custom entity models over vanilla's.
    pub block_entity_shapes: Arc<sunrise_client::block_entity_render::BlockEntityShapes>,
    /// The root's sound events and current language.
    pub sounds: sunrise_level::sound_events::SoundRegistry,
    pub language: lang::Language,
    pub(in crate::game) entity_models: super::EntityModels,
    pub(in crate::game) settings: Option<sunrise_world::native_load_context::SettingsCapture>,
}

/// A pack reload's stages, indexed by `progress`: the overlay draws how far
/// through them it is (`ReloadInstance.getActualProgress`), not their names.
pub(in crate::game) const RELOAD_LINES: [&str; 4] = [
    "Preparing resources",
    "Loading assets",
    "Loading pack tables",
    "Applying resources",
];
/// Whether a reload asked for now waits: for the world's resources, or for
/// the reload already under its overlay (`Minecraft.reloadResourcePacks`
/// hands back `pendingReload` while a `LoadingOverlay` is up).
pub fn reload_waits(world_loading: bool, in_flight: bool) -> bool {
    world_loading || in_flight
}
/// Who asked for a pack reload: its outcome goes back there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReloadKind {
    /// The player's own selection (`Minecraft.reloadResourcePacks`).
    Selection,
    /// A server's packs (`DownloadedPackSource.startReload`).
    Server,
}
#[cfg(feature = "client")]
pub type StageResult = std::result::Result<StagedPack, String>;
/// A pack reload whose CPU half runs on a worker while `progress` feeds the
/// loading overlay; `Game::drive_pack_reload` applies it on the render thread.
#[cfg(feature = "client")]
pub struct PackReload {
    pub progress: super::Progress,
    pub(in crate::game) kind: ReloadKind,
    pub handle: Option<std::thread::JoinHandle<StageResult>>,
    /// When the overlay went up. `LoadingOverlay.fadeInStart`, set on the
    /// first frame it draws.
    pub(in crate::game) started: Instant,
    /// How long the overlay stays up before the reload may be applied.
    pub hold: Duration,
}
/// `LoadingOverlay` over a pack reload, from its first frame until it has
/// faded out (`Minecraft.setOverlay(null)`), two seconds after the reload.
#[derive(Default)]
#[cfg(feature = "client")]
pub struct ReloadOverlay {
    /// `fadeInStart`: the overlay's first frame.
    pub(in crate::game) fade_in: Option<Instant>,
    /// `fadeOutStart`: when the reload was applied.
    pub(in crate::game) fade_out: Option<Instant>,
    /// `currentProgress`: the bar as drawn.
    pub(in crate::game) bar: f32,
}
/// `LoadingOverlay.isReadyToFadeOut`: a reload started from inside the game
/// (`fadeIn`) is not ready while `Util.getMillis() - fadeInStart < 1000L`, so
/// a pack that stages in a few frames still shows its overlay for a second
/// instead of flashing.
pub const OVERLAY_HOLD: Duration = Duration::from_millis(1000);
#[cfg(feature = "client")]
impl PackReload {
    /// Starts `stage` on its own thread and returns at once.
    pub fn spawn(
        kind: ReloadKind,
        stage: impl FnOnce(&super::Progress) -> StageResult + Send + 'static,
    ) -> std::io::Result<Self> {
        Self::spawn_holding(kind, OVERLAY_HOLD, stage)
    }
    /// `spawn` with the overlay's hold spelled out, so a test need not wait it.
    pub fn spawn_holding(
        kind: ReloadKind,
        hold: Duration,
        stage: impl FnOnce(&super::Progress) -> StageResult + Send + 'static,
    ) -> std::io::Result<Self> {
        let progress = super::Progress::with(&RELOAD_LINES);
        let worker = progress.clone();
        let handle = std::thread::Builder::new()
            .stack_size(sunrise_threads::GAME_THREAD_STACK_BYTES)
            .name("pack-reload".into())
            .spawn(move || stage(&worker))?;
        Ok(Self {
            progress,
            kind,
            handle: Some(handle),
            started: Instant::now(),
            hold,
        })
    }
    pub(in crate::game::client_side) fn kind(&self) -> ReloadKind {
        self.kind
    }
    /// The staged pack once the worker is done and the overlay has had its
    /// second (taken once); none while either is outstanding.
    pub fn finished(&mut self) -> Option<(ReloadKind, StageResult)> {
        if self.started.elapsed() < self.hold || !self.handle.as_ref()?.is_finished() {
            return None;
        }
        let staged = self
            .handle
            .take()?
            .join()
            .unwrap_or_else(|_| Err("the pack reload worker panicked".into()));
        Some((self.kind, staged))
    }
}

use sunrise_gameplay_common::game::side_ops::tables::TablesSideGameExt as _;

use sunrise_gameplay_common::game::mods::ModGameExt1 as _;

use sunrise_gameplay_common::game::pack_finders_common::CommonGameExt1 as _;

use crate::game::client_side::resource_tables_client::PackResourcesClientExt as _;
