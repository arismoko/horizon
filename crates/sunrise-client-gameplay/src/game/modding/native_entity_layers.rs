//! Layers on living entity renderers, inspected, hidden or added by mods on the client:
//! `native_register_entity_layers` (NeoForge's `EntityRenderersEvent.AddLayers`) is asked on every
//! resource load, after the entity renderers resolve, and the added layers and hidden stock-layer
//! masks it collects are resolved to the slot each draw reads: a mob's type (its shared `MobType`,
//! which the game's types never replace while it runs) or a player's model. A frame then skips
//! hidden stock layers and hands an entity to its own target's layers, after its model and
//! unhidden vanilla layers, each through the guard renderers draw through
//! ([`ClientGame::guarded_entity_draw`]), with a [`StockLayerDraw`] handle to redraw any hidden
//! stock layer conditionally per entity. With no layer added and no stock layer hidden, a frame
//! checks one atomic flag per entity and builds nothing.
use super::mob_blocks::MobView;
use super::stock_layer_mask::StockLayerMask;
use super::{
    entity_type_rows::EntityTypeRows,
    equipment_piece::{MOB_ARMOR, has_mob_armor_layer, stock_armor_layer},
    native_entity_renderers::RegisteredEntityRenderer,
    native_mob_draw::{entity_render_state, mob_pose},
};
use crate::game::ClientGame;
use glam::DVec3;
use mod_api::{
    __private::game::{CallbackOwner, RegistrationContextNew, discard_registration},
    ArmorLayerPass, ArmorLayerSlot, EntityLayers, EntityRenderLayer, FrameModel, Hand, HookId,
    LayerTarget, PlayerModel, RegistrationContext, StockLayer, StockLayerDraw,
};
use std::{
    collections::{BTreeMap, HashMap},
    ops::Range,
    sync::{
        Arc, LazyLock, Mutex, PoisonError, RwLock, Weak,
        atomic::{AtomicBool, Ordering},
    },
};
use sunrise_client::{mesher::Vertex, mobs::render};
use sunrise_level::fast_hash::FxHashMap;
use sunrise_sim::mobs::{Mob, MobType, Species};

/// The player's entity type: its renderers are the player models', so its layers go to those.
const PLAYER: &str = "minecraft:player";

/// The player models, each with a renderer of its own.
const PLAYER_MODELS: [PlayerModel; 2] = [PlayerModel::Wide, PlayerModel::Slim];

/// Stock layers drawn by `AvatarRenderer` for both player models, in vanilla registration order.
const PLAYER_STOCK_LAYERS: [StockLayer; 22] = [
    StockLayer::Armor(ArmorLayerSlot::Head, ArmorLayerPass::Base),
    StockLayer::Armor(ArmorLayerSlot::Head, ArmorLayerPass::Glint),
    StockLayer::Armor(ArmorLayerSlot::Head, ArmorLayerPass::Trim),
    StockLayer::Armor(ArmorLayerSlot::Chest, ArmorLayerPass::Base),
    StockLayer::Armor(ArmorLayerSlot::Chest, ArmorLayerPass::Glint),
    StockLayer::Armor(ArmorLayerSlot::Chest, ArmorLayerPass::Trim),
    StockLayer::Armor(ArmorLayerSlot::Legs, ArmorLayerPass::Base),
    StockLayer::Armor(ArmorLayerSlot::Legs, ArmorLayerPass::Glint),
    StockLayer::Armor(ArmorLayerSlot::Legs, ArmorLayerPass::Trim),
    StockLayer::Armor(ArmorLayerSlot::Feet, ArmorLayerPass::Base),
    StockLayer::Armor(ArmorLayerSlot::Feet, ArmorLayerPass::Glint),
    StockLayer::Armor(ArmorLayerSlot::Feet, ArmorLayerPass::Trim),
    StockLayer::HeldItem(Hand::Main),
    StockLayer::HeldItem(Hand::Off),
    StockLayer::Arrow,
    StockLayer::Deadmau5Ears,
    StockLayer::Cape,
    StockLayer::CustomHead,
    StockLayer::Wings,
    StockLayer::ParrotOnShoulder,
    StockLayer::SpinAttackEffect,
    StockLayer::BeeStinger,
];

const fn mask_of(layers: &[StockLayer]) -> StockLayerMask {
    let mut mask = StockLayerMask::EMPTY;
    let mut i = 0;
    while i < layers.len() {
        mask = mask.with(layers[i]);
        i += 1;
    }
    mask
}

const PLAYER_STOCK_MASK: StockLayerMask = mask_of(&PLAYER_STOCK_LAYERS);

/// Stock layers supported by `kind`'s vanilla renderer, resolved once at resource load.
fn stock_layers_for_mob(
    kind: &Arc<MobType>,
    data: &dyn render::RenderDataSource,
) -> Vec<StockLayer> {
    let mob = Mob::new(0, kind.clone(), sunrise_core::geometry::Vec3::ZERO, 0.0);
    let mut out = Vec::new();
    if kind.species == Species::SnowGolem {
        out.push(StockLayer::SnowGolemHead);
    }
    if has_mob_armor_layer(&mob) {
        let models = &data.mob_tables().models;
        for (_, key, _, slot) in MOB_ARMOR {
            if render::equipment_model_key(&mob, key, models).is_some() {
                for pass in [
                    ArmorLayerPass::Base,
                    ArmorLayerPass::Glint,
                    ArmorLayerPass::Trim,
                ] {
                    out.push(stock_armor_layer(slot, pass));
                }
            }
        }
    }
    let (custom_head, main_hand, off_hand) = render::stock_item_layers(&mob, data);
    if custom_head {
        out.push(StockLayer::CustomHead);
    }
    if main_hand {
        out.push(StockLayer::HeldItem(Hand::Main));
    }
    if off_hand {
        out.push(StockLayer::HeldItem(Hand::Off));
    }
    out
}

/// One registered entity layer and its session callback owner (if added by a mod).
pub struct RegisteredLayer {
    layer: Arc<dyn EntityRenderLayer>,
    owner: Option<CallbackOwner>,
}

impl RegisteredLayer {
    fn is_enabled(&self) -> bool {
        self.owner.as_ref().is_none_or(CallbackOwner::is_enabled)
    }
}

/// One target's layers, in the order they were added.
pub(in crate::game) type Layers = Arc<[RegisteredLayer]>;

/// What one mob lookup finds in the resolved layer table: added mod layers, hidden stock layers
/// and supported stock layers.
#[derive(Clone, Default)]
pub(in crate::game) struct ResolvedEntityLayers {
    pub(in crate::game) layers: Option<Layers>,
    pub(in crate::game) hidden: StockLayerMask,
    pub(in crate::game) supported: StockLayerMask,
}

/// Draw-site parameters needed when a mod layer calls [`StockLayerDraw::draw_stock_layer`].
#[derive(Clone, Copy)]
pub struct StockLayerContext<'a> {
    pub(in crate::game) is_player: bool,
    pub(in crate::game) local_player: bool,
    pub(in crate::game) invisible: bool,
    pub(in crate::game) in_world: bool,
    pub(in crate::game) player_name: Option<&'a str>,
    pub(in crate::game) alpha: f64,
    pub(in crate::game) camera: DVec3,
    pub(in crate::game) light: u32,
    pub(in crate::game) supported: StockLayerMask,
    pub(in crate::game) hidden: StockLayerMask,
}

impl StockLayerContext<'_> {
    /// A context with no stock layers supported or hidden (e.g. direct unit-test layer draws).
    #[cfg(any(test, feature = "test-support"))]
    pub fn none(alpha: f64, camera: DVec3, light: u32) -> Self {
        Self {
            is_player: false,
            local_player: false,
            invisible: false,
            in_world: true,
            player_name: None,
            alpha,
            camera,
            light,
            supported: StockLayerMask::EMPTY,
            hidden: StockLayerMask::EMPTY,
        }
    }
}

pub(in crate::game) struct LayerOwner {
    layer: Weak<dyn EntityRenderLayer>,
    owner: CallbackOwner,
}

type LayerOwners = HashMap<(&'static str, usize), LayerOwner>;

/// Session owners keyed by mod and callback allocation, independent of target and layer order.
/// The weak reference keeps the address from being reused while the entry exists, without
/// keeping the callback alive after a reload replaces it.
static LAYER_OWNERS: LazyLock<Mutex<LayerOwners>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Whether the last load resolved any added layer or hidden stock layer: a frame checks only this
/// when none was registered.
static ANY: AtomicBool = AtomicBool::new(false);

/// The table the last resource load resolved; none before the first.
static TABLE: RwLock<Option<LayerTable>> = RwLock::new(None);

type LayerEdit = (Option<&'static str>, Arc<dyn EntityRenderLayer>);

/// What one `register_entity_layers` round collects: the [`EntityLayers`] mods are handed, with
/// what this load's renderers are, and the layers added to each target.
pub struct LayerRegistration<'a> {
    rows: &'a EntityTypeRows,
    mod_renderers: &'a BTreeMap<String, RegisteredEntityRenderer>,
    /// Whether a living type draws where layers run (as a mob).
    routed: &'a dyn Fn(&str) -> bool,
    stock_by_entity: &'a BTreeMap<String, Vec<StockLayer>>,
    entity: BTreeMap<String, Vec<LayerEdit>>,
    wide: Vec<LayerEdit>,
    slim: Vec<LayerEdit>,
    hidden_entity: BTreeMap<String, StockLayerMask>,
    hidden_wide: StockLayerMask,
    hidden_slim: StockLayerMask,
}

impl<'a> LayerRegistration<'a> {
    pub fn new(
        rows: &'a EntityTypeRows,
        mod_renderers: &'a BTreeMap<String, RegisteredEntityRenderer>,
        routed: &'a dyn Fn(&str) -> bool,
        stock_by_entity: &'a BTreeMap<String, Vec<StockLayer>>,
    ) -> Self {
        Self {
            rows,
            mod_renderers,
            routed,
            stock_by_entity,
            entity: BTreeMap::new(),
            wide: Vec::new(),
            slim: Vec::new(),
            hidden_entity: BTreeMap::new(),
            hidden_wide: StockLayerMask::EMPTY,
            hidden_slim: StockLayerMask::EMPTY,
        }
    }

    pub(in crate::game) fn register_listener(
        &mut self,
        listen: &mut dyn FnMut(&mut dyn EntityLayers) -> bool,
    ) {
        let mut pending = Self::new(
            self.rows,
            self.mod_renderers,
            self.routed,
            self.stock_by_entity,
        );
        if listen(&mut pending) {
            for (id, layers) in pending.entity {
                self.entity.entry(id).or_default().extend(layers);
            }
            self.wide.extend(pending.wide);
            self.slim.extend(pending.slim);
            for (id, mask) in pending.hidden_entity {
                let entry = self.hidden_entity.entry(id).or_default();
                *entry = entry.union(mask);
            }
            self.hidden_wide = self.hidden_wide.union(pending.hidden_wide);
            self.hidden_slim = self.hidden_slim.union(pending.hidden_slim);
        } else {
            for (owner, layer) in pending
                .entity
                .into_values()
                .flatten()
                .chain(pending.wide)
                .chain(pending.slim)
            {
                discard_registration(
                    (HookId::RegisterEntityLayers, "register_entity_layers"),
                    owner,
                    layer,
                );
            }
        }
    }

    /// Why `id` can't take a layer; none for a living entity type.
    fn refusal(&self, id: &str) -> Option<&'static str> {
        if id == PLAYER {
            Some("a player takes layers through its model (wide or slim)")
        } else if !self.rows.contains(id) {
            Some("it is no entity type")
        } else if !self.rows.is_living(id) {
            Some("its renderer is not a living entity's, and only those take layers")
        } else if !(self.routed)(id) {
            Some("it draws through a route of its own, which does not run layers yet")
        } else {
            None
        }
    }
}

impl EntityLayers for LayerRegistration<'_> {
    fn entity_types(&self) -> &[String] {
        &self.rows.non_player_ids
    }

    fn is_living(&self, entity_type: &str) -> bool {
        entity_type != PLAYER && self.rows.is_living(entity_type)
    }

    fn player_models(&self) -> &[PlayerModel] {
        &PLAYER_MODELS
    }

    fn has_mod_renderer(&self, entity_type: &str) -> bool {
        self.rows.contains(entity_type) && self.mod_renderers.contains_key(entity_type)
    }

    fn stock_layers(&self, target: &LayerTarget) -> &[StockLayer] {
        match target {
            LayerTarget::Player(PlayerModel::Wide | PlayerModel::Slim) => &PLAYER_STOCK_LAYERS,
            LayerTarget::Entity(id) => {
                if self.refusal(id).is_some() || self.has_mod_renderer(id) {
                    return &[];
                }
                self.stock_by_entity.get(id).map_or(&[], Vec::as_slice)
            }
        }
    }

    fn hide(&mut self, target: LayerTarget, layer: StockLayer) -> Result<(), String> {
        match target {
            LayerTarget::Player(model) => {
                if !PLAYER_STOCK_MASK.contains(layer) {
                    let why = format!(
                        "hiding stock layer {layer:?} on player ({model:?}) is refused: its renderer does not have {layer:?}"
                    );
                    log::warn!("Sunrise: {why}");
                    return Err(why);
                }
                match model {
                    PlayerModel::Wide => self.hidden_wide.insert(layer),
                    PlayerModel::Slim => self.hidden_slim.insert(layer),
                }
                Ok(())
            }
            LayerTarget::Entity(id) => {
                if let Some(why) = self.refusal(&id) {
                    let why = format!("hiding stock layer {layer:?} on {id} is refused: {why}");
                    log::warn!("Sunrise: {why}");
                    return Err(why);
                }
                if self.has_mod_renderer(&id) {
                    let why = format!(
                        "hiding stock layer {layer:?} on {id} is refused: a mod replaced its renderer, which has no stock layers"
                    );
                    log::warn!("Sunrise: {why}");
                    return Err(why);
                }
                if !self
                    .stock_by_entity
                    .get(&id)
                    .is_some_and(|layers| layers.contains(&layer))
                {
                    let why = format!(
                        "hiding stock layer {layer:?} on {id} is refused: its renderer does not have {layer:?}"
                    );
                    log::warn!("Sunrise: {why}");
                    return Err(why);
                }
                self.hidden_entity.entry(id).or_default().insert(layer);
                Ok(())
            }
        }
    }

    fn add(
        &mut self,
        target: LayerTarget,
        layer: Arc<dyn EntityRenderLayer>,
    ) -> Result<(), String> {
        let layers = match target {
            LayerTarget::Player(PlayerModel::Wide) => &mut self.wide,
            LayerTarget::Player(PlayerModel::Slim) => &mut self.slim,
            LayerTarget::Entity(id) => {
                if let Some(why) = self.refusal(&id) {
                    let why = format!("the entity layer for {id} is refused: {why}");
                    log::warn!("Sunrise: {why}");
                    return Err(why);
                }
                self.entity.entry(id).or_default()
            }
        };
        layers.push((mod_api::content::crash::current_native_mod(), layer));
        Ok(())
    }
}

/// The layers and hidden stock-layer masks by the slot each draw reads: rebuilt on every resource
/// load.
#[derive(Default)]
pub(in crate::game) struct LayerTable {
    /// By the address of the mob type the game shares with every mob of the type.
    mobs: FxHashMap<usize, Layers>,
    wide: Option<Layers>,
    slim: Option<Layers>,
    hidden_mobs: FxHashMap<usize, StockLayerMask>,
    hidden_wide: StockLayerMask,
    hidden_slim: StockLayerMask,
    stock_mobs: FxHashMap<usize, StockLayerMask>,
}

impl LayerTable {
    /// Resolve the successfully registered layers and hides against the types the game draws as
    /// mobs.
    pub(in crate::game) fn resolve(
        registration: LayerRegistration<'_>,
        mob_types: &HashMap<String, Arc<MobType>>,
        owners: &Mutex<LayerOwners>,
    ) -> Self {
        let resolved = |layers: Vec<LayerEdit>| -> Option<Layers> {
            (!layers.is_empty()).then(|| {
                layers
                    .into_iter()
                    .map(|(mod_id, layer)| {
                        let owner = mod_id.map(|mod_id| {
                            owners
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner)
                                .entry((mod_id, Arc::as_ptr(&layer).cast::<()>() as usize))
                                .or_insert_with(|| LayerOwner {
                                    layer: Arc::downgrade(&layer),
                                    owner: CallbackOwner::for_mod(mod_id),
                                })
                                .owner
                                .clone()
                        });
                        RegisteredLayer { layer, owner }
                    })
                    .collect()
            })
        };
        let stock_mobs = registration
            .stock_by_entity
            .iter()
            .filter_map(|(id, layers)| {
                if registration.has_mod_renderer(id) {
                    return None;
                }
                let kind = mob_types.get(id)?;
                let mask = mask_of(layers);
                (!mask.is_empty()).then_some((Arc::as_ptr(kind) as usize, mask))
            })
            .collect();
        let hidden_mobs = registration
            .hidden_entity
            .into_iter()
            .filter_map(|(id, mask)| {
                if mask.is_empty() {
                    return None;
                }
                let kind = mob_types.get(&id)?;
                Some((Arc::as_ptr(kind) as usize, mask))
            })
            .collect();
        let mobs = registration
            .entity
            .into_iter()
            .filter_map(|(id, layers)| {
                let kind = mob_types.get(&id)?;
                Some((Arc::as_ptr(kind) as usize, resolved(layers)?))
            })
            .collect();
        Self {
            mobs,
            wide: resolved(registration.wide),
            slim: resolved(registration.slim),
            hidden_mobs,
            hidden_wide: registration.hidden_wide,
            hidden_slim: registration.hidden_slim,
            stock_mobs,
        }
    }

    fn is_empty(&self) -> bool {
        self.mobs.is_empty()
            && self.wide.is_none()
            && self.slim.is_none()
            && self.hidden_mobs.is_empty()
            && self.hidden_wide.is_empty()
            && self.hidden_slim.is_empty()
    }

    /// The added layers of `mob`: its player model's for a player, else its type's.
    pub(in crate::game) fn of(&self, mob: &Mob) -> Option<&Layers> {
        match (&*mob.kind.id == PLAYER, mob.slim) {
            (true, true) => self.slim.as_ref(),
            (true, false) => self.wide.as_ref(),
            (false, _) => self.mobs.get(&(Arc::as_ptr(&mob.kind) as usize)),
        }
    }

    /// The added layers, hidden stock layers and supported stock layers for `mob`.
    fn resolve_for(&self, mob: &Mob) -> ResolvedEntityLayers {
        let layers = self
            .of(mob)
            .filter(|layers| layers.iter().any(RegisteredLayer::is_enabled))
            .cloned();
        let (hidden, supported) = match (&*mob.kind.id == PLAYER, mob.slim) {
            (true, true) => (self.hidden_slim, PLAYER_STOCK_MASK),
            (true, false) => (self.hidden_wide, PLAYER_STOCK_MASK),
            (false, _) => {
                let key = Arc::as_ptr(&mob.kind) as usize;
                (
                    self.hidden_mobs
                        .get(&key)
                        .copied()
                        .unwrap_or(StockLayerMask::EMPTY),
                    self.stock_mobs
                        .get(&key)
                        .copied()
                        .unwrap_or(StockLayerMask::EMPTY),
                )
            }
        };
        ResolvedEntityLayers {
            layers,
            hidden,
            supported,
        }
    }
}

struct StockDraw<'a, 'g> {
    game: &'a ClientGame<'g>,
    mob: &'a Mob,
    entity_id: u32,
    ctx: &'a StockLayerContext<'a>,
    out: &'a mut Vec<Vertex>,
    wings_range: Option<Range<usize>>,
}

impl<'a> StockDraw<'a, '_> {
    fn chest(&self) -> Option<&'a str> {
        if self.ctx.local_player {
            self.game.common.armor[1].as_ref().map(|s| &*s.item)
        } else {
            self.mob.extra.slot_id(sunrise_items::equipment::CHEST)
        }
    }
}

impl StockLayerDraw for StockDraw<'_, '_> {
    fn draw_stock_layer(&mut self, layer: StockLayer) -> Result<(), String> {
        if !self.ctx.supported.contains(layer) {
            return Err(format!(
                "stock layer {layer:?} is not supported by {}'s renderer",
                self.mob.kind.id
            ));
        }
        if !self.ctx.hidden.contains(layer) {
            return Err(format!(
                "stock layer {layer:?} is not hidden on {}'s renderer",
                self.mob.kind.id
            ));
        }
        let partial = self.ctx.alpha as f32;
        match layer {
            StockLayer::Armor(_, _) => {
                let only_this = StockLayerMask::ALL_ARMOR.without(layer);
                let mut run = || {
                    if self.ctx.is_player {
                        let extra = if self.ctx.local_player {
                            self.game.player_armor_layers(only_this)
                        } else {
                            self.game.worn_armor_layers(self.mob, only_this)
                        };
                        render::render_extra_layers(
                            self.mob,
                            partial,
                            self.ctx.camera,
                            self.ctx.light,
                            self.game,
                            &extra,
                            self.out,
                        );
                    } else {
                        let equipment = self.game.mob_equipment_layers(
                            self.mob,
                            super::equipment_piece::EquipmentPass::Draw,
                            only_this,
                        );
                        let atlas = &self.game.client.gui.atlas;
                        let lookup =
                            |name: &str| atlas.sprites.get(name).map(|r| [r.u0, r.v0, r.u1, r.v1]);
                        render::render_equipment_layers(
                            self.mob,
                            render::LayerFrame {
                                alpha: partial,
                                camera: self.ctx.camera,
                                light: self.ctx.light,
                            },
                            &lookup,
                            self.game,
                            equipment,
                            self.out,
                        );
                    }
                };
                if self.ctx.in_world {
                    run();
                } else {
                    sunrise_client::mobs::cem_anim::in_gui(run);
                }
            }
            StockLayer::SnowGolemHead => {
                self.game.snow_golem_head_geometry(
                    self.mob,
                    self.ctx.camera,
                    self.ctx.alpha,
                    self.ctx.light,
                    self.out,
                );
            }
            StockLayer::CustomHead => {
                if !self.ctx.is_player {
                    self.game.captain_banner_geometry(
                        self.mob,
                        self.ctx.camera,
                        self.ctx.alpha,
                        self.ctx.light,
                        self.out,
                    );
                }
                self.game.head_item_geometry(
                    self.mob,
                    self.ctx.camera,
                    self.ctx.alpha,
                    self.ctx.light,
                    self.ctx.in_world,
                    self.out,
                );
            }
            StockLayer::HeldItem(hand) => {
                let hide_other = match hand {
                    Hand::Main => StockLayerMask::only(StockLayer::HeldItem(Hand::Off)),
                    Hand::Off => StockLayerMask::only(StockLayer::HeldItem(Hand::Main)),
                };
                self.game.held_item_geometry(
                    self.mob,
                    MobView {
                        camera: self.ctx.camera,
                        alpha: self.ctx.alpha,
                        light: self.ctx.light,
                    },
                    self.ctx.in_world,
                    hide_other,
                    None,
                    self.out,
                );
            }
            StockLayer::Cape => {
                if !self.ctx.invisible {
                    self.game.player_cape_geometry(
                        self.mob,
                        self.chest(),
                        self.ctx.camera,
                        partial,
                        self.ctx.light,
                        self.out,
                    );
                }
            }
            StockLayer::Deadmau5Ears => {
                if !self.ctx.invisible
                    && let Some(name) = self.ctx.player_name
                {
                    self.game.player_ears_geometry(
                        self.mob,
                        name,
                        self.ctx.camera,
                        partial,
                        self.ctx.light,
                        self.out,
                    );
                }
            }
            StockLayer::Wings => {
                let start = self.out.len();
                self.game.player_elytra_geometry(
                    self.mob,
                    self.chest(),
                    self.ctx.camera,
                    partial,
                    self.ctx.light,
                    self.out,
                );
                if self.out.len() > start {
                    self.wings_range = Some(start..self.out.len());
                }
            }
            StockLayer::ParrotOnShoulder => {
                self.game.shoulder_parrots_geometry(
                    self.mob,
                    self.ctx.camera,
                    partial,
                    self.ctx.light,
                    self.out,
                );
            }
            StockLayer::SpinAttackEffect => {
                if self.ctx.in_world {
                    self.game.spin_attack_geometry(
                        self.mob,
                        self.ctx.camera,
                        partial,
                        self.ctx.light,
                        self.out,
                    );
                }
            }
            StockLayer::Arrow => {
                if self.ctx.in_world {
                    self.game.stuck_in_body_geometry(
                        self.mob,
                        self.entity_id as i32,
                        MobView {
                            camera: self.ctx.camera,
                            alpha: f64::from(partial),
                            light: self.ctx.light,
                        },
                        StockLayerMask::only(StockLayer::BeeStinger),
                        self.out,
                    );
                }
            }
            StockLayer::BeeStinger => {
                if self.ctx.in_world {
                    self.game.stuck_in_body_geometry(
                        self.mob,
                        self.entity_id as i32,
                        MobView {
                            camera: self.ctx.camera,
                            alpha: f64::from(partial),
                            light: self.ctx.light,
                        },
                        StockLayerMask::only(StockLayer::Arrow),
                        self.out,
                    );
                }
            }
        }
        Ok(())
    }
}

impl crate::game::ClientGame<'_> {
    /// The layers and hidden stock-layer mask the last load resolved for `mob` (a player's model's
    /// for a player), looked up once per mob a frame. With no layer added and no stock layer hidden
    /// at all, returns empty after one atomic check, with no lock and no lookup.
    pub(in crate::game) fn entity_layers(&self, mob: &Mob) -> ResolvedEntityLayers {
        if !ANY.load(Ordering::Relaxed) {
            return ResolvedEntityLayers::default();
        }
        let table = TABLE.read().unwrap_or_else(PoisonError::into_inner);
        table
            .as_ref()
            .map_or_else(ResolvedEntityLayers::default, |t| t.resolve_for(mob))
    }

    /// The added layers the last load resolved for `mob` (a player's model's for a player).
    #[cfg(any(test, feature = "test-support"))]
    pub fn mod_layers(&self, mob: &Mob) -> Option<Layers> {
        self.entity_layers(mob).layers
    }

    /// Asks the mods for their layers and stock-layer hides on the renderers this load built
    /// (`rows` its entity types, `mod_renderers` the registered renderers) and resolves them.
    pub(in crate::game::client_side) fn resolve_entity_layers(
        &self,
        rows: &EntityTypeRows,
        mod_renderers: &BTreeMap<String, RegisteredEntityRenderer>,
    ) {
        let mob_types = self.common.res.mob_types();
        let routed = |id: &str| mob_types.contains_key(id);
        let stock_by_entity: BTreeMap<String, Vec<StockLayer>> = mob_types
            .iter()
            .map(|(id, kind)| (id.clone(), stock_layers_for_mob(kind, self)))
            .collect();
        let mut registration =
            LayerRegistration::new(rows, mod_renderers, &routed, &stock_by_entity);
        crate::game::client_side::native_hooks_client::register_entity_layers::native_register_entity_layers(
            &RegistrationContext::new(),
            &mut registration,
        );
        let table = LayerTable::resolve(registration, mob_types, &LAYER_OWNERS);
        let any = !table.is_empty();
        *TABLE.write().unwrap_or_else(PoisonError::into_inner) = Some(table);
        ANY.store(any, Ordering::Relaxed);
        // Drop stale identities after the previous table releases its layers.
        LAYER_OWNERS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|_, entry| entry.layer.strong_count() != 0);
    }

    /// Draws `layers` over `mob`, told as the entity `id`, each from its frame (at the feet,
    /// turned with the body), in the order they were added, using `view` (or the current
    /// world view when `None`). Each layer borrows the same final `model`, if this draw
    /// produced a body pose, and may redraw hidden stock layers via [`StockLayerDraw`]. Returns
    /// the vertex range in `out` of any redrawn `Wings` layer (used by the GUI player preview to
    /// disable back-face culling on elytra quads).
    #[allow(clippy::too_many_arguments)]
    pub(in crate::game) fn registered_layer_geometry_with_view(
        &self,
        mob: &Mob,
        id: u32,
        layers: &Layers,
        model: Option<&dyn FrameModel>,
        stock_ctx: &StockLayerContext<'_>,
        partial: f32,
        render_origin: DVec3,
        explicit_view: Option<&mod_api::WorldRenderView<'_>>,
        light: u32,
        out: &mut Vec<Vertex>,
    ) -> Option<Range<usize>> {
        let view_camera = explicit_view.map_or(render_origin, |v| DVec3::from(v.camera.position));
        let entity = entity_render_state(mob, id, partial, view_camera);
        let feet = mob.previous.lerp(mob.position, f64::from(partial));
        let feet = DVec3::new(feet.x, feet.y, feet.z);
        let pose = mob_pose(
            feet - render_origin,
            entity.living.expect("mob living state").body_rot,
        );
        let block = [feet.x, feet.y, feet.z].map(|c| c.floor() as i32);
        let mut wings_range = None;
        for layer in layers.iter() {
            let mut stock_out = Vec::new();
            let mut layer_wings = None;
            let drew = self.guarded_entity_draw(
                layer.owner.as_ref(),
                "entity layer",
                "EntityRenderLayer::submit",
                partial,
                &mob.kind.id,
                block,
                pose,
                light,
                None,
                view_camera,
                explicit_view,
                out,
                |draw| {
                    let mut stock = StockDraw {
                        game: self,
                        mob,
                        entity_id: id,
                        ctx: stock_ctx,
                        out: &mut stock_out,
                        wings_range: None,
                    };
                    let result = layer.layer.submit(&entity, model, draw, &mut stock);
                    layer_wings = stock.wings_range;
                    result
                },
            );
            if drew && !stock_out.is_empty() {
                let base = out.len();
                out.extend_from_slice(&stock_out);
                if let Some(range) = layer_wings {
                    wings_range = Some((base + range.start)..(base + range.end));
                }
            }
        }
        wings_range
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::{
        BlockEntityDrawContext, EntityRenderState, Hooks, ModEntry, RegisterEntityLayersEvent,
    };

    struct NoDraw;
    impl EntityRenderLayer for NoDraw {
        fn submit(
            &self,
            _: &EntityRenderState,
            _: Option<&dyn FrameModel>,
            _: &mut dyn BlockEntityDrawContext,
            _: &mut dyn StockLayerDraw,
        ) -> Result<(), String> {
            Ok(())
        }
    }

    struct Registrant {
        layer: Arc<dyn EntityRenderLayer>,
        hide: Option<(LayerTarget, StockLayer)>,
        panic_after: Option<Arc<AtomicBool>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Registrant {
        fn register_entity_layers(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterEntityLayersEvent<'_>,
        ) {
            for target in [
                LayerTarget::Entity("test:golem".into()),
                LayerTarget::Player(PlayerModel::Wide),
                LayerTarget::Player(PlayerModel::Slim),
            ] {
                event.layers.add(target, self.layer.clone()).unwrap();
            }
            if let Some((target, stock_layer)) = &self.hide {
                event.layers.hide(target.clone(), *stock_layer).unwrap();
            }
            if let Some(added) = &self.panic_after {
                added.store(true, Ordering::Relaxed);
                panic!("after adding all three layers");
            }
        }
    }

    #[test]
    fn a_panicking_layer_registrar_does_not_publish_its_layers_or_hides() {
        let added = Arc::new(AtomicBool::new(false));
        let earlier: Arc<dyn EntityRenderLayer> = Arc::new(NoDraw);
        let later: Arc<dyn EntityRenderLayer> = Arc::new(NoDraw);
        mod_platform::testing::with_mods(
            vec![
                ModEntry::new(
                    "earlier",
                    Box::new(Registrant {
                        layer: earlier.clone(),
                        hide: Some((LayerTarget::Player(PlayerModel::Wide), StockLayer::Cape)),
                        panic_after: None,
                    }),
                ),
                ModEntry::new(
                    "failed",
                    Box::new(Registrant {
                        layer: Arc::new(NoDraw),
                        hide: Some((LayerTarget::Player(PlayerModel::Slim), StockLayer::Wings)),
                        panic_after: Some(added.clone()),
                    }),
                ),
                ModEntry::new(
                    "later",
                    Box::new(Registrant {
                        layer: later.clone(),
                        hide: Some((
                            LayerTarget::Entity("test:golem".into()),
                            StockLayer::SnowGolemHead,
                        )),
                        panic_after: None,
                    }),
                ),
            ]
            .into(),
            || {
                let rows = EntityTypeRows::parse(br#"{"test:golem":{"attributes":{}}}"#);
                let renderers = BTreeMap::new();
                let stock =
                    BTreeMap::from([("test:golem".into(), vec![StockLayer::SnowGolemHead])]);
                let mut registration = LayerRegistration::new(&rows, &renderers, &|_| true, &stock);
                crate::game::client_side::native_hooks_client::register_entity_layers::native_register_entity_layers(
                    &RegistrationContext::new(),
                    &mut registration,
                );
                assert!(added.load(Ordering::Relaxed));
                let entity = &registration.entity["test:golem"];
                assert_eq!(entity.len(), 2);
                assert!(Arc::ptr_eq(&entity[0].1, &earlier));
                assert!(Arc::ptr_eq(&entity[1].1, &later));
                assert!(registration.hidden_wide.contains(StockLayer::Cape));
                assert!(!registration.hidden_slim.contains(StockLayer::Wings));
                assert!(
                    registration.hidden_entity["test:golem"].contains(StockLayer::SnowGolemHead)
                );
                let owners = Mutex::default();
                let table = LayerTable::resolve(registration, &HashMap::new(), &owners);
                assert!(table.hidden_wide.contains(StockLayer::Cape));
                assert!(table.hidden_slim.is_empty());
                for list in [table.wide.unwrap(), table.slim.unwrap()] {
                    assert_eq!(list.len(), 2);
                    assert!(Arc::ptr_eq(&list[0].layer, &earlier));
                    assert!(Arc::ptr_eq(&list[1].layer, &later));
                }
            },
        );
    }
}
