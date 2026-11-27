//! Entity types drawn by mods' renderers on the client: `native_register_entity_renderers`
//! (NeoForge's `EntityRenderersEvent.RegisterRenderers`) is asked once at start, before resources
//! load, and every resource load resolves the registrations into the slots the draw routes read, so
//! a frame hands an entity to its type's renderer without a lookup by name or a scan over the mods.
//! The renderer draws through the shared drawing boundary (`BlockEntityDrawContext`).
//!
//! The routes that read the table so far: the content mods' thrown types
//! (`ProjectileKind::Thrown`, by [`ThrownType::slot`]), the living types drawn as mobs (by
//! their shared `MobType`, which the game's types never replace while it runs), whose renderer
//! draws in place of the model and its vanilla layers, primed TNT (`minecraft:tnt`), experience
//! orbs (`minecraft:experience_orb`), paintings (`minecraft:painting`), and item frames
//! (`minecraft:item_frame`, `minecraft:glow_item_frame`), and a mod's general entity types (by
//! their slot in the frozen catalog, `general_type_catalog`). Each other entity family becomes a route
//! of its own; until then a registration for one of its types is kept and the type keeps its
//! drawing.
//! Each load also asks for the layers of the living renderers (`native_entity_layers`), which draw
//! through the same guard ([`Game::guarded_entity_draw`]).
use super::native_block_entity_draw::{BlockEntityDraw, LevelLight};
use crate::game::host::draw::GameBlockEntityDrawContext;
use glam::{DVec3, Mat4};
use mod_api::{
    __private::game::{CallbackOwner, contain_unwind},
    BlockEntityDrawContext, EntityRenderState, EntityRenderer,
};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc, OnceLock, PoisonError, RwLock,
        atomic::{AtomicBool, Ordering},
    },
};
use sunrise_client::{block_entity_render::diffuse_lights, mesher::Vertex};
use sunrise_gameplay_common::game::general::{GeneralTypeCatalog, general_type_catalog};
use sunrise_level::fast_hash::FxHashMap;
use sunrise_sim::{
    mobs::{Mob, MobType},
    projectiles::ThrownType,
};

/// One registered entity renderer and its session callback owner (if registered by a mod).
#[derive(Clone)]
pub struct RegisteredEntityRenderer {
    pub(in crate::game) renderer: Arc<dyn EntityRenderer>,
    pub(in crate::game) owner: Option<CallbackOwner>,
}

impl RegisteredEntityRenderer {
    pub fn new(renderer: Arc<dyn EntityRenderer>, owner: Option<CallbackOwner>) -> Self {
        Self { renderer, owner }
    }

    pub(in crate::game) fn is_enabled(&self) -> bool {
        self.owner.as_ref().is_none_or(CallbackOwner::is_enabled)
    }
}

/// The registrations, by entity type, the last one for a type winning: kept once at start.
static REGISTERED: OnceLock<BTreeMap<String, RegisteredEntityRenderer>> = OnceLock::new();

/// The registrations when no mod registered any (and before the start registered them).
static NO_RENDERERS: BTreeMap<String, RegisteredEntityRenderer> = BTreeMap::new();

/// The table the last resource load resolved; none before the first.
static TABLE: RwLock<Option<Arc<EntityRendererTable>>> = RwLock::new(None);

/// Whether the last load resolved a renderer for any living type: a frame checks only this for
/// each mob when none was registered.
static ANY_LIVING: AtomicBool = AtomicBool::new(false);

/// Whether the last load resolved a renderer for primed TNT: a frame checks only this when none
/// was registered, so the vanilla TNT route stays off the table lock.
static ANY_TNT: AtomicBool = AtomicBool::new(false);
/// Whether the last load resolved a renderer for an experience orb: a frame checks only this when
/// none was registered, so the vanilla orb route stays off the table lock.
static ANY_EXPERIENCE_ORB: AtomicBool = AtomicBool::new(false);
/// Whether the last load resolved a renderer for a painting: a frame checks only this when none
/// was registered, so the vanilla painting route stays off the table lock.
static ANY_PAINTING: AtomicBool = AtomicBool::new(false);
/// Whether the last load resolved a renderer for an item frame or a glow item frame: a frame
/// checks only this when none was registered, so the vanilla frame route stays off the table lock.
static ANY_ITEM_FRAME: AtomicBool = AtomicBool::new(false);

/// Whether the last load resolved a renderer for any general entity type: a frame checks only this
/// when none was registered, so the general route stays off the table lock.
static ANY_GENERAL: AtomicBool = AtomicBool::new(false);

/// The player's entity type: its model goes by its skin, not by entity type.
const PLAYER: &str = "minecraft:player";

/// Primed TNT's entity type.
const TNT: &str = "minecraft:tnt";
/// An experience orb's entity type.
const EXPERIENCE_ORB: &str = "minecraft:experience_orb";
/// A painting's entity type.
const PAINTING: &str = "minecraft:painting";
/// An item frame's entity type.
const ITEM_FRAME: &str = "minecraft:item_frame";

/// A glow item frame's entity type.
const GLOW_ITEM_FRAME: &str = "minecraft:glow_item_frame";

/// Keeps the mods' registrations, once per process (false when they already were).
pub(in crate::game) fn register(
    renderers: BTreeMap<String, (Option<&'static str>, Arc<dyn EntityRenderer>)>,
) -> bool {
    let registered = renderers
        .into_iter()
        .map(|(id, (owner, renderer))| {
            (
                id,
                RegisteredEntityRenderer::new(renderer, owner.map(CallbackOwner::for_mod)),
            )
        })
        .collect();
    REGISTERED.set(registered).is_ok()
}

/// The registered renderers, by the slot each draw route reads: rebuilt on every resource load.
pub struct EntityRendererTable {
    /// By [`ThrownType::slot`].
    thrown: Vec<Option<RegisteredEntityRenderer>>,
    /// By the address of the mob type the game shares with every mob of the type.
    living: FxHashMap<usize, RegisteredEntityRenderer>,
    /// Primed TNT (`minecraft:tnt`).
    tnt: Option<RegisteredEntityRenderer>,
    /// An experience orb (`minecraft:experience_orb`).
    experience_orb: Option<RegisteredEntityRenderer>,
    /// A painting (`minecraft:painting`).
    painting: Option<RegisteredEntityRenderer>,
    /// An item frame (`minecraft:item_frame`).
    item_frame: Option<RegisteredEntityRenderer>,
    /// A glow item frame (`minecraft:glow_item_frame`).
    glow_item_frame: Option<RegisteredEntityRenderer>,
    /// By the general type's slot in the frozen catalog (`GeneralTypeCatalog::slot`).
    general: Vec<Option<RegisteredEntityRenderer>>,
}

impl EntityRendererTable {
    /// The table for `registered`, with `thrown` the content mods' thrown types, `mobs` the
    /// living types the game draws as mobs, `general` the mods' general entity types and `known`
    /// whether an id is any other entity type; and the ids of the registrations for no entity
    /// type, which it leaves out. A registration for a known type no route reads yet is kept
    /// silently: the type keeps its own drawing.
    pub fn resolve(
        registered: &BTreeMap<String, RegisteredEntityRenderer>,
        thrown: &[ThrownType],
        mobs: &HashMap<String, Arc<MobType>>,
        general: &GeneralTypeCatalog,
        known: impl Fn(&str) -> bool,
    ) -> (Self, Vec<String>) {
        let mut table = Self {
            thrown: thrown.iter().map(|_| None).collect(),
            living: FxHashMap::default(),
            tnt: None,
            experience_orb: None,
            painting: None,
            item_frame: None,
            glow_item_frame: None,
            general: vec![None; general.len()],
        };
        let mut unknown = Vec::new();
        for (id, renderer) in registered {
            if let Some(t) = thrown.iter().find(|t| t.id == id) {
                table.thrown[t.slot] = Some(renderer.clone());
            } else if let Some(kind) = mobs.get(id).filter(|_| id != PLAYER) {
                table
                    .living
                    .insert(Arc::as_ptr(kind) as usize, renderer.clone());
            } else if id == TNT {
                table.tnt = Some(renderer.clone());
            } else if id == EXPERIENCE_ORB {
                table.experience_orb = Some(renderer.clone());
            } else if id == PAINTING {
                table.painting = Some(renderer.clone());
            } else if id == ITEM_FRAME {
                table.item_frame = Some(renderer.clone());
            } else if id == GLOW_ITEM_FRAME {
                table.glow_item_frame = Some(renderer.clone());
            } else if let Some(slot) = general.slot(id) {
                table.general[slot] = Some(renderer.clone());
            } else if !known(id) {
                unknown.push(id.clone());
            }
        }
        (table, unknown)
    }

    /// The renderer of the thrown type `t`, when a mod registered one.
    pub fn thrown(&self, t: &ThrownType) -> Option<&RegisteredEntityRenderer> {
        self.thrown.get(t.slot)?.as_ref()
    }

    /// The renderer of `mob`'s type, when a mod registered one for it.
    pub(in crate::game) fn living(&self, mob: &Mob) -> Option<&RegisteredEntityRenderer> {
        self.living.get(&(Arc::as_ptr(&mob.kind) as usize))
    }

    /// The renderer of primed TNT, when a mod registered one for `minecraft:tnt`.
    pub fn tnt(&self) -> Option<&RegisteredEntityRenderer> {
        self.tnt.as_ref()
    }

    /// The renderer of an experience orb, when a mod registered one for `minecraft:experience_orb`.
    pub(in crate::game) fn experience_orb(&self) -> Option<&RegisteredEntityRenderer> {
        self.experience_orb.as_ref()
    }

    /// The renderer of a painting, when a mod registered one for `minecraft:painting`.
    pub(in crate::game) fn painting(&self) -> Option<&RegisteredEntityRenderer> {
        self.painting.as_ref()
    }

    /// The renderer of the general type in catalog slot `slot`, when a mod registered one.
    pub fn general(&self, slot: usize) -> Option<&RegisteredEntityRenderer> {
        self.general.get(slot)?.as_ref()
    }

    /// The general types of `catalog` no mod registered a renderer for: their rows draw nothing.
    pub fn unrendered_general<'c>(&self, catalog: &'c GeneralTypeCatalog) -> Vec<&'c str> {
        (0..catalog.len())
            .filter(|&slot| self.general(slot).is_none())
            .map(|slot| catalog.key(slot))
            .collect()
    }

    /// The renderer of an item frame or glow item frame, when a mod registered one for that type.
    pub(in crate::game) fn item_frame(&self, glow: bool) -> Option<&RegisteredEntityRenderer> {
        if glow {
            self.glow_item_frame.as_ref()
        } else {
            self.item_frame.as_ref()
        }
    }
}

/// The renderer the last resource load resolved for the thrown type `t`, when a mod registered one.
pub(in crate::game) fn thrown_renderer(t: &ThrownType) -> Option<RegisteredEntityRenderer> {
    let table = TABLE.read().unwrap_or_else(PoisonError::into_inner);
    table
        .as_ref()?
        .thrown(t)
        .filter(|r| r.is_enabled())
        .cloned()
}

/// The renderer the last resource load resolved for `mob`'s type, when a mod registered one: with
/// none registered for any living type, one flag check, with no lock and no lookup.
pub(in crate::game) fn living_renderer(mob: &Mob) -> Option<RegisteredEntityRenderer> {
    if !ANY_LIVING.load(Ordering::Relaxed) {
        return None;
    }
    let table = TABLE.read().unwrap_or_else(PoisonError::into_inner);
    table
        .as_ref()?
        .living(mob)
        .filter(|r| r.is_enabled())
        .cloned()
}

/// The renderer the last resource load resolved for primed TNT, when a mod registered one: with
/// none registered, one flag check, with no lock.
pub(in crate::game) fn tnt_renderer() -> Option<RegisteredEntityRenderer> {
    if !ANY_TNT.load(Ordering::Relaxed) {
        return None;
    }
    let table = TABLE.read().unwrap_or_else(PoisonError::into_inner);
    table.as_ref()?.tnt().filter(|r| r.is_enabled()).cloned()
}

/// The renderer the last resource load resolved for an experience orb, when a mod registered one:
/// with none registered, one flag check, with no lock.
pub(in crate::game) fn experience_orb_renderer() -> Option<RegisteredEntityRenderer> {
    if !ANY_EXPERIENCE_ORB.load(Ordering::Relaxed) {
        return None;
    }
    let table = TABLE.read().unwrap_or_else(PoisonError::into_inner);
    table
        .as_ref()?
        .experience_orb()
        .filter(|r| r.is_enabled())
        .cloned()
}

/// The renderer the last resource load resolved for a painting, when a mod registered one: with
/// none registered, one flag check, with no lock.
pub(in crate::game) fn painting_renderer() -> Option<RegisteredEntityRenderer> {
    if !ANY_PAINTING.load(Ordering::Relaxed) {
        return None;
    }
    let table = TABLE.read().unwrap_or_else(PoisonError::into_inner);
    table
        .as_ref()?
        .painting()
        .filter(|r| r.is_enabled())
        .cloned()
}

/// The renderer the last resource load resolved for an item frame or glow item frame, when a mod
/// registered one for that type: with none registered for either, one flag check, with no lock.
pub(in crate::game) fn item_frame_renderer(glow: bool) -> Option<RegisteredEntityRenderer> {
    if !ANY_ITEM_FRAME.load(Ordering::Relaxed) {
        return None;
    }
    let table = TABLE.read().unwrap_or_else(PoisonError::into_inner);
    table
        .as_ref()?
        .item_frame(glow)
        .filter(|r| r.is_enabled())
        .cloned()
}

/// A frame's immutable renderer table. Capture it once, releasing the lock before calling mods.
pub(in crate::game) fn general_renderers() -> Option<Arc<EntityRendererTable>> {
    if !ANY_GENERAL.load(Ordering::Relaxed) {
        return None;
    }
    TABLE.read().unwrap_or_else(PoisonError::into_inner).clone()
}

impl crate::game::ClientGame<'_> {
    /// Resolves the registered renderers against the entity types of the resources now loaded, then
    /// asks for the living renderers' layers: at the first load and on every reload after it. A
    /// registration for no entity type is logged with its id.
    pub fn resolve_entity_renderers(&self) {
        let registered = REGISTERED.get().unwrap_or(&NO_RENDERERS);
        let rows = super::entity_type_rows::entity_type_rows(&self.common.options.resources);
        let catalog = general_type_catalog();
        let (table, unknown) = EntityRendererTable::resolve(
            registered,
            sunrise_sim::projectiles::thrown_types(),
            self.common.res.mob_types(),
            catalog,
            |id| rows.contains(id),
        );
        for id in unknown {
            log::warn!(
                "Sunrise: an entity renderer is registered for {id}, which is no entity type; it is left out"
            );
        }
        // A general type has no vanilla look to fall back to: its rows draw nothing.
        for id in table.unrendered_general(catalog) {
            log::info!(
                "Sunrise: no entity renderer is registered for general entity type {id}; its entities draw nothing"
            );
        }
        let any_living = !table.living.is_empty();
        let any_tnt = table.tnt.is_some();
        let any_experience_orb = table.experience_orb.is_some();
        let any_painting = table.painting.is_some();
        let any_item_frame = table.item_frame.is_some() || table.glow_item_frame.is_some();
        let any_general = table.general.iter().any(Option::is_some);
        *TABLE.write().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(table));
        ANY_LIVING.store(any_living, Ordering::Relaxed);
        ANY_TNT.store(any_tnt, Ordering::Relaxed);
        ANY_EXPERIENCE_ORB.store(any_experience_orb, Ordering::Relaxed);
        ANY_PAINTING.store(any_painting, Ordering::Relaxed);
        ANY_ITEM_FRAME.store(any_item_frame, Ordering::Relaxed);
        ANY_GENERAL.store(any_general, Ordering::Relaxed);
        self.resolve_entity_layers(&rows, registered);
    }

    /// Draws `entity` with `resolved`'s renderer at `at` relative to the camera, with
    /// `world_origin` the same point in the level (`None` for a preview). `block` is the
    /// item owner's block. `light` is the entity pass's fixed light. True when it drew, and the
    /// type's own drawing is left out. False, with nothing it drew kept, when the renderer
    /// returns an `Err` (which does not disable it for later draws) or panics (which disables only
    /// this renderer for the rest of the session through its [`CallbackOwner`]).
    pub(in crate::game) fn mod_entity_renderer_geometry(
        &self,
        resolved: &RegisteredEntityRenderer,
        entity: &EntityRenderState,
        entity_type: &str,
        at: DVec3,
        world_origin: Option<DVec3>,
        block: [i32; 3],
        light: u32,
        out: &mut Vec<Vertex>,
    ) -> bool {
        let camera = world_origin
            .map(|origin| origin - at)
            .unwrap_or_else(|| DVec3::from_array(entity.position) - at);
        self.mod_entity_renderer_geometry_with_view(
            resolved,
            entity,
            entity_type,
            at,
            world_origin,
            camera,
            None,
            block,
            light,
            out,
        )
    }

    /// [`Self::mod_entity_renderer_geometry`] with an explicit [`mod_api::WorldRenderView`]
    /// (for screen scenes, GUI entity previews, and secondary views).
    #[allow(clippy::too_many_arguments)]
    pub(in crate::game) fn mod_entity_renderer_geometry_with_view(
        &self,
        resolved: &RegisteredEntityRenderer,
        entity: &EntityRenderState,
        entity_type: &str,
        at: DVec3,
        world_origin: Option<DVec3>,
        camera: DVec3,
        explicit_view: Option<&mod_api::WorldRenderView<'_>>,
        block: [i32; 3],
        light: u32,
        out: &mut Vec<Vertex>,
    ) -> bool {
        let pose = Mat4::from_translation(at.as_vec3());
        self.guarded_entity_draw(
            resolved.owner.as_ref(),
            "entity renderer",
            "EntityRenderer::submit",
            entity.partial,
            entity_type,
            block,
            pose,
            light,
            world_origin,
            camera,
            explicit_view,
            out,
            |draw| resolved.renderer.submit(entity, draw),
        )
    }

    /// Runs one mod renderer's or layer's `submit` on a draw posed at `pose` (`block` the block the
    /// entity stands in, lit by `light`). `world_origin` enables level light for a translation-only
    /// starting pose. Adds to `out` and returns true when it drew. An `Err` rolls back this draw's
    /// geometry and logs a warning without disabling the callback; a panic runs through
    /// [`CallbackOwner::call`], rolling back this draw and disabling only this callback for the
    /// session.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::game::client_side) fn guarded_entity_draw(
        &self,
        owner: Option<&CallbackOwner>,
        kind: &'static str,
        operation: &'static str,
        partial: f32,
        entity_type: &str,
        block: [i32; 3],
        pose: Mat4,
        light: u32,
        world_origin: Option<DVec3>,
        camera: DVec3,
        explicit_view: Option<&mod_api::WorldRenderView<'_>>,
        out: &mut Vec<Vertex>,
        submit: impl FnOnce(&mut dyn BlockEntityDrawContext) -> Result<(), String>,
    ) -> bool {
        if owner.is_some_and(|owner| !owner.is_enabled()) {
            return false;
        }
        let start = out.len();
        self.with_world_view(camera, partial, explicit_view, |view| {
            let mut surface = BlockEntityDraw::posed_with_view(
                view,
                light,
                block,
                pose,
                diffuse_lights(self.nether_lighting()),
                std::mem::take(out),
            )
            .with_quad_budget(
                self.entity_quad_capacity(),
                self.entity_batch_reserved_quads(),
            );
            if world_origin.is_some() {
                surface.track_entity_local_light_pose();
            }
            let run = || {
                submit(&mut GameBlockEntityDrawContext {
                    game: self,
                    surface: &mut surface,
                })
            };
            let outcome = match owner {
                Some(owner) => owner.call(kind, operation, run),
                None => contain_unwind(run),
            };
            match outcome {
                Ok(Ok(())) => {
                    let light_at = |[x, y, z]: [i32; 3]| self.common.world.light(x, y, z);
                    let level = world_origin.map(|origin| LevelLight {
                        origin,
                        light_at: &light_at,
                    });
                    *out = surface.into_vertices_with_light(level.as_ref());
                    true
                }
                Ok(Err(error)) => {
                    *out = surface.into_vertices();
                    out.truncate(start);
                    match owner {
                        Some(owner) => log::warn!(
                            "Sunrise: mod {}'s {kind} for {entity_type} failed ({error}); skipping this draw",
                            owner.id()
                        ),
                        None => log::warn!(
                            "Sunrise: {kind} for {entity_type} failed ({error}); skipping this draw"
                        ),
                    }
                    false
                }
                Err(error) => {
                    *out = surface.into_vertices();
                    out.truncate(start);
                    if owner.is_none() {
                        log::error!(
                            "Sunrise: {kind} for {entity_type} panicked ({error}); skipping this draw"
                        );
                    }
                    false
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_level::chunk::{Chunk, Light, index};

    #[test]
    #[ignore = "daily"]
    fn entity_renderer_samples_the_elements_world_cell_but_a_preview_stays_fixed() {
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "game::client_side::native_entity_renderers::tests::entity_renderer_samples_the_elements_world_cell_but_a_preview_stays_fixed",
            "SUNRISE_ENTITY_LEVEL_LIGHT_CHILD",
            &["world-and-preview"],
        ) {
            return;
        }

        struct LitQuad;
        impl EntityRenderer for LitQuad {
            fn submit(
                &self,
                _: &EntityRenderState,
                draw: &mut dyn BlockEntityDrawContext,
            ) -> Result<(), String> {
                draw.level_light();
                draw.translate([0.25, 0.0, 0.0]);
                assert!(draw.quad(
                    "block/stone",
                    [[0.0, 1.0, 0.0], [0.0; 3], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
                    [0.0, 0.0, 16.0, 16.0],
                    0xFF_FFFF,
                ));
                Ok(())
            }
        }

        super::super::native_entity_refs::tests::with_client_game(|game| {
            let mut chunk = Chunk::empty(0, 1);
            let mut cells = Box::new([0u8; 4096]);
            cells[index(11, 64, 4)] = 0x37;
            let section = chunk.section_index(64).unwrap();
            Arc::make_mut(&mut chunk.sections[section]).light = Light::packed(cells);
            game.world.chunks.insert((0, 1), Arc::new(chunk));

            let renderer = RegisteredEntityRenderer::new(Arc::new(LitQuad), None);
            let at = DVec3::new(10.875, 64.25, 20.375);
            let camera = DVec3::new(6.5, 63.0, 25.5);
            let entity = mod_api::__private::built::EntityRenderState {
                id: 1,
                partial: 0.5,
                age: 1.5,
                yaw: 0.0,
                pitch: 0.0,
                position: at.to_array(),
                size: [0.5, 1.0],
                eye_height: 0.85,
                distance_to_camera_sq: (at - camera).length_squared(),
                living: None,
                tnt_fuse: None,
                orb_value: None,
                painting: None,
                item_frame: None,
                attachments: None,
            }
            .into();
            for (world_origin, expected) in [(Some(at), 0x7030), (None, 0x50A0)] {
                let mut out = Vec::new();
                assert!(game.mod_entity_renderer_geometry(
                    &renderer,
                    &entity,
                    "test:lit_quad",
                    at - camera,
                    world_origin,
                    [10, 64, 20],
                    0x50A0,
                    &mut out,
                ));
                assert_eq!(
                    out.iter().map(|v| v.light).collect::<Vec<_>>(),
                    [expected; 4]
                );
            }
        });
    }
}
