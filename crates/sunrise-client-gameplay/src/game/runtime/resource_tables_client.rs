use super::*;
use std::path::Path;
use sunrise_client_render::streaming::{MeshWorkerContext, WorkerContext};
use sunrise_gameplay_common::game::api_values::{self, TryIntoApi};
use sunrise_gameplay_common::game::resource_tables::PackParts;
use sunrise_level::chunk_streamer::ChunkWorkerContext;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// A title screen over the current resources and their splash pool.
    pub fn new_title(&self, saves: PathBuf) -> Title {
        Title::new(
            saves,
            &self.common.options.resources,
            &self.client.gui.splashes,
        )
    }
}

pub(in crate::game) trait PackResourcesClientExt: Sized {
    fn load(
        root: &sunrise_assets::resource_source::ResourceRoot,
        assets: Arc<Assets>,
        render: Arc<RenderAssets>,
        facts: &BlockFacts,
        biomes: Arc<Biomes>,
        light: Arc<LightTables>,
        item_data: &sunrise_world::item_defaults::ItemData,
    ) -> Result<(PackResources, ClientPackResources)>;
}
impl PackResourcesClientExt for PackResources {
    /// Everything a pack overlay at `root` provides, over `assets` and `render`
    /// loaded from it.
    fn load(
        root: &sunrise_assets::resource_source::ResourceRoot,
        assets: Arc<Assets>,
        render: Arc<RenderAssets>,
        facts: &BlockFacts,
        biomes: Arc<Biomes>,
        light: Arc<LightTables>,
        item_data: &sunrise_world::item_defaults::ItemData,
    ) -> Result<(PackResources, ClientPackResources)> {
        let visibility = models::load_visibility(root.prepared(), assets.registry)?;
        api_values::resolve_block_definitions_for_role(
            assets.registry,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
        )?;
        mods::extend_poi_types(assets.registry);
        mods::register_cauldron_fluid_contents(assets.registry);
        let tables = Arc::new(
            MeshTables::new(
                &assets,
                &render,
                facts,
                visibility,
                crate::game::client_side::api_values_client::mod_tint_values(assets.registry)?,
            )?
            .with_polytone_colormaps(root.source(), assets.registry),
        );
        let item_renderer = sunrise_client::item_render::ItemRenderer::load_with_library(
            root,
            render.model_library.clone(),
            item_data,
        )?;
        let pack = Self::assemble(
            root,
            PackParts {
                assets,
                biomes,
                light,
                item_models: Arc::clone(&item_renderer.models),
                item_ids: Arc::clone(item_renderer.item_ids()),
            },
        )?;
        let client_pack = ClientPackResources::assemble(
            root,
            &pack,
            ClientPackParts {
                render,
                tables,
                item_renderer,
            },
            true,
        )?;
        Ok((pack, client_pack))
    }
}

impl PlainTexts {
    /// `draw` with the `Text` of `text`, made the first time it is asked for.
    pub(in crate::game) fn with<R>(
        &self,
        text: &str,
        draw: impl FnOnce(&sunrise_game_data::text::Text) -> R,
    ) -> R {
        let mut texts = self.0.borrow_mut();
        if !texts.contains_key(text) {
            if texts.len() >= Self::MAX {
                texts.clear();
            }
            texts.insert(text.into(), sunrise_game_data::text::Text::literal(text));
        }
        draw(&texts[text])
    }
}

impl ClientResources {
    pub(in crate::game) fn ambient(&self) -> &sunrise_client::ambience::AmbientTables {
        &self.ambient
    }
    pub fn animate(&self) -> &sunrise_client::animate_tick::AnimateTables {
        &self.animate
    }
    pub(in crate::game) fn block_entity_render(&self) -> &BlockEntityRenderer {
        &self.block_entity_render
    }
    pub(in crate::game) fn fog(&self) -> &sunrise_client::fog::FogTables {
        &self.fog
    }
    pub(in crate::game) fn music(&self) -> &sunrise_client::music::MusicTables {
        &self.music
    }
    pub(in crate::game) fn player_arm(
        &self,
        slim: bool,
        left: bool,
    ) -> &Option<sunrise_client::mobs::Part> {
        let arms = if slim {
            &self.player_arm_slim
        } else {
            &self.player_arm
        };
        &arms[usize::from(left)]
    }
    pub(in crate::game) fn spin_attack(&self) -> &Option<sunrise_client::mobs::Part> {
        &self.spin_attack
    }
    pub fn player_type(&self) -> &Option<Arc<sunrise_sim::mobs::MobType>> {
        &self.player_type
    }
    pub fn client_mob_models(&self) -> &sunrise_client::mobs_render::ClientMobModels {
        &self.client_mobs.models
    }

    pub fn client_mob_data(&self) -> &sunrise_client::mobs_render::ClientMobData {
        &self.client_mobs.data
    }
}

impl UiResources {
    /// The resolved resource generation, including the base layer and pack filters, over the
    /// game's own resources at `vanilla` (which tell the packs' core shaders from the game's).
    /// Its splash pool is empty until `with_splashes`.
    #[cfg(test)]
    pub fn load(
        root: &sunrise_assets::resource_source::ResourceSource,
        vanilla: &Path,
    ) -> Result<Self> {
        Self::load_with_limit(root, vanilla, sunrise_client::hud::MAX_ATLAS_SIZE)
    }
    /// Build the title resources within the supplied device texture dimension.
    pub fn load_with_limit(
        root: &sunrise_assets::resource_source::ResourceSource,
        vanilla: &Path,
        max_atlas_size: u32,
    ) -> Result<Self> {
        // The title screen waits for both: the panorama's six faces decode
        // beside the atlas.
        std::thread::scope(|scope| {
            let panorama = named_scope(scope, "panorama", || {
                startup::time("ui.panorama", || {
                    sunrise_client::gfx::panorama::Faces::load(root, root)
                })
            });
            let game = sunrise_assets::resource_source::ResourceSource::directory(vanilla.into());
            let atlas = startup::time("ui.atlas", || {
                HudAtlas::build_over_and_limit(root, root, &game, max_atlas_size)
            });
            let panorama = panorama
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
            Ok(Self {
                atlas: atlas?,
                panorama: Arc::new(panorama),
                splashes: Arc::from(""),
                plain_texts: PlainTexts::default(),
            })
        })
    }
    /// The provisional dimensions may have refused sprites even when the base HUD itself fits.
    /// Rebuild the final stitch from the accepted staged source on either limit change.
    pub(in crate::game) fn stitch_for_device(
        &mut self,
        source: &sunrise_assets::resource_source::ResourceSource,
        vanilla: &Path,
        limit: u32,
    ) -> Result<bool> {
        if self.atlas.stitch_limit == Some(limit) {
            return Ok(false);
        }
        let game =
            sunrise_assets::resource_source::ResourceSource::directory(vanilla.to_path_buf());
        let mut atlas = HudAtlas::build_over_and_limit(source, source, &game, limit)?;
        atlas.font.uniform = self.atlas.font.uniform;
        atlas.font.japanese = self.atlas.font.japanese;
        self.atlas = atlas;
        Ok(true)
    }
    /// The title's splash pool from `stack` (`sunrise_client::splashes::pool`).
    pub fn with_splashes(
        mut self,
        stack: &sunrise_assets::resource_source::ResourceSource,
        vanilla: &std::path::Path,
        skip_if_modified: bool,
    ) -> Self {
        self.splashes = sunrise_client::splashes::pool(stack, vanilla, skip_if_modified).into();
        self
    }
}

impl UiResources {
    pub fn fonts_only(root: &sunrise_assets::resource_source::ResourceSource) -> Result<Self> {
        Ok(Self {
            atlas: HudAtlas::fonts(root)?,
            panorama: Arc::new(sunrise_client::gfx::panorama::Faces::colour([0; 4])),
            splashes: Arc::from(""),
            plain_texts: PlainTexts::default(),
        })
    }
}

pub(in crate::game) fn load_animate_tables(
    root: &Path,
    registry: &sunrise_world::registry::BlockRegistry,
    biomes: &Biomes,
) -> Result<(
    sunrise_client::animate_tick::AnimateTables,
    sunrise_client::fog::FogTables,
)> {
    let _step = startup::step("rest.animate_fog");
    let animate = sunrise_client::animate_tick::AnimateTables::load(root, registry)?;
    Ok((animate, sunrise_client::fog::FogTables::load(root, biomes)))
}

pub(in crate::game) struct ClientPackParts {
    pub(in crate::game) render: Arc<RenderAssets>,
    pub(in crate::game) tables: Arc<MeshTables>,
    pub(in crate::game) item_renderer: sunrise_client::item_render::ItemRenderer,
}

impl ClientPackResources {
    pub(in crate::game) fn assemble(
        root: &sunrise_assets::resource_source::ResourceRoot,
        pack: &PackResources,
        parts: ClientPackParts,
        draws: bool,
    ) -> Result<Self> {
        let ClientPackParts {
            render,
            tables,
            item_renderer,
        } = parts;
        let assets = &pack.assets;
        let water_overlay_state = assets
            .registry
            .default_state("minecraft:water")
            .ok_or("block registry is missing the default minecraft:water state")?
            .try_into_api()
            .map_err(|e| format!("cannot bind the default minecraft:water state: {e:?}"))?;
        let (clouds, forcefield, custom_sky, environments, polytone_particles) =
            std::thread::scope(|scope| -> Result<_> {
                let (clouds, forcefield, custom_sky, environments, particles) = if draws {
                    let clouds = named_scope(scope, "pack.clouds", || {
                        sunrise_client::clouds::load(root.source())
                    });
                    let forcefield =
                        named_scope(scope, "pack.forcefield", || load_forcefield(root.source()));
                    let custom_sky = named_scope(scope, "pack.custom_sky", || {
                        sunrise_client::custom_sky::load(root.source())
                    });
                    let environments =
                        named_scope(scope, "pack.environments", || pack_environments(root));
                    let particles = named_scope(scope, "pack.particles", || {
                        pack_particles(root, assets.registry)
                    });
                    (
                        clouds.join().unwrap().map_err(|e| e.to_string())?,
                        forcefield.join().unwrap(),
                        custom_sky.join().unwrap(),
                        environments.join().unwrap(),
                        particles.join().unwrap(),
                    )
                } else {
                    (
                        Default::default(),
                        Default::default(),
                        sunrise_client::custom_sky::empty(),
                        None,
                        None,
                    )
                };
                Ok((clouds, forcefield, custom_sky, environments, particles))
            })?;
        let polytone_post = if draws {
            sunrise_client::polytone_post::PolytonePost::read(
                root.source(),
                sunrise_client::packs::polytone_configs(&sunrise_client::packs::Source::Opened(
                    root.source().clone(),
                )),
            )
            .map(Arc::new)
        } else {
            None
        };
        Ok(Self {
            layer_names: Arc::new(sunrise_client::block_sprites::texture_names(
                &render.sprite_layers,
            )),
            context: Arc::new(WorkerContext {
                chunk: (*pack.chunk_context).clone(),
                mesh: MeshWorkerContext {
                    render: render.clone(),
                    tables,
                    hidden_states: Default::default(),
                    part_tables: Default::default(),
                },
            }),
            water_overlay_state,
            render,
            item_renderer: Arc::new(item_renderer),
            clouds: Arc::new(clouds),
            forcefield: Arc::new(forcefield),
            custom_sky,
            environments,
            polytone_particles,
            polytone_post,
            polytone_colors: if draws {
                Arc::new(read_colors(root))
            } else {
                Arc::default()
            },
        })
    }

    pub(in crate::game) fn context_with(&self, biomes: Arc<Biomes>) -> Arc<WorkerContext> {
        let old = &self.context;
        if Arc::ptr_eq(&old.chunk.biomes, &biomes) {
            return old.clone();
        }
        Arc::new(WorkerContext {
            chunk: ChunkWorkerContext {
                assets: old.chunk.assets.clone(),
                biomes,
                light: old.chunk.light.clone(),
            },
            mesh: old.mesh.clone(),
        })
    }
}

/// The overworld's, Nether's and End's attribute stacks with the Polytone biome and dimension
/// modifiers of the packs merged at `root` (#11486); `None` when no pack has any. The overlay
/// root links the vanilla data the stacks start from.
pub(in crate::game) fn pack_environments(
    root: &sunrise_assets::resource_source::ResourceRoot,
) -> Option<Arc<[Environment; 3]>> {
    let modifiers = sunrise_client::environment::PackModifiers::read(root.source());
    if modifiers.is_empty() {
        return None;
    }
    let modifiers = modifiers.with_configs(sunrise_client::packs::polytone_configs(
        &sunrise_client::packs::Source::Opened(root.source().clone()),
    ));
    let load = |d: &str| Environment::load_dimension_with(root.data(), d, &modifiers);
    match (load("overworld"), load("the_nether"), load("the_end")) {
        (Ok(o), Ok(n), Ok(e)) => Some(Arc::new([o, n, e])),
        (o, n, e) => {
            for error in [o.err(), n.err(), e.err()].into_iter().flatten() {
                log::warn!("pack environment: {error}");
            }
            None
        }
    }
}

/// The custom particles and block emitters of the packs merged at `root` (#11486); `None` when
/// no pack has any.
pub(in crate::game) fn pack_particles(
    root: &sunrise_assets::resource_source::ResourceRoot,
    blocks: &'static sunrise_world::registry::BlockRegistry,
) -> Option<Arc<sunrise_client::particles::polytone::Registry>> {
    let configs = sunrise_client::packs::polytone_configs(&sunrise_client::packs::Source::Opened(
        root.source().clone(),
    ));
    let registry =
        sunrise_client::particles::polytone::Registry::read(root.source(), configs, blocks);
    (!registry.is_empty()).then(|| Arc::new(registry))
}

/// The packs' custom colours; their dye and sheep colours go to the draw paths' table.
pub(in crate::game) fn read_colors(
    root: &sunrise_assets::resource_source::ResourceRoot,
) -> sunrise_client::pack_palette::PackColors {
    let colors = sunrise_client::pack_palette::read(root.source());
    sunrise_client::pack_palette::set(&colors);
    sunrise_level::brewing::set_pack_colors(colors.effects.clone());
    colors
}

impl crate::game::client_side::ClientState {
    #[inline]
    pub(in crate::game) fn expect_client_res(
        slot: &std::sync::OnceLock<ClientResources>,
    ) -> &ClientResources {
        slot.get().expect("world resources joined before entry")
    }

    #[inline]
    pub(in crate::game) fn client_res(&self) -> &ClientResources {
        Self::expect_client_res(&self.client_res)
    }

    #[inline]
    pub fn try_block_entity_render(&self) -> Option<&BlockEntityRenderer> {
        self.client_res
            .get()
            .map(ClientResources::block_entity_render)
    }
}

impl crate::game::ClientGame<'_> {
    #[inline]
    pub fn client_res(&self) -> &ClientResources {
        self.client.client_res()
    }

    #[inline]
    pub fn try_block_entity_render(&self) -> Option<&BlockEntityRenderer> {
        self.client.try_block_entity_render()
    }
}

#[cfg(feature = "client")]
pub(in crate::game) struct ClientMobTables {
    pub(in crate::game) models: sunrise_client::mobs_render::ClientMobModels,
    pub(in crate::game) data: sunrise_client::mobs_render::ClientMobData,
}

#[cfg(feature = "client")]
pub struct ClientResources {
    pub(in crate::game) block_entity_render: BlockEntityRenderer,
    pub(in crate::game) animate: sunrise_client::animate_tick::AnimateTables,
    pub(in crate::game) fog: sunrise_client::fog::FogTables,
    pub(in crate::game) music: sunrise_client::music::MusicTables,
    pub(in crate::game) ambient: sunrise_client::ambience::AmbientTables,
    /// The player model's right and left arms.
    pub(in crate::game) player_arm: [Option<sunrise_client::mobs::Part>; 2],
    /// The slim model's right and left arms (3 pixels wide).
    pub(in crate::game) player_arm_slim: [Option<sunrise_client::mobs::Part>; 2],
    pub(in crate::game) player_type: Option<Arc<MobType>>,
    /// `SpinAttackEffectModel` (Riptide's spinning boxes).
    pub(in crate::game) spin_attack: Option<sunrise_client::mobs::Part>,
    pub(in crate::game) client_mobs: ClientMobTables,
}

#[cfg(feature = "client")]
#[derive(Clone)]
pub struct ClientPackResources {
    /// `Blocks.WATER.defaultBlockState()` for screen-effect extraction.
    pub water_overlay_state: mod_api::BlockState,
    pub render: Arc<RenderAssets>,
    pub context: Arc<sunrise_client_render::streaming::WorkerContext>,
    pub layer_names: Arc<HashMap<u32, String>>,
    /// Item definitions, baked item models and special renderers.
    pub item_renderer: Arc<sunrise_client::item_render::ItemRenderer>,
    /// clouds.png: (width, height, filled cells).
    pub clouds: Arc<(u32, u32, Vec<bool>)>,
    /// misc/forcefield.png: (width, height, RGBA) for the world border wall.
    pub(in crate::game) forcefield: Arc<(u32, u32, Vec<u8>)>,
    /// Custom sky layers (OptiFine's `optifine/sky`) with their textures; none without them.
    pub(in crate::game) custom_sky: Arc<dyn sunrise_client::custom_sky::SkyLayers>,
    /// The overworld's, Nether's and End's attribute stacks with the packs' Polytone biome and
    /// dimension modifiers (#11486), when a pack has some: what the frame draws.
    pub environments: Option<Arc<[Environment; 3]>>,
    /// The packs' Polytone custom particles and block emitters (#11486), when a pack has some.
    pub(in crate::game) polytone_particles:
        Option<Arc<sunrise_client::particles::polytone::Registry>>,
    /// The packs' Polytone post chains (#11486), when a pack has some.
    pub(in crate::game) polytone_post: Option<Arc<sunrise_client::polytone_post::PolytonePost>>,
    /// The packs' Polytone custom colours (`polytone/colors.json`).
    pub polytone_colors: Arc<sunrise_client::pack_palette::PackColors>,
}

/// Everything the title and menus draw: the GUI atlas with its font, the
/// title panorama and the splash pool. Kept apart from the world bundle so the
/// title can appear before the block sprites, models and gameplay tables are ready.
#[cfg(feature = "client")]
pub struct UiResources {
    /// The GUI, font, particle, map and skin atlas.
    pub atlas: HudAtlas,
    /// The title screen's panorama faces.
    pub panorama: Arc<sunrise_client::gfx::panorama::Faces>,
    /// The title screen's splashes (`sunrise_client::splashes::pool`).
    pub splashes: Arc<str>,
    /// The `Text` of each plain string mods draw (`DrawContext::text`).
    pub plain_texts: PlainTexts,
}

/// The `Text` of each plain string a mod draws by `&str` (`DrawContext::text`), kept so drawing
/// the same string again builds no new `Text`. It holds at most [`PlainTexts::MAX`] strings and
/// starts over past that, so text that changes every frame can't grow it.
#[cfg(feature = "client")]
#[derive(Default)]
#[rustfmt::skip]
pub struct PlainTexts(
    pub(in crate::game)
        std::cell::RefCell<std::collections::HashMap<Box<str>, sunrise_game_data::text::Text>>,
);

#[cfg(feature = "client")]
impl PlainTexts {
    pub(in crate::game) const MAX: usize = 256;
}

/// misc/forcefield.png for the world border wall (square, power of two).
#[cfg(feature = "client")]
fn load_forcefield(root: &sunrise_assets::resource_source::ResourceSource) -> (u32, u32, Vec<u8>) {
    root.read("assets/minecraft/textures/misc/forcefield.png")
        .map_err(Into::into)
        .and_then(|bytes| sunrise_client::hud::decode_bytes(&bytes))
        .ok()
        .filter(|(w, h, _)| w == h && w.is_power_of_two())
        .unwrap_or((1, 1, vec![0; 4]))
}
