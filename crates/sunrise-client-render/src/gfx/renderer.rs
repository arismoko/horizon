use super::*;

pub struct Renderer {
    pub(crate) instance: wgpu::Instance,
    pub(crate) adapter: wgpu::Adapter,
    /// The window's swapchain; `None` when rendering offscreen (`--offscreen`). Shared
    /// with frame generation's present thread while it runs.
    pub(crate) surface: Option<Arc<wgpu::Surface<'static>>>,
    /// The surface `release_surface` set aside for the renderer that replaces
    /// this one on the same device: reused, not created again, as a new
    /// surface for the window can come back lost (Wayland on Mesa).
    pub(crate) spare_surface: std::sync::Mutex<Option<wgpu::Surface<'static>>>,
    /// The offscreen colour target, sized like `config`.
    pub(crate) offscreen: Option<wgpu::Texture>,
    /// Offscreen submissions still on the GPU: at most two frames run ahead.
    pub(crate) in_flight: std::collections::VecDeque<wgpu::SubmissionIndex>,
    /// Which steps of the first frame on this device still go to the log
    /// (see `startup_step`).
    pub(crate) first_frame: FirstFrame,
    pub(crate) gpu_timer: Option<timing::GpuTimer>,
    /// GPU frame times (ms) read back since the caller last drained them.
    pub gpu_times: Vec<f32>,
    /// The light texture's pixels F3+4 shows, with what drew them (`lightmap_sample`).
    pub(crate) lightmap_chart: core_lightmap::LightmapChart,
    /// Time the last `render` spent blocked on the GPU or the display
    /// (acquiring, presenting, offscreen pacing, the submit thread holding the queue),
    /// excluded from CPU time.
    pub last_wait: std::time::Duration,
    /// CPU time of the last `render`, waits excluded.
    pub last_encode: std::time::Duration,
    /// Watched only while HDR is On or Auto, including Auto in SDR;
    /// before `window`, so it's dropped first.
    pub(crate) display: Option<DisplayWatch>,
    /// What this renderer's display last reported (`DisplayWatch`), its own: another
    /// renderer's watch never changes it.
    pub(crate) reading: output::DisplayReading,
    /// The watch was just made: the settings' Auto values take what it reported.
    pub(crate) display_fresh: bool,
    pub(crate) window: Arc<Window>,
    pub device: wgpu::Device,
    /// The shader modules, layouts and pipelines kept for the next renderer on
    /// this device ([`reuse`]).
    pub(crate) reuse: Arc<reuse::Reuse>,
    /// Frames are finished and submitted on its submit thread ([`submit`]).
    pub queue: submit::Queue,
    /// The last frame went to the submit thread; whether it had no window surface.
    /// The next frame takes it in ([`Self::land_frame`]).
    pub(crate) deferred: Option<bool>,
    /// Set by the device's lost callback. wgpu reports nothing else when an
    /// operation fails on a lost device (no error scope sees it).
    pub(crate) device_lost: Arc<std::sync::atomic::AtomicBool>,
    pub config: wgpu::SurfaceConfiguration,
    pub(crate) solid: SolidPipeline,
    /// `solid` split in two passes for the world pass (terrain.wgsl's `opaque_quad`).
    pub(crate) solid_opaque: SolidPipeline,
    pub(crate) solid_cutout: SolidPipeline,
    pub(crate) translucent: wgpu::RenderPipeline,
    pub(crate) translucent_mark: wgpu::RenderPipeline,
    /// Flat section draws (`flat`), where multi-draw is emulated.
    pub(crate) flat: Option<flat::Flat>,
    /// Sections upload their solid quads in the compact layout (`mesher::packed`): the solid
    /// passes read the arena themselves (`flat`) and no shader pack reads it as vertices.
    pub(crate) packed_quads: bool,
    /// The terrain's acceleration structures, with ray tracing on (`ray_scene`).
    pub(crate) ray_scene: Option<surface_setup::ray_query::ray_scene::RayScene>,
    /// The compact layout is allowed (`SUNRISE_PACKED_QUADS`, `set_packed_layout`).
    pub(crate) packed_on: bool,
    /// The visibility buffer for flat frames' solid terrain (`visbuffer`,
    /// `SUNRISE_VISBUFFER=1`).
    pub(crate) visbuf: Option<visbuffer::Visbuffer>,
    /// `flat` draws this frame's sections (off: the per-section draws).
    pub(crate) flat_on: bool,
    /// Occlusion culling of solid section draws, where the GPU's instance
    /// counts reach a multi-draw's entries (Vulkan, Metal).
    pub(crate) cull: Option<cull::Cull>,
    pub(crate) cull_on: bool,
    /// Solid quads facing away from the eye are left out (`mesher::Sides`).
    pub(crate) sides_on: bool,
    /// The near solid sections culled and drawn on the GPU (`gpu_culling`): its pass,
    /// records and fades while it is on, where it is offered (`gpu_culling_offer`, mirrored
    /// for the settings in `graphics::gpu_culling_offer`), and whether the last frame drew
    /// them so (`gpu_frame`).
    pub(crate) gpu_culling: Option<gpu_culling::GpuCulling>,
    /// Sourced geometry's residency beside them (`sourced_geometry::gpu::Sourced`): its
    /// record table, made with the first recorded mesh and dropped with the last, each
    /// mesh's record in it, and the meshes a GPU frame still lists the CPU. Whether the
    /// last frame drew them so is `gpu_sourced_frame`.
    pub(crate) sourced: sourced_geometry::gpu::Sourced,
    pub(crate) gpu_sourced_frame: bool,
    pub(crate) gpu_culling_offer: crate::graphics::GpuCullingOffer,
    pub(crate) gpu_frame: bool,
    /// GPU frames since GPU culling was switched on: the first and every 64th after it
    /// check its records (`check_gpu_records`).
    #[cfg(debug_assertions)]
    pub(crate) gpu_checks: u32,
    pub(crate) sky: wgpu::RenderPipeline,
    /// The main sky without the sunrise fan, which `sunrise_pipeline` draws as
    /// geometry over it. The mirror keeps `sky`: its per-pixel fan is cheap at
    /// the reflection's size and needs no second projection.
    pub(crate) sky_base: wgpu::RenderPipeline,
    pub(crate) sunrise_pipeline: wgpu::RenderPipeline,
    pub(crate) sunrise_buffer: wgpu::Buffer,
    pub(crate) dark_disc: wgpu::RenderPipeline,
    /// Clears the world pass's depth to the far plane so the first-person hand draws in the
    /// same pass (`hand_in_world`), as the hand's own pass would clear it.
    pub(crate) depth_reset: wgpu::RenderPipeline,
    pub(crate) lines: wgpu::RenderPipeline,
    pub(crate) lines_secondary: wgpu::RenderPipeline,
    /// A resource pack's `core/rendertype_lines`, which draws the block outline in their place.
    pub(crate) core_lines: Option<core_lines::CoreLines>,
    pub(crate) core_text: Option<core_text::CoreText>,
    /// The packs' `core/sky` and `core/stars`, drawn in place of the native sky and stars.
    pub(crate) core_sky: Option<core_sky::CoreSky>,
    /// A resource pack's `core/lightmap`, which draws the light texture in place of the native
    /// one.
    pub(crate) core_lightmap: Option<core_lightmap::CoreLightmap>,
    /// A resource pack's `core/clouds`, which draws the clouds in place of the native ones.
    pub(crate) core_clouds: Option<core_clouds::CoreClouds>,
    /// Polytone GPU particle types with their own shader, and this frame's batches of them.
    pub(crate) core_particles: core_particles::CoreParticles,
    /// Gizmo lines: opaque ones (`RenderTypes.lines`, which writes depth),
    /// translucent ones (`RenderTypes.linesTranslucentNoDepthWrite`) and the
    /// always-on-top ones.
    pub(crate) debug_lines_opaque: wgpu::RenderPipeline,
    pub(crate) debug_lines: wgpu::RenderPipeline,
    pub(crate) debug_lines_on_top: wgpu::RenderPipeline,
    pub(crate) debug_crosshair: Option<wgpu::RenderPipeline>,
    /// The debug crosshair and the first-person hand in `overlay_format`, where it isn't the
    /// frame's.
    pub(crate) overlay_crosshair: Option<wgpu::RenderPipeline>,
    pub(crate) overlay_hand: Option<wgpu::RenderPipeline>,
    pub(crate) overlay_hand_banner_pattern: Option<wgpu::RenderPipeline>,
    pub(crate) overlay_hand_glint: Option<wgpu::RenderPipeline>,
    pub(crate) debug_line_buffer: wgpu::Buffer,
    pub(crate) debug_line_capacity: u64,
    /// The terrain resources with the debug crosshair's projection
    /// (`crosshair_globals`).
    pub(crate) crosshair_group: wgpu::BindGroup,
    pub(crate) crosshair_globals: wgpu::Buffer,
    /// The water mirror: its solid terrain pipeline, its camera's globals and their group,
    /// and its targets once drawn.
    pub(crate) solid_mirror: wgpu::RenderPipeline,
    pub(crate) mirror_globals: wgpu::Buffer,
    pub(crate) mirror_group: wgpu::BindGroup,
    pub(crate) mirror: Option<MirrorTarget>,
    /// Native mods' world views' targets and pipelines, and the largest side the device gives one.
    pub(crate) world_views: world_views::Kept,
    pub(crate) world_view_side: u32,
    pub(crate) entity_pipeline: wgpu::RenderPipeline,
    /// `RenderPipelines.BANNER_PATTERN` (`RenderTypes.bannerPattern`): blended no-depth-write patterns.
    pub(crate) banner_pattern_pipeline: wgpu::RenderPipeline,
    /// `RenderPipelines.GLINT` (`RenderTypes.patternedShieldGlint`): equal-depth glint-only pass.
    pub(crate) glint_only_pipeline: wgpu::RenderPipeline,
    pub(super) particle_pipelines: particle_draws::ParticlePipelines,
    pub(super) explicit_opaque_particles: particle_draws::ParticleDraws,
    pub(super) explicit_translucent_particles: particle_draws::ParticleDraws,
    pub(super) explicit_additive_particles: particle_draws::ParticleDraws,
    pub(crate) static_entity_pipeline: wgpu::RenderPipeline,
    pub(crate) static_entity_group: wgpu::BindGroup,
    pub(crate) static_entity_transform: wgpu::Buffer,
    pub(crate) static_entity_camera: Option<DVec3>,
    pub(crate) static_entity_catalog_revision: u64,
    pub(crate) static_entity_frame_stats: StaticCacheFrameStats,
    pub(crate) hdr_mark_pipeline: wgpu::RenderPipeline,
    /// `hdr_mark_pipeline` at `entity_pipeline`'s depth bias, for what that drew.
    pub(crate) hdr_entity_mark_pipeline: wgpu::RenderPipeline,
    pub(crate) hdr_hand_unmark_pipeline: wgpu::RenderPipeline,
    pub(crate) hdr_mark_lightning_pipeline: wgpu::RenderPipeline,
    pub(crate) weather: weather::Weather,
    pub(crate) crumbling_pipeline: wgpu::RenderPipeline,
    pub(crate) shadow_blob_pipeline: wgpu::RenderPipeline,
    pub(crate) translucent_item_pipeline: wgpu::RenderPipeline,
    /// `RenderPipelines.WATER_MASK`: depth writes only, no colour.
    pub(crate) water_mask_pipeline: wgpu::RenderPipeline,
    pub(crate) cloud_pipeline: wgpu::RenderPipeline,
    pub(crate) hdr_cloud_mark_pipeline: wgpu::RenderPipeline,
    pub(crate) cloud_mesh: crate::clouds::Mesh,
    pub(crate) cloud_buffer: wgpu::Buffer,
    pub(crate) cloud_capacity: u64,
    pub(crate) cloud_uniform: wgpu::Buffer,
    pub(crate) cloud_group: wgpu::BindGroup,
    pub(crate) additive_pipeline: wgpu::RenderPipeline,
    pub(crate) dragon_rays_pipeline: wgpu::RenderPipeline,
    /// `RenderPipelines.ENERGY_SWIRL`: additive, unlit, the sprite repeated within itself.
    pub(crate) energy_swirl_pipeline: wgpu::RenderPipeline,
    pub(crate) lightning_pipeline: wgpu::RenderPipeline,
    pub(crate) lightning: lightning::Renderer,
    pub(crate) see_through_pipeline: wgpu::RenderPipeline,
    /// Native mods' world primitives (`Frame::world_primitives`), taken briefly during OIT pass.
    pub(crate) world_primitives: Option<world_primitives::WorldPrimitives>,
    /// Device handles and the mod mesh, buffer and render target tables, shared with `Game`.
    pub(crate) mod_gpu: mod_gpu::ModGpuResources,
    /// Cross-frame pool of transient GPU render targets for `Hooks::frame_graph_setup`.
    pub(crate) transient_targets: std::sync::Mutex<mod_frame_graph::TransientTargetPool>,
    pub(crate) text_polygon_offset_pipeline: wgpu::RenderPipeline,
    pub(crate) world_text_pipeline: wgpu::RenderPipeline,
    pub(crate) text_mark_pipelines: [wgpu::RenderPipeline; 2],
    pub(crate) fluid_pipeline: wgpu::RenderPipeline,
    pub(crate) terrain_model_pipelines: [wgpu::RenderPipeline; 4],
    pub(crate) celestial: celestial::Celestial,
    pub(crate) celestial_buffer: wgpu::Buffer,
    pub(crate) end_sky: end_sky::EndSky,
    pub(crate) stars: stars::Stars,
    /// Custom sky layers (`crate::custom_sky`), and the frame's from the game.
    pub(crate) custom_sky: custom_sky::CustomSky,
    pub(crate) custom_sky_draws: Vec<crate::custom_sky::Draw>,
    /// The glowing entities' outline (`LevelRenderer.entityOutlineTarget`).
    pub(crate) entity_outline: entity_outline::EntityOutline,
    pub(crate) panorama: panorama::Panorama,
    /// `RenderPipelines.WORLD_BORDER` and its buffers.
    pub(crate) border_pipeline: wgpu::RenderPipeline,
    pub(crate) border_group: wgpu::BindGroup,
    pub(crate) border_uniform: wgpu::Buffer,
    pub(crate) border_vertices: wgpu::Buffer,
    /// Screen effects: alpha-blended, then vignette (`dst * (1 - src)`)
    /// pipelines, each for every sampling (index `blend * 4 + sampling`).
    pub(crate) screen_pipelines: [wgpu::RenderPipeline; 16],
    pub(crate) screen_page_groups: Vec<Option<wgpu::BindGroup>>,
    pub(crate) screen_buffer: wgpu::Buffer,
    pub(crate) screen_capacity: u64,
    pub(crate) entity_group: wgpu::BindGroup,
    /// The layout of `entity_group`, for the native shadow pass.
    pub(crate) entity_bindings: wgpu::BindGroupLayout,
    /// Generation-owned prepared model geometry. World block entities instance it
    /// when the draw is not kept, spinning, or claimed by a shader pack.
    pub(crate) prepared_models: Option<PreparedModels>,
    pub(crate) prepared_draws: Vec<PreparedModelDraw>,
    /// Ordinary-stream cuts for `prepared_draws`, filled once per frame.
    pub(crate) prepared_cuts: Vec<u32>,
    pub(crate) prepared_ranges: Vec<prepared_model::OrderedSpan>,
    pub(crate) prepared_uploaded: Vec<PreparedModelInstance>,
    pub(crate) prepared_instance_buffer: wgpu::Buffer,
    pub(crate) prepared_instance_capacity: u64,
    /// The entity resources with the hand's projection (`hand_globals`).
    pub(crate) hand_group: wgpu::BindGroup,
    pub(crate) hand_globals: wgpu::Buffer,
    pub(crate) entity_buffer: wgpu::Buffer,
    pub(crate) entity_capacity: u64,
    pub(crate) static_entity_sections: HashMap<[i32; 3], GpuStaticEntitySection>,
    pub(crate) static_entity_indices: wgpu::Buffer,
    pub(crate) static_entity_index_quads: u32,
    pub(crate) entity_batch: EntityBatch,
    /// Quads of client mods' dynamic textures (`BlockEntityDrawContext::textured_quad`).
    pub(super) dynamic_entity: dynamic_entity::DynamicEntity,
    pub(crate) entity_order: entity_skin::OrdinaryOrder,
    /// Ordinary entity models posed on the GPU; `None` without compute shaders.
    pub(crate) entity_skin: Option<entity_skin::EntitySkin>,
    pub(crate) group: wgpu::BindGroup,
    pub(crate) fluid_group: wgpu::BindGroup,
    pub(crate) fluid_sampler: wgpu::Sampler,
    pub(crate) terrain_model_group: wgpu::BindGroup,
    pub(crate) terrain_model_sampler: wgpu::Sampler,
    pub(crate) globals: wgpu::Buffer,
    /// Resource-pack normal/specular maps, split like the colour sprite arrays.
    pub(crate) pbr_maps: Option<Vec<[wgpu::Texture; 2]>>,
    pub(crate) lightmap: wgpu::Texture,
    /// The block sprites, `sprite_split` layers per array (the first is
    /// `sprites` in terrain.wgsl, the others `sprites_1`...).
    pub(crate) sprite_textures: Vec<wgpu::Texture>,
    /// Block sprite layers per array on this device ([`layer_split`]).
    pub(crate) sprite_split: u32,
    /// A 1-layer array bound where a pack needs fewer arrays.
    pub(crate) sprite_unused: wgpu::TextureView,
    /// The sprite arrays as the terrain samples them (`SPRITE_ARRAYS` views, `sprite_unused`
    /// past the ones in use, the Mipmap Levels option's levels), for native mods
    /// (`WorldDrawn::block_sprites`).
    pub(crate) sprite_views: Vec<wgpu::TextureView>,
    /// Nearest, repeating, nearest mip level: for drawing a sprite's texels sharp elsewhere.
    pub(crate) sprite_nearest: wgpu::Sampler,
    pub(crate) mip_levels: u32,
    /// The mipmap levels sampled (the Mipmap Levels option), at most `mip_levels - 1`.
    pub(crate) mipmaps: u32,
    pub(crate) terrain_layout: wgpu::BindGroupLayout,
    /// terrain.wgsl's `SPRITE_ARRAYS` and `SPRITE_SPLIT`, for a shader pack's terrain.
    pub(crate) terrain_sprites: [f64; 2],
    pub(crate) sprite_sampler: wgpu::Sampler,
    /// `Options.textureFiltering`: 0 none, 1 RGSS, 2 anisotropic.
    pub(crate) texture_filtering: u32,
    /// The anisotropic sampler's clamp (`Options.maxAnisotropyValue`); 1 unless anisotropic.
    pub(crate) max_anisotropy: u16,
    pub(crate) lightmap_view: wgpu::TextureView,
    pub(crate) lightmap_sampler: wgpu::Sampler,
    pub(crate) animations: animate::SpriteAnimator,
    pub(crate) material_animations: Vec<SpriteAnimation>,
    pub(crate) active_animated_layers: Vec<u64>,
    pub(crate) uploads: upload::Uploads,
    /// Section meshes are staged through their own belt into
    /// `section_copies`, which the next frame records its commands after:
    /// `Queue::write_buffer` would make a staging buffer per write.
    pub(crate) section_uploads: upload::Uploads,
    pub(crate) section_copies: Option<wgpu::CommandEncoder>,
    /// Bytes staged in `section_copies`.
    pub(crate) section_copy_bytes: u64,
    /// When a large translucent page may start to shrink (`shrink_translucent`).
    pub(crate) shrink_gate: ShrinkGate,
    /// Whether `lightmap` holds `lightmap_chart`'s native pixels yet.
    pub(crate) lightmap_uploaded: bool,
    pub(crate) sprite_minimum_size: u32,
    /// Each sprite layer's own side, before the arrays scaled it up (`mipmap::Settings::native_size`).
    pub(crate) sprite_native: Vec<u32>,
    /// The Mipmap Levels option (`set_mipmap_levels`), which sets a shader pack atlas's padding.
    pub(crate) mipmap_option: u32,
    pub(crate) indices: wgpu::Buffer,
    pub(crate) index_quads: u32,
    pub(crate) instances: wgpu::Buffer,
    pub(crate) instance_capacity: u64,
    pub(crate) outline: wgpu::Buffer,
    pub(crate) depth: wgpu::TextureView,
    pub(crate) sections: resident::Resident<SectionGpu>,
    pub(crate) lods: FxHashMap<crate::lod::TileKey, lod::Tile>,
    /// Vertex bytes of the current tile meshes (not their fading stand-ins).
    pub(crate) lod_vertex_bytes: u64,
    /// The same tiles by cell (`lod::TileGrid`), to test them a handful of
    /// boxes at a time (`draw_frame`).
    pub(crate) lod_cells: lod::TileGrid,
    /// Far tiles' occlusion culling slots (`lod::Tile::cull_slot`): those
    /// free, and the next never used.
    pub(crate) lod_cull_free: Vec<u32>,
    pub(crate) lod_cull_next: u32,
    /// The chunk columns the resident tiles leave out (`Lod::take_cutout`):
    /// their sections are drawn even past the render distance, so terrain
    /// leaving it never opens a hole before the tiles fill it in.
    pub(crate) lod_cutout: crate::fast_hash::FxHashSet<(i32, i32)>,
    /// Removed sections of `lod_cutout` columns, released once tiles cover them.
    pub(crate) lod_retiring: crate::fast_hash::FxHashSet<SectionKey>,
    /// Removed sections fading out over far terrain (`leave_section`).
    pub(crate) lod_leaving: Vec<(SectionKey, std::time::Instant)>,
    /// Finished tile meshes `upload_lods` has yet to upload.
    pub(crate) lod_pending: Option<lod::Pending>,
    /// When the fades the last tile job began are over (`upload_lods`): until
    /// then tiles may be dying or standing in for their replacements.
    pub(crate) lod_settled: Option<std::time::Instant>,
    pub(crate) detail_columns: FxHashMap<(i32, i32), SectionMask>,
    /// Bumped whenever a column becomes or stops being complete.
    pub(crate) detail_revision: u64,
    pub(crate) arena: arena::Arena,
    /// Translucent quads' own arena, with large pages, while the layout is split
    /// (`translucent_split`): the back-to-front list then stays in one page (and
    /// one sorted-index page) and draws in a few multi-draws rather than one per
    /// few sections.
    pub(crate) translucent_arena: Option<arena::Arena>,
    /// Translucent Batching (`Graphics::translucent_batching`, read when a renderer
    /// is made), where it is offered (`graphics::offer_translucent_batching`).
    pub(crate) translucent_batching: bool,
    /// Every section and far tile was dropped to change the layout since the game
    /// last asked (`take_dropped_sections`).
    pub(crate) sections_dropped: bool,
    pub(crate) sorted_indices: arena::Arena,
    /// Buffers of released arena pages, freed a few frames later on a thread of their own.
    pub(crate) graveyard: arena::Graveyard<wgpu::Buffer>,
    pub(crate) translucent_sorter: translucency::Sorter,
    pub(crate) translucency_cursor: usize,
    /// `DrawIndexedIndirectArgs` for this frame's solid then translucent draws.
    pub(crate) indirect: wgpu::Buffer,
    pub(crate) indirect_capacity: u64,
    /// Indirect draws may set `first_instance` (the section origin).
    pub multi_draw: bool,
    /// Per-frame scratch, kept to avoid allocating every frame.
    pub(crate) visible: Vec<resident::Visible>,
    pub(crate) visible_lods: Vec<lod::Visible>,
    /// Canonical primary-camera visible sections exposed to `FrameHooks` (reused across frames).
    pub(crate) visible_renderable_sections: Vec<mod_api::RenderableSection>,
    pub(crate) visible_renderable_scratch: Vec<(u64, SectionKey, Option<u32>)>,
    /// Reusable submission and merge buffers for `FrameHooks`' CPU models and name tags.
    pub(crate) hook_geometry: world_primitives::PreparedHookGeometry,
    pub(crate) hook_entities_scratch: Vec<Vertex>,
    pub(crate) hook_see_through_scratch: Vec<Vertex>,
    /// Every far tile in view, whether the GPU's solid draws own it or not: the water's
    /// re-sort walks it (`draw_frame`), so its budget and rotation do not follow which
    /// solid draws the sourced-geometry pass took over.
    pub(crate) translucent_lods: Vec<lod::Visible>,
    /// The sections a shader pack's shadow camera sees (`external::ShadowView`).
    pub(crate) shadow_visible: Vec<resident::Visible>,
    /// The external shader pipeline world frames call (`external`); `None` draws vanilla's way.
    pub(crate) external: Option<Box<dyn external::ExternalPipeline>>,
    pub(crate) translucent_draws: Vec<(u64, usize, [u32; 5], (u32, Option<u32>))>,
    /// Per visible section: the occlusion test that also sees its translucent quads
    /// (`NO_TEST` without one): the section is left to phase 2.
    pub(crate) translucent_tests: Vec<u32>,
    /// This frame's translucent draws that a test sees: (index among them, test).
    pub(crate) translucent_marks: Vec<(u32, u32)>,
    /// Phase 2's draws and the translucent ones after them, with the instance counts the
    /// tests give (`Cull::upload`).
    pub(crate) phase2_upload: Vec<[u32; 5]>,
    pub(crate) origins: Vec<[f32; 4]>,
    /// `DrawIndexedIndirectArgs` words: index count, instance count, first
    /// index, base vertex, first instance.
    pub(crate) draws: Vec<[u32; 5]>,
    /// Vertex arena page and optional sorted index arena page of each draw:
    /// with sorted indices (a translucent draw), a page of `translucent_arena`
    /// when there is one.
    pub(crate) draw_pages: Vec<(u32, Option<u32>)>,
    /// Solid draws left to occlusion culling's phase 2, while the list is
    /// built, and their pages.
    pub(crate) phase2_draws: Vec<[u32; 5]>,
    pub(crate) phase2_pages: Vec<(u32, Option<u32>)>,
    /// Each section's solid draws, to group them by arena page.
    pub(crate) solid_runs: page_runs::PageRuns,
    pub(crate) phase2_runs: page_runs::PageRuns,
    /// The shadow camera's draws by page, in bands of distance from the light.
    pub(crate) shadow_runs: page_runs::PageRuns,
    /// Block-entity blocks per meshed section (world position, state id).
    pub(crate) block_entities: super::census::SectionLists,
    /// Changes on every mesh upload for that section; block-entity static
    /// geometry also depends on the section's packed light.
    pub(crate) block_entity_section_revisions: FxHashMap<SectionKey, u64>,
    /// Changes only when the block-entity positions/states in a section
    /// change. Empty-section tombstones remain until that section unloads so
    /// the static cache can remove deleted models without a global flush.
    pub(crate) block_entity_membership_revisions: FxHashMap<SectionKey, u64>,
    /// When each section was first uploaded (`RenderSection.uploadedTime`): kept across meshing
    /// it again, gone with its column. Sections that fade in over far terrain are not in it.
    pub(crate) section_uploaded: FxHashMap<SectionKey, std::time::Instant>,
    /// When `section_uploaded` last gained a section: once the fade time has passed since,
    /// every section is whole and frames skip the per-section lookups.
    pub(crate) newest_upload: Option<std::time::Instant>,
    /// When a section last began to fade in over far terrain (`SectionGpu::fading`), for the
    /// same shortcut.
    pub(crate) newest_lod_fade: Option<std::time::Instant>,
    /// Changes whenever `block_entities` does (unique across renderers).
    pub(crate) block_entity_revision: u64,
    /// The light sources of each section and their revision, for `block_light`.
    pub(crate) light_blocks: super::census::SectionLists,
    pub(crate) light_revision: u64,
    /// VisGraph face connectivity per meshed section (cave culling).
    pub(crate) occlusion: occlusion::Occlusion,
    /// The column `occlusion`, `sections` and `detail_columns` count in: the
    /// level's (`clear_sections`).
    pub(crate) layout: crate::chunk::ColumnLayout,
    /// Resolution scale, FXAA and TAA (Sunrise settings).
    pub(crate) world_post: world_post::Post,
    /// Frame generation (Sunrise setting).
    pub(crate) frame_gen: frame_gen::FrameGen,
    /// The entities' texture atlas (`entity_group`'s), for frame generation's motion.
    pub(crate) entity_atlas: wgpu::TextureView,
    /// HDR output (Sunrise setting): `Some` while the swapchain is HDR.
    pub(crate) hdr: Option<hdr::Hdr>,
    /// Which pixels of each block sprite may shine in HDR output (terrain.wgsl's
    /// `emission_thresholds`), measured when HDR output turns on.
    pub(crate) emission_stats: hdr_stats::EmissionStats,
    /// What this renderer's destination offers and its device draws.
    pub(crate) output_caps: output::OutputCapabilities,
    /// An output that failed to apply, not tried again until something it needs changes.
    pub(crate) output_rejected: Option<output_apply::Rejected>,
    /// Since when Auto keeps HDR on through the display's loss of its report.
    pub(crate) auto_off_since: Option<std::time::Instant>,
    /// The output applied (`update_output`), with the frame every pipeline that draws it
    /// targets: the swapchain's 8-bit format, or `Rgba16Unorm` when the renderer was made with
    /// HDR output on and Smooth Gradients and the device draws and blends into it, or
    /// `LINEAR_FRAME` (`frame_format`). A 16-bit frame with HDR off since reaches the
    /// swapchain through `readback`.
    pub(crate) output: output::EffectiveOutput,
    /// Whether a pack picked or dropped since the renderer was made has been told it runs on
    /// the frame that exists (`set_native_shader_pack`).
    pub(crate) frame_kept_logged: bool,
    /// The marker file of an HDR output that has not presented a frame yet
    /// (`graphics::HDR_MARKER`).
    pub(crate) hdr_marker: Option<std::path::PathBuf>,
    /// The window's graphics driver (`hdr_start`); `None` offscreen.
    pub(crate) hdr_driver: Option<String>,
    /// The HDR Lighting setting, kept for the HDR output made when HDR turns on.
    pub(crate) hdr_lighting_setting: bool,
    /// The shader pack's post-processing, and the dimension (legacy id) it was loaded for.
    pub(crate) pack_post: Option<(i32, Option<pack_post::PackPost>)>,
    /// A `shaders/` pack whose pipelines build on a thread of their own (`load_shader_pack`).
    pub(crate) pack_loading: Option<PackLoading>,
    /// Counts shader packs loaded, switched or turned off (`ShaderPackState::generation`).
    pub(crate) pack_generation: u32,
    /// The post passes of one of Sunrise's own shader packs; never beside `pack_post`'s.
    pub(crate) native: Option<native_pack::NativePack>,
    /// Time the shader pack's passes on the GPU (F3, or a capture made with `--gpu`).
    pub(crate) pass_timing: bool,
    /// Times the parts of passes too (`set_pass_detail`).
    pub(crate) pass_detail: bool,
    /// Least time between timed frames (`set_pass_timing_gap`).
    pub(crate) pass_timing_gap: std::time::Duration,
    /// Whether the timers keep every timed frame for `take_timed_frames`.
    pub(crate) keep_timed_frames: bool,
    /// Times Sunrise's own passes on the GPU, as `pass_timing` does (`own_pass_times`).
    pub(crate) own_timing: Option<pack_timing::PassTiming>,
    /// The entity vertices Sunrise draws itself while a shader pack draws the others.
    pub(crate) pack_entity_rest: Vec<Vertex>,
    /// Set 1 of the cloud pipeline, for a shader pack's `gbuffers_clouds`.
    pub(crate) cloud_bindings: wgpu::BindGroupLayout,
    /// Rain and snow handed to a shader pack's `gbuffers_weather`, once a pack draws them.
    pub(crate) pack_weather: Option<pack_weather::PackWeather>,
    /// The block sprites as one atlas, while a shader pack is loaded.
    pub(crate) pack_atlas: Option<pack_atlas::PackAtlas>,
    pub(crate) pack_material_atlases: Option<[pack_atlas::PackAtlas; 2]>,
    /// The shared atlas's LabPBR normal and specular paged maps (`set_shared_material_maps`),
    /// which shader packs read for entities, items and the hand.
    pub(crate) shared_material_maps: [Option<wgpu::Texture>; 2],
    /// The pack's terrain program reads flat draws' vertices (`pack_terrain_inputs`).
    pub(crate) pack_terrain_flat: bool,
    /// Vertex shaders read the section arena's pages (`flat::pages_readable`, and
    /// `pack_post::Gpu::vertex_reads`): a pack's terrain
    /// program can read its quad's corners there in per-section draws too (`pack_terrain_draws`).
    pub(crate) terrain_pages: bool,
    /// The pack's terrain programs read their pages in per-section draws: decided as
    /// `pack_terrain_inputs` last took them, from `pack_terrain_bindings`.
    pub(crate) pack_terrain_pages: bool,
    /// The pack's terrain programs read the normal stream in per-section draws that read no
    /// pages: decided as `pack_terrain_inputs` last took them, from `pack_terrain_bindings`.
    pub(crate) pack_terrain_normals: bool,
    /// What the pack being loaded binds in its terrain programs' vertex stage
    /// (`set_pack_terrain_bindings`).
    pub(crate) pack_terrain_bindings: pack_atlas::TerrainBindings,
    /// The section arena's pages are storage too (`arena_storage`, `follow_arena_layout`).
    pub(crate) arena_storage: bool,
    /// The running `shaders/` pack's block ids, two a state (`set_shader_pack`'s `block_ids`):
    /// the normal stream holds each quad's id under them (`terrain_normals`). None without any:
    /// no pack, or one that lists no block.
    /// Boxed: `Game` holds the `Renderer` inline, and its tests that build a `Game` on their
    /// thread's stack have little room left.
    pub(crate) pack_block_ids: Option<Box<PackBlockIds>>,
    /// The game's meshes carry a `shaders/` pack's quad sources (`keep_pack_sources`).
    pub(crate) pack_sources_on: bool,
    /// OptiFine/Iris separate ambient occlusion: pass vertex alpha to hz_Color.a.
    pub separate_ao: bool,
    /// Shader-pack terrain pages used by post processing, retained between frames.
    pub(crate) pack_pages: Vec<pack_post::TerrainPage>,
    /// Camera-relative selection outline vertices, retained between frames.
    pub(crate) outline_edges_scratch: Vec<[f32; 6]>,
    /// Selection outline line vertices, retained between frames.
    pub(crate) outline_lines_scratch: Vec<[f32; 8]>,
    /// Debug line vertices, retained between frames.
    pub(crate) debug_lines_scratch: Vec<[f32; 8]>,
    /// Screen effect vertices, retained between frames.
    pub(crate) screen_vertices_scratch: Vec<crate::screen_effects::ScreenVertex>,
    /// LOD leaving chunk columns, retained between frames.
    pub(crate) lod_leaving_scratch: crate::fast_hash::FxHashSet<(i32, i32)>,
    /// Shadow visible boxed sections, retained between frames.
    pub(crate) shadow_boxed_scratch: Vec<resident::Visible>,
    /// Shadow visible seen section keys, retained between frames.
    pub(crate) shadow_seen_scratch: crate::fast_hash::FxHashSet<SectionKey>,
    /// Shadow translucent section draws, retained between frames.
    pub(crate) shadow_translucent_scratch: Vec<(u64, [u32; 5], (u32, Option<u32>))>,
    /// Lava's sprite layers, which a pack's solid terrain draws (`pack_atlas::game_terrain`).
    pub(crate) pack_lava_layers: Vec<u16>,
    pub hud: HudRenderer,
    /// Vanilla's menu background blur, made on first use.
    pub(crate) menu_blur: Option<crate::menu_blur::MenuBlur>,
    /// `post_effect` chains, made on first use.
    pub(crate) post_effects: Option<crate::post_chain::PostEffects>,
    /// Where a screenshot's frame is drawn when the swapchain cannot be read back.
    pub(crate) readback: readback::Readback,
    pub info: wgpu::AdapterInfo,
    pub suspended: bool,
    /// Consecutive frames whose surface acquisition failed validation. The
    /// first gets the swapchain a fresh configuration; later ones reattach
    /// the window, as a lost surface does.
    pub(crate) acquire_validation_failures: u32,
    /// A surface from `recreate_surface` whose configuration failed. wgpu
    /// panics when such a surface is acquired from (it never had one that
    /// worked), so frames configure it again first and skip until it takes.
    pub(crate) surface_unconfigured: bool,
    /// The last frame's statistics, all of one frame (`show_stats`): its sections drawn
    /// (F3's `C:`, the world icon's check), section draws (face ranges and translucent
    /// sections) and the draw calls the backend issued for them. A CPU frame's are its own;
    /// in GPU frames (`gpu_culling`) they are the newest GPU frame's whose counts came back,
    /// a frame or a few earlier (`stats_lag`).
    pub drawn_sections: usize,
    pub section_draws: usize,
    pub section_draw_calls: usize,
    /// `draw_frame`'s frames past its early returns, and the one whose statistics are shown.
    pub(crate) frame_serial: u64,
    pub(crate) stats_frame: u64,
    /// The readback halves a GPU frame's statistics wait for (`in_flight::collect_stats`):
    /// a half alone undercounts the frame.
    pub(crate) stats_halves: in_flight::StatsHalves,
    pub uploaded_bytes: u64,
    pub(crate) started: Instant,
    /// The next frame to write to a PNG path (control socket, F2, world icon).
    pub screenshot: Option<ScreenshotRequest>,
    /// Readback outcome: CPU pixels for the game to approve before writing, or an error.
    pub screenshot_result: Option<std::result::Result<PendingScreenshot, String>>,
    pub(crate) screenshot_readbacks: screenshot_readback::Readbacks,
    /// Screenshot Quality (Sunrise setting): a `scaled` screenshot's frame is this many
    /// times the game's resolution (1, 2, 4 or 8).
    pub screenshot_quality: u32,
    /// How many times the pending scaled screenshot ran out of video memory: each
    /// halves its size, down to the window's.
    pub(crate) screenshot_halvings: u32,
    /// Brightness option (lightmap gamma), 0..1.
    pub gamma: f32,
    /// Optional full-light lightmap (Sunrise Extras).
    pub fullbright: bool,
    /// The Improved transparency option (`GameRenderer.useImprovedTransparency`).
    pub improved_transparency: bool,
    /// Its passes and targets, made when first drawn with.
    pub(crate) oit: Option<oit::Oit>,
    /// Advance animated block textures (Sunrise setting; vanilla always does).
    pub animated_textures: bool,
    pub(crate) present_modes: Vec<wgpu::PresentMode>,
    /// CPU copies of the section and far tile meshes uploaded while `render-capture` is on
    /// (`keep_capture_copies`, or `SUNRISE_RENDER_CAPTURE` at creation); `None` otherwise.
    pub(crate) capture_copies: Option<capture::Copies>,
    /// A scene to write from the next frame (`sunrise-ctl render-capture DIR`).
    pub capture: Option<capture::Request>,
    /// Outcome of the last capture: its summary, or the error.
    pub capture_result: Option<std::result::Result<serde_json::Value, String>>,
}

impl Drop for Renderer {
    /// Once the device is lost, the window's surfaces are left alive: wgpu refuses to present
    /// or discard a frame on a lost device and keeps that frame acquired, and destroying the
    /// swapchain under an acquired frame panics (while a panic unwinds, an abort).
    fn drop(&mut self) {
        if !self.device_lost() {
            return;
        }
        std::mem::forget(self.surface.take());
        std::mem::forget(
            self.spare_surface
                .get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take(),
        );
    }
}
