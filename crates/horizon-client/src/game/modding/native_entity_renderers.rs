//! Entity types drawn by mods' renderers on the client: `native_register_entity_renderers`
//! (NeoForge's `EntityRenderersEvent.RegisterRenderers`) is asked once at start, before resources
//! load, and every resource load resolves the registrations into the slots the draw routes read, so
//! a frame hands an entity to its type's renderer without a lookup by name or a scan over the mods.
//! The renderer draws through the shared drawing boundary (`BlockEntityDrawContext`) for native and
//! wasm mods alike.
//!
//! The routes that read the table so far: the content mods' thrown types
//! (`ProjectileKind::Thrown`, by [`ThrownType::slot`]). Each other entity family becomes a route of
//! its own; until then a registration for one of its types is kept and the type keeps its drawing.
use super::{
    Game, host::draw::GameBlockEntityDrawContext, native_block_entity_draw::BlockEntityDraw,
};
use glam::{DVec3, Mat4};
use horizon_client::{
    block_entity_render::diffuse_lights, mesher::Vertex, projectiles::ThrownType,
};
use mod_api::{EntityRenderState, EntityRenderer};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, OnceLock, PoisonError, RwLock,
        atomic::{AtomicBool, Ordering},
    },
};

/// The registrations, by entity type, the last one for a type winning: kept once at start.
static REGISTERED: OnceLock<BTreeMap<String, Arc<dyn EntityRenderer>>> = OnceLock::new();

/// The table the last resource load resolved; empty before the first.
static TABLE: RwLock<EntityRendererTable> = RwLock::new(EntityRendererTable { thrown: Vec::new() });

/// Keeps the mods' registrations, once per process (false when they already were).
pub(super) fn register(renderers: BTreeMap<String, Arc<dyn EntityRenderer>>) -> bool {
    REGISTERED.set(renderers).is_ok()
}

/// One type's resolved renderer, and whether it has failed (then it is asked no more until the
/// next resource load resolves the table again).
pub(in crate::game) struct Resolved {
    renderer: Arc<dyn EntityRenderer>,
    failed: AtomicBool,
}

/// The registered renderers, by the slot each draw route reads: rebuilt on every resource load.
pub(super) struct EntityRendererTable {
    /// By [`ThrownType::slot`].
    thrown: Vec<Option<Arc<Resolved>>>,
}

impl EntityRendererTable {
    /// The table for `registered`, with `thrown` the content mods' thrown types and `known`
    /// whether an id is any entity type; and the ids of the registrations for no entity type,
    /// which it leaves out. A registration for a known type no route reads yet is kept silently:
    /// the type keeps its own drawing.
    pub(super) fn resolve(
        registered: &BTreeMap<String, Arc<dyn EntityRenderer>>,
        thrown: &[ThrownType],
        known: impl Fn(&str) -> bool,
    ) -> (Self, Vec<String>) {
        let mut table = Self {
            thrown: thrown.iter().map(|_| None).collect(),
        };
        let mut unknown = Vec::new();
        for (id, renderer) in registered {
            if let Some(t) = thrown.iter().find(|t| t.id == id) {
                table.thrown[t.slot] = Some(Arc::new(Resolved {
                    renderer: renderer.clone(),
                    failed: AtomicBool::new(false),
                }));
            } else if !known(id) {
                unknown.push(id.clone());
            }
        }
        (table, unknown)
    }

    /// The renderer of the thrown type `t`, when a mod registered one.
    pub(super) fn thrown(&self, t: &ThrownType) -> Option<&Arc<Resolved>> {
        self.thrown.get(t.slot)?.as_ref()
    }
}

/// The renderer the last resource load resolved for the thrown type `t`, when a mod registered one.
pub(in crate::game) fn thrown_renderer(t: &ThrownType) -> Option<Arc<Resolved>> {
    let table = TABLE.read().unwrap_or_else(PoisonError::into_inner);
    table.thrown(t).cloned()
}

impl Game<'_> {
    /// Resolves the registered renderers against the entity types of the resources now loaded:
    /// at the first load and on every reload after it. A registration for no entity type is
    /// logged with its id.
    pub(in crate::game) fn resolve_entity_renderers(&self) {
        let Some(registered) = REGISTERED.get() else {
            return;
        };
        let root = &self.options.resources;
        let (table, unknown) = EntityRendererTable::resolve(
            registered,
            horizon_client::projectiles::thrown_types(),
            |id| super::chat::is_entity_type(root, id),
        );
        for id in unknown {
            log::warn!(
                "Horizon: an entity renderer is registered for {id}, which is no entity type; it is left out"
            );
        }
        *TABLE.write().unwrap_or_else(PoisonError::into_inner) = table;
    }

    /// Draws `entity` with `resolved`'s renderer, at `at` relative to the camera (`block` the
    /// block it stands in), lit by `light` as the entity pass packs it: true when it drew, and the
    /// type's own drawing is left out. False, with nothing it drew kept, when it fails this frame
    /// (an `Err`, a panic, a refused or trapped wasm call) and from then until the next resource
    /// load resolves it again.
    pub(in crate::game) fn mod_entity_renderer_geometry(
        &self,
        resolved: &Resolved,
        entity: &EntityRenderState,
        at: DVec3,
        block: [i32; 3],
        light: u32,
        out: &mut Vec<Vertex>,
    ) -> bool {
        if resolved.failed.load(Ordering::Relaxed) {
            return false;
        }
        let start = out.len();
        let mut surface = BlockEntityDraw::posed(
            entity.partial,
            self.world.game_time,
            light,
            block,
            Mat4::from_translation(at.as_vec3()),
            diffuse_lights(self.nether_lighting()),
            std::mem::take(out),
        )
        .animated(self.animation_time());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut draw = GameBlockEntityDrawContext {
                game: self,
                surface: &mut surface,
            };
            resolved.renderer.submit(entity, &mut draw)
        }))
        .unwrap_or_else(|_| Err("it panicked".into()));
        *out = surface.into_vertices();
        let Err(error) = result else {
            return true;
        };
        out.truncate(start);
        if !resolved.failed.swap(true, Ordering::Relaxed) {
            log::warn!(
                "Horizon: an entity renderer failed ({error}); it is asked no more until resources reload"
            );
        }
        false
    }
}
