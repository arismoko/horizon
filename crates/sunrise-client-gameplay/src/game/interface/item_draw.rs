//! Items drawn through the resolved item model pipeline
//! (`sunrise_client::item_render`): GUI icons (`GuiGraphics.renderItem`),
//! dropped items (`ItemEntityRenderer`) and items posed in the world (hands,
//! frames, armour stands). This binds the resolver's host services
//! (`ItemEnv`) and the special renderers' registries to the game.
use super::{Dim, entity_culling::EntityCullDecision};
#[cfg(feature = "client")]
use crate::game::client_side::item_model_context;
use serde_json::Value;
use std::{cell::RefCell, collections::HashMap};
use sunrise_assets::item_model::{
    Components, ComponentsView, DisplayContext, FluidAppearance, GlobalPos, ItemEnv,
    LayerTransform, LevelState, ModelChoice, OwnerState, RegisteredItemModel,
    RegisteredRangeProperty, RegisteredSelectKey, RegisteredSelectProperty, RegisteredTintSource,
    StackView,
};
use sunrise_client::{
    block_entity_render::{diffuse_lights, vertex_light},
    camera::joml,
    cit::{Hand as CitHand, Kind as CitKind},
    graphics::EntityCullingMode,
    hud::{HudBatch, Rect},
    item_render::{self, ItemRenderState, SpecialHost, WorldSink},
    mesher::Vertex,
    skins::HeadSkinJournal,
};
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_gameplay_common::game::api_values::TryIntoApi;
use sunrise_items::items::ItemStack;

struct ItemEntityDraw<'a> {
    item: &'a sunrise_sim::entities::ItemEntity,
    at: [f64; 3],
    partial: f32,
    entity: bool,
}

/// The game's painter for the HUD kit's icons (`hud_kit::icon`): items by their models.
#[cfg(feature = "client")]
pub(in crate::game) struct IconPainter<'g, 'a> {
    pub(in crate::game) game: &'g crate::game::ClientGame<'a>,
}

#[cfg(feature = "client")]
impl sunrise_client::hud_kit::icon::IconPainter for IconPainter<'_, '_> {
    fn item(&self, batch: &mut HudBatch, id: &str, [x, y, size]: [f32; 3]) -> bool {
        let start = batch.vertices.len();
        self.game
            .draw_item_model(batch, &ItemStack::new(id, 1), x, y, 0);
        let scale = size / sunrise_client::hud_kit::icon::ITEM_SIZE;
        batch.scale_since(start, x, y, scale, scale);
        batch.vertices.len() > start
    }
}

/// The game's answers to what the resolver cannot read from the stack.
#[cfg(feature = "client")]
pub struct Env<'a, 'g> {
    game: &'a crate::game::ClientGame<'g>,
}
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// Before a frame is drawn: drop the model draws prepared for the last one.
    pub fn begin_frame_draws(&self) {
        self.client.prepared_model_draws.borrow_mut().clear();
    }
}

#[cfg(feature = "client")]
impl item_render::ItemDrawEnv for Env<'_, '_> {
    fn special_model_input(
        &self,
        stack: &StackView,
        context: &sunrise_assets::item_model::Context<'_>,
    ) -> Option<item_render::SpecialModelInput<'_>> {
        super::native_special_renderers::draw_input(self.game, stack, context)
    }

    fn render_type(
        &self,
        handle: mod_api::RenderTypeRef,
    ) -> Result<
        sunrise_client_render::mod_pipelines::ResolvedRenderType<'static>,
        mod_api::RenderTypeError,
    > {
        self.game
            .client
            .client_pack
            .render
            .mod_pipelines
            .render_type_slot(handle)
    }

    fn special_model_stack(
        &self,
        stack: &mod_api::ItemStack,
        display: mod_api::ItemDisplayContext,
    ) -> Option<(StackView, DisplayContext)> {
        super::native_special_renderers::nested_stack(stack, display)
    }
}

#[cfg(feature = "client")]
fn item_model_context(ctx: &sunrise_assets::item_model::Context<'_>) -> mod_api::ItemModelContext {
    let Ok(display) = item_model_context::display_context(&ctx.display);
    mod_api::ItemModelContext::new(
        display,
        ctx.owner
            .and_then(|owner| item_model_context::owner_state(owner).ok()),
        ctx.level
            .and_then(|level| item_model_context::level_state(level).ok()),
        ctx.seed,
    )
}

#[cfg(feature = "client")]
impl ItemEnv for Env<'_, '_> {
    fn item_model_property(&self, property: &str, stack: &StackView) -> bool {
        if !mod_api::__private::listened(mod_api::HookId::ItemModelProperty) {
            return false;
        }
        let Ok(snapshot) = stack.try_into_api() else {
            return false;
        };
        let mut value = false;
        self.game.native_item_model_property(
            property,
            &snapshot,
            stack.bundle_selected,
            &mut value,
        );
        value
    }
    fn item_model_range_property(
        &self,
        property: &RegisteredRangeProperty,
        stack: &StackView,
        ctx: &sunrise_assets::item_model::Context<'_>,
    ) -> f32 {
        let Some(bound) = property.0.downcast_ref::<item_render::BoundRangeProperty>() else {
            return 0.0;
        };
        if !bound.eval.owner().is_enabled() {
            return 0.0;
        }
        let Ok(snapshot) = stack.try_into_api() else {
            return 0.0;
        };
        let context = item_model_context(ctx);
        let input =
            mod_api::ItemModelPropertyInput::new(&snapshot, stack.bundle_selected, &context);
        bound
            .eval
            .owner()
            .call(
                "item model range property",
                "item_model_range_property",
                || (bound.eval.value())(self.game, &input),
            )
            .unwrap_or(0.0)
    }
    fn item_model_select_property(
        &self,
        property: &RegisteredSelectProperty,
        stack: &StackView,
        ctx: &sunrise_assets::item_model::Context<'_>,
        keys: &mut dyn Iterator<Item = (usize, &RegisteredSelectKey)>,
    ) -> Option<usize> {
        let bound = property
            .payload
            .downcast_ref::<item_render::BoundSelectProperty>()?;
        if !bound.eval.owner().is_enabled() {
            return None;
        }
        let Ok(snapshot) = stack.try_into_api() else {
            return None;
        };
        let context = item_model_context(ctx);
        let input =
            mod_api::ItemModelPropertyInput::new(&snapshot, stack.bundle_selected, &context);
        bound.select(self.game, &input, keys)
    }
    fn item_tint_source(&self, source: &RegisteredTintSource, stack: &StackView) -> i32 {
        let Some(bound) = source.0.downcast_ref::<item_render::BoundTintSource>() else {
            return -1;
        };
        if !bound.eval.owner().is_enabled() {
            return -1;
        }
        let Ok(snapshot) = stack.try_into_api() else {
            return -1;
        };
        let input = mod_api::ItemTintInput::new(&snapshot, stack.bundle_selected);
        bound
            .eval
            .owner()
            .call("item tint source", "item_tint_source", || {
                (bound.eval.value())(self.game, &input)
            })
            .map_or(-1, |color| color as i32)
    }
    fn item_model_update(
        &self,
        model: &RegisteredItemModel,
        stack: &StackView,
        ctx: &sunrise_assets::item_model::Context<'_>,
        models: &mut Vec<ModelChoice>,
    ) {
        let Some(bound) = model.0.downcast_ref::<item_render::BoundItemModel>() else {
            return;
        };
        if !bound.eval.owner().is_enabled() {
            return;
        }
        let Ok(snapshot) = stack.try_into_api() else {
            return;
        };
        let context = item_model_context(ctx);
        let input = mod_api::ItemModelUpdateInput::new(&snapshot, stack.bundle_selected, &context);
        let mut layers = Vec::new();
        if bound
            .eval
            .owner()
            .call("item model", "item_model_update", || {
                (bound.eval.value())(self.game, &input, &mut layers);
            })
            .is_err()
        {
            return;
        }
        models.extend(layers.into_iter().map(|layer| ModelChoice {
            model: layer.model,
            transform: match layer.item_transform {
                mod_api::ItemTransformMode::None => LayerTransform::None,
                _ => LayerTransform::Model,
            },
        }));
    }
    fn stack_fluid(&self, stack: &StackView) -> Option<&str> {
        // Vanilla's items keep vanilla's closed set; only a mod's own item asks its parts.
        if stack.item.starts_with("minecraft:") {
            return sunrise_client::fluid_models::held_fluid(&stack.item);
        }
        if !mod_api::__private::item_listens(mod_api::HookId::ContainedFluid, &stack.item) {
            return None;
        }
        let Ok(snapshot) = stack.try_into_api() else {
            return None;
        };
        let mut fluid = None;
        self.game.native_contained_fluid(&snapshot, &mut fluid);
        drawn_contained_fluid(&stack.item, fluid?)
    }
    fn fluid_appearance(&self, fluid: &str) -> Option<FluidAppearance> {
        sunrise_client::fluid_models::item_appearance(fluid)
    }
    fn enchantment_in_tag(&self, enchantment: &str, tag: &str) -> bool {
        self.game.stations().enchantments.in_tag(tag, enchantment)
    }
    fn potion_effect_color(&self, contents: &Value) -> Option<i32> {
        // `PotionContents.getColorOr`: the effect mix when any effect is visible.
        let id = contents["potion"].as_str();
        let effects = id
            .and_then(sunrise_level::brewing::potion)
            .is_some_and(|p| !p.effects.is_empty());
        effects.then(|| (0xFF00_0000 | sunrise_level::brewing::potion_color(id)) as i32)
    }
    fn grass_color(&self, temperature: f64, downfall: f64) -> i32 {
        self.game
            .client
            .client_pack
            .item_renderer
            .grass_color(temperature, downfall)
    }
    fn bundle_fullness(&self, _stack: &StackView, components: &Components) -> f32 {
        // `BundleContents.weight`: count / max stack size, summed.
        let Some(items) = components
            .get("minecraft:bundle_contents")
            .and_then(Value::as_array)
        else {
            return 0.0;
        };
        let prototypes = &self.game.pack.item_models.prototypes;
        let mut total = 0.0f64;
        for item in items {
            let id = item["id"].as_str().unwrap_or("");
            let count = item["count"].as_i64().unwrap_or(1) as f64;
            let max = prototypes
                .get(id)
                .and_then(|c| c.get("minecraft:max_stack_size"))
                .and_then(Value::as_i64)
                .unwrap_or(1) as f64;
            total += count / max;
        }
        total as f32
    }
    fn has_shift_down(&self) -> bool {
        self.game.has_shift_down()
    }
    fn local_time(&self, pattern: &str, _locale: &str, _time_zone: Option<&str>) -> String {
        local_time(pattern)
    }
    fn random_float(&self) -> f32 {
        if self.game.clock.still() {
            // The same draw for a frame of the same world tick.
            let mixed =
                (self.game.visual_ticks() ^ 0x9e37_79b9).wrapping_mul(0x9e37_79b9_7f4a_7c15);
            return (mixed >> 40) as f32 / (1u64 << 24) as f32;
        }
        let nanos = self.game.client.started.elapsed().subsec_nanos();
        (nanos % 1_000_000) as f32 / 1_000_000.0
    }
}
/// The fluid a mod's `contained_fluid` answer `key` draws in a stack of `item`: none (the stack
/// draws as holding none) for a key no fluid is registered under, logged the first time an item
/// answers so. A registered fluid with no look in an item is drawn, as the missing sprite.
fn drawn_contained_fluid(item: &str, key: mod_api::FluidKey) -> Option<&'static str> {
    let fluid = mod_api::__private::fluid_id(key);
    if fluid.is_none() && first_unregistered_answer(item) {
        log::warn!(
            "contained_fluid for {item} answered a fluid key no fluid is registered under: drawn as holding none"
        );
    }
    fluid
}
/// Whether `item` answers an unregistered fluid key for the first time: only that path reads the
/// list, so a stack that draws pays nothing for it.
fn first_unregistered_answer(item: &str) -> bool {
    static REPORTED: std::sync::Mutex<Vec<Box<str>>> = std::sync::Mutex::new(Vec::new());
    let mut reported = REPORTED.lock().unwrap_or_else(|poison| poison.into_inner());
    if reported.iter().any(|seen| &**seen == item) {
        return false;
    }
    reported.push(item.into());
    true
}
/// The days since 1970-01-01 as a civil (year, month, day) date.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}
/// `DateTimeFormatter.ofPattern(pattern).format(now)` for the month, day,
/// year and time letters item models use, in the system time zone
/// (`LocalTime` without a `time_zone`).
pub(in crate::game::client_side) fn local_time(pattern: &str) -> String {
    let now = chrono::Local::now();
    let secs = now.timestamp() + i64::from(now.offset().local_minus_utc());
    let (y, mo, d) = civil(secs.div_euclid(86_400));
    let s = secs.rem_euclid(86_400);
    let (h, mi, se) = (s / 3600, s / 60 % 60, s % 60);
    let mut out = String::new();
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let mut n = 1;
        while i + n < chars.len() && chars[i + n] == c {
            n += 1;
        }
        let num = |v: i64| format!("{v:0n$}");
        match c {
            'M' => out += &num(i64::from(mo)),
            'd' => out += &num(i64::from(d)),
            'y' | 'u' => out += &if n == 2 { num(y % 100) } else { num(y) },
            'H' => out += &num(h),
            'm' => out += &num(mi),
            's' => out += &num(se),
            '\'' => {
                // Quoted literal text.
                let end = chars[i + 1..].iter().position(|&x| x == '\'');
                if let Some(e) = end {
                    out.extend(&chars[i + 1..i + 1 + e]);
                    i += e + 2;
                    continue;
                }
                n = 1;
            }
            other => out.extend(std::iter::repeat_n(other, n)),
        }
        i += n;
    }
    out
}

/// Banner and shield pattern textures from the banner pattern registry.
pub struct Host<'a> {
    banners: &'a sunrise_sim::banner::Banners,
    sprites: &'a HashMap<String, Rect>,
    skins: &'a RefCell<sunrise_client::skins::Downloader>,
    profiles: &'a RefCell<sunrise_client::skins::Resolver>,
    head_skins: Option<&'a HeadSkinJournal>,
}
impl SpecialHost for Host<'_> {
    fn banner_texture(&self, pattern: &str) -> Option<String> {
        self.banners
            .pattern(pattern)
            .map(|_| self.banners.banner_texture(pattern))
    }
    fn shield_texture(&self, pattern: &str) -> Option<String> {
        self.banners
            .pattern(pattern)
            .map(|_| self.banners.shield_texture(pattern))
    }
    fn player_skin(&self, profile: &Value) -> Option<String> {
        // SkullBlockRenderer.getPlayerSkin: SkinManager.getInsecureSkinInformation
        // accepts a texture property without a signature, unlike the tab list.
        // A `Dynamic` profile (a name or id only) resolves through the
        // services (`PlayerSkinRenderCache.getOrDefault`).
        Some(sunrise_client::skins::head_skin_with_journal(
            profile,
            &mut self.profiles.borrow_mut(),
            &mut self.skins.borrow_mut(),
            |location| self.sprites.contains_key(location),
            self.head_skins,
        ))
    }
}

/// One item a block entity renderer hands to the item renderer
/// (`ItemStackRenderState.submit`): the stack it resolved, as
/// `ItemModelResolver.updateForTopItem` was asked for it, under `pose`
/// (camera-relative) at `light` (packed lightmap coordinates).
/// `stack`, `display` and `seed` are what the parity test compares with vanilla's renderers;
/// drawing reads only the resolved `state`.
pub struct BlockItemDraw<'a> {
    #[cfg(any(test, feature = "test-support"))]
    pub stack: &'a ItemStack,
    #[cfg(any(test, feature = "test-support"))]
    pub display: DisplayContext,
    #[cfg(any(test, feature = "test-support"))]
    pub seed: i32,
    pub(in crate::game) state: &'a ItemRenderState,
    pub pose: glam::Mat4,
    pub light: u32,
}

/// The pose `LevelRenderer` gives a block entity's renderer: a `PoseStack`
/// translated to the block from the camera (`translate(double...)`, narrowed).
pub(in crate::game) fn block_entity_pose(key: (i32, i32, i32), camera: glam::DVec3) -> joml::Mat4 {
    let mut pose = joml::Mat4::IDENTITY;
    pose.translate(
        (f64::from(key.0) - camera.x) as f32,
        (f64::from(key.1) - camera.y) as f32,
        (f64::from(key.2) - camera.z) as f32,
    );
    pose
}

/// `ItemOwner` of an entity holding or showing an item.
pub(in crate::game) fn owner_at(position: [f64; 3], yaw: f32, entity_type: &str) -> OwnerState {
    OwnerState {
        position,
        visual_rotation_y_degrees: yaw,
        entity_type: entity_type.to_owned(),
        ..Default::default()
    }
}
/// `UseDuration` of a stack (`Item.getUseDuration`): 72000 for drawn items
/// and for `blocks_attacks` items (shields), the consume time for food.
fn use_duration(components: ComponentsView<'_>) -> i32 {
    if let Some(c) = components.get("minecraft:consumable") {
        let seconds = c["consume_seconds"].as_f64().unwrap_or(1.6);
        return (seconds * 20.0) as i32;
    }
    if components.get("minecraft:blocks_attacks").is_some()
        || components.get("minecraft:kinetic_weapon").is_some()
    {
        return 72_000;
    }
    let item_model = components
        .get("minecraft:item_model")
        .and_then(Value::as_str)
        .unwrap_or("");
    match item_model {
        "minecraft:bow"
        | "minecraft:crossbow"
        | "minecraft:trident"
        | "minecraft:spyglass"
        | "minecraft:goat_horn" => 72_000,
        // BrushItem.USE_DURATION.
        "minecraft:brush" => 200,
        _ => 0,
    }
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    pub fn item_env(&self) -> Env<'_, '_> {
        Env { game: self }
    }
    pub(in crate::game::client_side) fn item_host(&self) -> Host<'_> {
        Host {
            banners: &self.stations().banners,
            sprites: &self.client.gui.atlas.sprites,
            skins: &self.client.skin_downloads.0,
            profiles: &self.client.skin_profiles,
            head_skins: None,
        }
    }
    /// The client level as item models read it: worked out once a frame (a
    /// screen of items read the same level, its sky sampled once).
    pub(in crate::game::client_side) fn item_level(&self) -> LevelState {
        let dimension = self.common.dim.id();
        let key = (
            self.client.frames,
            self.common.world.game_time,
            self.common.world.day_time,
            dimension,
        );
        let mut cached = self.client.item_level_cache.borrow_mut();
        if let Some((at, level)) = cached.as_ref()
            && *at == key
        {
            return level.clone();
        }
        let level = self.level_state_now(dimension);
        *cached = Some((key, level.clone()));
        level
    }
    pub fn level_state_now(&self, dimension: &'static str) -> LevelState {
        let sky = self
            .dim_environment()
            .sample(&sunrise_level::environment::Sample {
                day_time: self.common.world.day_time,
                ..Default::default()
            });
        LevelState {
            dimension: dimension.to_owned(),
            game_time: self.common.world.game_time,
            sun_angle: sky.sun_angle,
            moon_phase: (self.common.world.day_time.div_euclid(24_000)).rem_euclid(8) as i32,
            respawn: self.common.spawn.map(|s| GlobalPos {
                dimension: self
                    .common
                    .level
                    .world_spawn_dimension
                    .clone()
                    .unwrap_or_else(|| "minecraft:overworld".to_owned()),
                pos: [s.x.floor() as i32, s.y.floor() as i32, s.z.floor() as i32],
            }),
            runs_normally: true,
        }
    }
    /// The local player as `ItemOwner`; `stack` is the stack drawn, to tell
    /// whether it is the one in use.
    pub(in crate::game::client_side) fn player_owner(
        &self,
        stack: Option<&ItemStack>,
    ) -> OwnerState {
        let position = self
            .common
            .player
            .as_ref()
            .map_or([0.0; 3], |p| [p.position.x, p.position.y, p.position.z]);
        let mut owner = owner_at(position, self.common.yaw, "minecraft:player");
        owner.main_arm_left = self.client.settings.main_hand_left;
        owner.is_local_player = true;
        owner.is_view_entity = true;
        owner.last_death_location = self
            .common
            .level
            .last_death
            .clone()
            .map(|(dimension, pos)| GlobalPos { dimension, pos });
        if let (Some((slot, ticks)), Some(stack)) = (self.common.using, stack)
            && self.common.stack_in(slot) == Some(stack)
        {
            let view = sunrise_sim::item_models::stack_view(stack);
            let mut duration = use_duration(self.common.pack.item_models.components_view(&view));
            self.native_use_duration(stack, &mut duration);
            owner.using_this_item = true;
            owner.use_duration = duration;
            // getUseItemRemainingTicks: the duration the use started with,
            // as LivingEntityUseItemEvent.Start and .Tick left it (`game/items/item_use.rs`).
            owner.use_item_remaining_ticks = self.common.use_duration - ticks as i32;
            owner.crossbow_charge_duration =
                sunrise_gameplay_common::game::CommonGame::charge_ticks(stack).max(1) as i32;
        }
        owner.fishing_cast_with_this_item = stack
            .zip(self.cast_rod())
            .is_some_and(|(s, rod)| std::ptr::eq(s, rod));
        owner.is_selected = stack.is_some()
            && self
                .common
                .inventory
                .get(self.common.selected)
                .and_then(Option::as_ref)
                == stack;
        owner.is_carried = owner.is_selected;
        owner
    }
    /// `ItemModelResolver.updateForTopItem`.
    pub fn resolve_item(
        &self,
        stack: &ItemStack,
        display: DisplayContext,
        owner: Option<&OwnerState>,
        seed: i32,
    ) -> ItemRenderState {
        self.resolve_item_in_hand(stack, display, owner, seed, CitHand::Main)
    }
    /// `ItemStack.hasFoil`, with the item's default glint.
    fn has_foil(&self, stack: &ItemStack) -> bool {
        let item_glint = self
            .common
            .res
            .items()
            .enchantments
            .props
            .get(&*stack.item)
            .and_then(|p| p.glint);
        stack.has_foil(item_glint)
    }
    /// [`Self::resolve_item`] of a stack held in `hand`, which OptiFine CIT rules can test
    /// (GUI items count as the main hand).
    pub(in crate::game::client_side) fn resolve_item_in_hand(
        &self,
        stack: &ItemStack,
        display: DisplayContext,
        owner: Option<&OwnerState>,
        seed: i32,
        hand: CitHand,
    ) -> ItemRenderState {
        self.resolve_item_in_level(stack, display, owner, Some(&self.item_level()), seed, hand)
    }
    /// Tint only the resolved draw, never the model shared by other holders or contexts.
    pub(in crate::game::client_side) fn tint_held_item(
        &self,
        state: &mut ItemRenderState,
        stack: &ItemStack,
        entity_id: u32,
        hand: mod_api::Hand,
        partial_tick: f32,
    ) {
        // Resolving an empty stack produces no layers.
        if state.is_empty()
            || !mod_api::__private::item_listens(mod_api::HookId::HeldItemTint, &stack.item)
        {
            return;
        }
        let mut tint = None;
        self.native_held_item_tint(entity_id, hand, stack, partial_tick, &mut tint);
        if let Some(rgb) = tint {
            state.set_held_tint(rgb);
        }
    }
    /// [`Self::resolve_item_in_hand`] with the level the model reads given: `None` for a
    /// resolution that has no level, whose `context_dimension` selects take their fallback.
    pub(in crate::game) fn resolve_item_in_level(
        &self,
        stack: &ItemStack,
        display: DisplayContext,
        owner: Option<&OwnerState>,
        level: Option<&LevelState>,
        seed: i32,
        hand: CitHand,
    ) -> ItemRenderState {
        let renderer = &self.client.client_pack.item_renderer;
        let (rule, glint) = renderer.cit.as_ref().map_or((None, None), |cit| {
            let item_max = self
                .common
                .res
                .items()
                .get(&stack.item)
                .map_or(0, |i| i.max_damage);
            let max_damage = item_render::cit::stack_max_damage(stack, item_max);
            (
                cit.rules.select(CitKind::Item, stack, hand, max_damage),
                cit.rules
                    .glint(stack, hand, max_damage, self.has_foil(stack)),
            )
        });
        let mut state = renderer.resolve_with_rule(
            &sunrise_sim::item_models::stack_view(stack),
            display,
            owner,
            level,
            seed,
            &self.item_env(),
            rule,
        );
        if let Some(glint) = glint {
            // The pack's enchantment rules replace vanilla's glint unless one keeps it, so far
            // only in the GUI, where their own layers draw: held, dropped and framed items keep
            // vanilla's until theirs do. The pack's `useGlint=false` removes it everywhere.
            let gui = display == DisplayContext::Gui;
            let pack_keeps = renderer
                .cit
                .as_ref()
                .is_none_or(|c| c.rules.global.use_glint);
            if !glint.vanilla && (gui || !pack_keeps) {
                for layer in &mut state.layers {
                    layer.foil = sunrise_assets::item_model::FoilType::None;
                }
            }
            if gui && !glint.is_empty() {
                state.cit_glint = Some(glint);
            }
        }
        state.hdr_tier = self.luminance().hdr_tier(&stack.item);
        if state.hdr_tier > 0 && self.client.renderer.0.as_ref().is_some_and(|r| r.hdr_on()) {
            state.hdr_light = self.luminance().of(&stack.item);
        }
        state
    }
    /// `GuiGraphics.renderItem(player, level, stack, x, y, seed)`.
    pub fn draw_item_model(
        &self,
        batch: &mut HudBatch,
        stack: &ItemStack,
        x: f32,
        y: f32,
        seed: i32,
    ) {
        self.draw_item_model_with_skins(batch, stack, [x, y], seed, None);
    }

    /// The seeded GUI item draw, with optional deferral for a transactional canvas.
    pub(in crate::game) fn draw_item_model_with_skins(
        &self,
        batch: &mut HudBatch,
        stack: &ItemStack,
        [x, y]: [f32; 2],
        seed: i32,
        head_skins: Option<&HeadSkinJournal>,
    ) {
        let owner = self.player_owner(Some(stack));
        self.draw_item_model_with_owner_and_skins(
            batch,
            stack,
            [x, y],
            seed,
            Some(&owner),
            head_skins,
        );
    }
    /// `GuiGraphicsExtractor.item`: `fakeItem` uses the same model draw with no owner.
    pub(in crate::game::client_side) fn draw_item_model_with_owner(
        &self,
        batch: &mut HudBatch,
        stack: &ItemStack,
        x: f32,
        y: f32,
        owner: Option<&OwnerState>,
    ) {
        self.draw_item_model_with_owner_and_skins(batch, stack, [x, y], 0, owner, None);
    }
    fn draw_item_model_with_owner_and_skins(
        &self,
        batch: &mut HudBatch,
        stack: &ItemStack,
        [x, y]: [f32; 2],
        seed: i32,
        owner: Option<&OwnerState>,
        head_skins: Option<&HeadSkinJournal>,
    ) {
        let state = self.resolve_item(stack, DisplayContext::Gui, owner, seed);
        let host = Host {
            head_skins,
            ..self.item_host()
        };
        item_render::gui::draw_with_frame(
            &state,
            &self.client.client_pack.item_renderer,
            &self.client.gui.atlas.sprites,
            &host,
            batch,
            x,
            y,
            self.client.glint_frame,
        );
    }
    /// An item under `pose` (GUI pixels), cut by no slot and with a depth of its own: what
    /// `DrawContext::model_3d` draws a turned item with.
    pub(in crate::game::client_side) fn draw_item_model_posed(
        &self,
        batch: &mut HudBatch,
        stack: &ItemStack,
        pose: glam::Mat4,
    ) -> bool {
        let owner = self.player_owner(Some(stack));
        let state = self.resolve_item(stack, DisplayContext::Gui, Some(&owner), 0);
        let host = self.item_host();
        item_render::gui::draw_posed(
            &state,
            &item_render::gui::GuiItemEnv {
                renderer: &self.client.client_pack.item_renderer,
                sprites: &self.client.gui.atlas.sprites,
                host: &host,
            },
            batch,
            pose,
            self.client.glint_frame,
        )
    }
    /// The glint clock, options and sprite for this frame: read once per
    /// frame, not per item (`GameRenderer.extractOptions`).
    pub(in crate::game::client_side) fn capture_glint_frame(&self) -> item_render::GlintFrame {
        item_render::GlintFrame {
            millis: self.visual_millis(),
            speed: self.client.settings.file.double("glintSpeed"),
            alpha: self.client.settings.file.double("glintStrength") as f32,
            sprite: self
                .client
                .gui
                .atlas
                .sprites
                .contains_key("misc/enchanted_glint_item"),
        }
    }
    /// The `GLINT_TEXTURING` inputs of this frame (`WorldSink` picks `ENTITY_SCALE` per quad).
    pub(in crate::game::client_side) fn glint(&self) -> Option<item_render::Glint> {
        self.client
            .glint_frame
            .glint(item_render::Glint::ITEM_SCALE)
    }
    /// Submit a resolved item under `pose` (camera-relative) at `light`
    /// (packed lightmap coordinates).
    pub(in crate::game) fn submit_item_world(
        &self,
        state: &ItemRenderState,
        pose: glam::Mat4,
        light: u32,
        out: &mut Vec<Vertex>,
    ) {
        let lights = diffuse_lights(self.nether_lighting());
        self.submit_item_lit(state, pose, light, lights, out);
    }
    /// [`Self::submit_item_world`] preserving JOML `Matrix4f` rounding through baked quads.
    pub(in crate::game::client_side) fn submit_item_world_joml(
        &self,
        state: &ItemRenderState,
        pose: sunrise_client::camera::joml::Mat4,
        light: u32,
        out: &mut Vec<Vertex>,
    ) {
        let lights = diffuse_lights(self.nether_lighting());
        let mut sink = WorldSink {
            out,
            light: item_hdr_light(state, light),
            lights: (state.hdr_light == 0).then_some(lights),
            glint: self.glint(),
            tier: state.hdr_tier,
        };
        state.submit_joml(
            &self.client.client_pack.item_renderer,
            &self.client.gui.atlas.sprites,
            &self.item_host(),
            pose,
            &mut sink,
        );
    }
    /// [`Self::submit_item_world`] shaded by other diffuse lights (a
    /// `Lighting.Entry` other than the level's), in the pose's output space.
    pub(in crate::game::client_side) fn submit_item_lit(
        &self,
        state: &ItemRenderState,
        pose: glam::Mat4,
        light: u32,
        lights: (glam::Vec3, glam::Vec3),
        out: &mut Vec<Vertex>,
    ) {
        let mut sink = WorldSink {
            out,
            light: item_hdr_light(state, light),
            lights: (state.hdr_light == 0).then_some(lights),
            glint: self.glint(),
            tier: state.hdr_tier,
        };
        state.submit(
            &self.client.client_pack.item_renderer,
            &self.client.gui.atlas.sprites,
            &self.item_host(),
            pose,
            &mut sink,
        );
    }
    /// [`Self::submit_item_world`] without the level's diffuse shading.
    pub fn submit_item_unlit(
        &self,
        state: &ItemRenderState,
        pose: glam::Mat4,
        light: u32,
        out: &mut Vec<Vertex>,
    ) {
        let mut sink = WorldSink {
            out,
            light: item_hdr_light(state, light),
            lights: None,
            glint: self.glint(),
            tier: state.hdr_tier,
        };
        state.submit(
            &self.client.client_pack.item_renderer,
            &self.client.gui.atlas.sprites,
            &self.item_host(),
            pose,
            &mut sink,
        );
    }
    /// [`Self::submit_item_lit`] of `state` under each of `poses` on the GPU path: the mark
    /// before its draws and their bounds, or `None` (nothing drawn) when the state cannot
    /// take it.
    pub(in crate::game::client_side) fn submit_item_skinned(
        &self,
        state: &ItemRenderState,
        poses: &[glam::Mat4],
        light: u32,
        lights: (glam::Vec3, glam::Vec3),
    ) -> Option<((usize, usize, u32), Option<(glam::Vec3, glam::Vec3)>)> {
        let mut models = self.client.skin_models.0.borrow_mut();
        let mut frame = self.client.skin_frame.borrow_mut();
        let mark = frame.mark();
        let mut skin = sunrise_client::mobs::skin::Skin::new(&mut models, &mut frame);
        for &pose in poses {
            if !self.skin_item(state, pose, light, lights, &mut skin) {
                // Decided before any draw, the same for every pose.
                return None;
            }
        }
        Some((mark, skin.bounds))
    }
    /// One pose of [`Self::submit_item_skinned`] into an entity's open
    /// `skin` (its bounds grow by the item's): false, with nothing drawn,
    /// when the item cannot take the GPU path.
    pub(in crate::game::client_side) fn skin_item(
        &self,
        state: &ItemRenderState,
        pose: glam::Mat4,
        light: u32,
        lights: (glam::Vec3, glam::Vec3),
        skin: &mut sunrise_client::mobs::skin::Skin,
    ) -> bool {
        let sink = (
            item_hdr_light(state, light),
            (state.hdr_light == 0).then_some(lights),
            state.hdr_tier,
        );
        let renderer = &self.client.client_pack.item_renderer;
        state.submit_skinned(renderer, &self.client.gui.atlas.sprites, pose, sink, skin)
    }
    /// [`Self::submit_item_world`] of `state` under `pose` on the GPU path, its draws counted
    /// with an entity's (`skinned`: its mark and bounds, `cull_skinned`'s); false, with
    /// nothing drawn, when the state cannot take it.
    pub(in crate::game::client_side) fn skin_item_into(
        &self,
        state: &ItemRenderState,
        pose: glam::Mat4,
        light: u32,
        skinned: &mut super::creatures::Skinned,
    ) -> bool {
        let lights = diffuse_lights(self.nether_lighting());
        let Some((mark, bounds)) = self.submit_item_skinned(state, &[pose], light, lights) else {
            return false;
        };
        *skinned = Some(match skinned.take() {
            Some((first, before)) => (
                first,
                match (before, bounds) {
                    (Some((a, b)), Some((c, d))) => Some((a.min(c), b.max(d))),
                    (one, other) => one.or(other),
                },
            ),
            None => (mark, bounds),
        });
        true
    }
    /// `ItemEntityRenderer.submit` for every dropped item.
    pub fn dropped_item_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
        run: std::ops::Range<usize>,
    ) {
        let live = self.common.dropped[run]
            .iter()
            .filter(|item| !self.hides_as_camera(item.id))
            .map(|item| {
                let (p, q) = (item.previous, item.position);
                let at = [
                    p.x + (q.x - p.x) * alpha,
                    p.y + (q.y - p.y) * alpha,
                    p.z + (q.z - p.z) * alpha,
                ];
                ItemEntityDraw {
                    item,
                    at,
                    partial: alpha as f32,
                    entity: true,
                }
            });
        self.item_entity_geometry(camera, live, out);
    }

    pub(in crate::game::client_side) fn pickup_item_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
    ) {
        // ItemPickupParticle: the render state extracted at partial tick 1.
        let taken = self
            .client
            .pickups
            .0
            .live
            .iter()
            .filter_map(|p| match &p.taken {
                super::pickups::Taken::Item(item) => Some(ItemEntityDraw {
                    item,
                    at: super::pickups::position(p, alpha),
                    partial: 1.0,
                    entity: false,
                }),
                _ => None,
            });
        self.item_entity_geometry(camera, taken, out);
    }

    fn item_entity_geometry<'a>(
        &self,
        camera: glam::DVec3,
        items: impl Iterator<Item = ItemEntityDraw<'a>>,
        out: &mut Vec<Vertex>,
    ) {
        // Entity.shouldRenderAtSqrDistance for the item's 0.25 box: 16 blocks
        // times the view scale (32 at render distance 16), from its current
        // position. A pickup particle is not an entity and keeps 64 blocks.
        let range =
            super::entity_culling::default_range(glam::DVec3::splat(0.25), self.view_scale());
        let skinning = self.skinning();
        let lights = diffuse_lights(self.common.dim == Dim::Nether);
        for ItemEntityDraw {
            item,
            at,
            partial,
            entity,
        } in items
        {
            let entity_start = out.len();
            self.mark_motion(entity_start, (2 << 32) | u64::from(item.id));
            let q = item.position;
            let pos = glam::DVec3::from_array(at) - camera;
            let far = if entity {
                (glam::DVec3::new(q.x, q.y, q.z) - camera).length_squared() >= range * range
            } else {
                pos.length_squared() > 64.0 * 64.0
            };
            if far {
                continue;
            }
            // EntityRenderer.shouldRender: the item's 0.25 box, grown by 0.5,
            // in the view frustum. Checked before its model is resolved.
            if entity && let Some(view) = &self.client.view_frustum {
                let (lo, hi) = dropped_item_cull_box(glam::DVec3::from_array(at), camera);
                if !view.intersects(lo, hi) {
                    continue;
                }
            }
            // `extractItemGroupRenderState`: GROUND, seeded by the entity id.
            let owner = owner_at([q.x, q.y, q.z], 0.0, "minecraft:item");
            let state = self.resolve_item(
                &item.stack,
                DisplayContext::Ground,
                Some(&owner),
                item.id as i32,
            );
            if state.is_empty() {
                continue;
            }
            let age = item.age as f32 + partial;
            let (lo, hi) = state.bounding_box();
            let min_offset_y = -lo.y + 0.0625;
            let motion = self.dropped_stack_motion(&item.stack);
            let bob = item_render::item_bob(age, item.bob_offset, motion.bob);
            let spin = item_render::item_spin(age, item.bob_offset);
            let pose = glam::Mat4::from_translation(pos.as_vec3())
                * glam::Mat4::from_translation(glam::Vec3::new(0.0, bob + min_offset_y, 0.0))
                * glam::Mat4::from_rotation_y(spin);
            // `EntityRenderer.getPackedLightCoords`: an item entity is lit from
            // the block its light probe (its eye, 0.2125 up) is in, at 15 block
            // light while it burns. A pickup particle is not an entity and keeps
            // the cell it is drawn in.
            let packed = if entity {
                // The shared extraction already interpolated the drawn position in f64.
                let probe = [
                    at[0],
                    at[1] + sunrise_sim::entities::ItemEntity::EYE_HEIGHT,
                    at[2],
                ];
                item.render_light(self.common.world.light(
                    probe[0].floor() as i32,
                    probe[1].floor() as i32,
                    probe[2].floor() as i32,
                ))
            } else {
                vertex_light(self.common.world.light(
                    q.x.floor() as i32,
                    (q.y + 0.1).floor() as i32,
                    q.z.floor() as i32,
                ))
            };
            let seed = i64::from(self.common.item_number(&item.stack.item).unwrap_or(0))
                + i64::from(item.stack.damage);
            let copies = item_render::dropped_copy_poses(
                pose,
                item_render::rendered_amount(item.stack.count),
                seed,
                hi.z - lo.z,
                motion.spread,
            );
            // Use the same animated, spun, clustered transforms as the draw.
            // ItemEntity's collision box is only 0.25 blocks and cannot prove
            // that this larger rendered model stays behind an occluder.
            let culling = entity.then(|| {
                // The posed envelope walks every model point in every copy;
                // with culling off the decision is `Bypass` whatever it holds.
                if self.client.settings.sunrise.entity_culling == EntityCullingMode::Off {
                    return EntityCullDecision::Bypass;
                }
                state
                    .posed_bounds(&copies)
                    .map(|(lo, hi)| {
                        self.entity_cull_decision_in_bounds(
                            item,
                            Aabb::new(
                                Vec3::new(
                                    camera.x + f64::from(lo.x),
                                    camera.y + f64::from(lo.y),
                                    camera.z + f64::from(lo.z),
                                ),
                                Vec3::new(
                                    camera.x + f64::from(hi.x),
                                    camera.y + f64::from(hi.y),
                                    camera.z + f64::from(hi.z),
                                ),
                            ),
                        )
                    })
                    .unwrap_or_else(|| self.entity_cull_decision(item, f64::from(partial)))
            });
            if culling.is_some_and(|decision| decision.hides_entity()) {
                continue;
            }
            let outline = self.thing_outline(item.id, "minecraft:item");
            // Posed on the GPU when every layer can be, its silhouette too when it glows.
            let skinned = skinning
                .then(|| self.submit_item_skinned(&state, &copies, packed, lights))
                .flatten();
            if skinned.is_none() {
                for &copy in &copies {
                    self.submit_item_world(&state, copy, packed, out);
                }
            }
            if let Some(rgb) = outline {
                self.mark_outline(&out[entity_start..], rgb);
                if let Some(((first, _, _), _)) = skinned {
                    let color = sunrise_client::mobs::render::abgr(rgb);
                    self.client
                        .skin_frame
                        .borrow_mut()
                        .outline_since(first, color);
                }
            } else if let Some(culling) = culling {
                self.finish_entity_occlusion(culling, item, out, entity_start);
            } else {
                self.cull_skinned(out, entity_start, skinned);
            }
        }
    }
}

/// `EntityRenderer.shouldRender`'s box for a dropped item, in camera space:
/// `EntityRenderer.getBoundingBoxForCulling(entity, partialTick)` is
/// `Entity.getInterpolatedBoundingBox(partialTick)` - the item's 0.25 box at
/// the position the frame draws, `ItemEntity` overriding neither - grown by 0.5
/// all round before the frustum test. `position` is the already-extracted f64
/// frame position, shared with drawing and lighting.
fn dropped_item_cull_box(position: glam::DVec3, camera: glam::DVec3) -> (glam::Vec3, glam::Vec3) {
    let feet = (position - camera).as_vec3();
    let half = glam::Vec3::new(0.125 + 0.5, 0.5, 0.125 + 0.5);
    (
        feet - half,
        feet + glam::Vec3::new(half.x, 0.25 + 0.5, half.z),
    )
}

/// The packed light of an item (sky low, block above it: `vertex_light`, entity.wgsl), its block
/// light raised to what it glows with in HDR output.
fn item_hdr_light(state: &ItemRenderState, light: u32) -> u32 {
    let block = ((light >> 8) & 0xff).max(u32::from(state.hdr_light) * 16);
    (light & !0xff00) | block << 8
}

use sunrise_gameplay_common::game::end_common::EndGameExt as _;

use sunrise_gameplay_common::game::native_hooks::base::GameHooks as _;

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

#[cfg(feature = "client")]
#[cfg(test)]
pub(in crate::game) mod tests {
    use super::*;
    use mod_api::ClientHost;
    use serde_json::json;

    /// The resolver skips a mod that leaves `item_model_property` at its default;
    /// a mod that implements it is asked on every resolve.
    #[test]
    #[ignore = "daily"]
    fn item_model_property_is_asked_only_while_a_mod_answers() {
        use mod_api::{self, HookId, Hooks, ItemModelPropertyEvent, ModEntry};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        struct Quiet;
        #[mod_api::__subscriptions]
        impl Hooks for Quiet {}
        struct Charged(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl Hooks for Charged {
            fn item_model_property(
                &self,
                _: &dyn ClientHost,
                event: &mut ItemModelPropertyEvent<'_>,
            ) {
                self.0.fetch_add(1, Ordering::Relaxed);
                event.value = event.property == "example:charged";
            }
        }
        let stack = StackView::new("minecraft:stick", 1);

        let mods: mod_api::Mods = vec![ModEntry::new("quiet", Box::new(Quiet))].into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_client_game(|game| {
                let env = Env { game };
                assert!(!mod_api::__private::listened(HookId::ItemModelProperty));
                assert!(!env.item_model_property("example:charged", &stack));
                assert!(!mod_api::__private::listened(HookId::ItemModelProperty));
                assert!(!env.item_model_property("example:charged", &stack));
            });
        });

        let calls = Arc::new(AtomicUsize::new(0));
        let mods: mod_api::Mods =
            vec![ModEntry::new("charged", Box::new(Charged(calls.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_client_game(|game| {
                let env = Env { game };
                for _ in 0..3 {
                    assert!(env.item_model_property("example:charged", &stack));
                }
                assert!(mod_api::__private::listened(HookId::ItemModelProperty));
            });
        });
        assert_eq!(calls.load(Ordering::Relaxed), 3);
    }

    struct Fill;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Fill {
        fn register_range_select_item_model_property(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut mod_api::RegisterRangeSelectItemModelPropertyEvent<'_>,
        ) {
            event.register(mod_api::Identifier::parse("example:fill").unwrap(), |_| {
                Ok(
                    |_: &dyn ClientHost, input: &mod_api::ItemModelPropertyInput<'_>| {
                        input.stack.count() as f32 / 64.0
                    },
                )
            });
        }
    }

    /// A mod registers its numeric property once at start.
    #[test]
    fn a_mod_registers_its_range_select_item_model_property() {
        use mod_api::__private::game::*;
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("fill", Box::new(Fill))].into();
        mod_platform::testing::with_mods(mods, || {
            let mut properties = Vec::new();
            sunrise_gameplay_common::game::native_hooks::native_register_range_select_item_model_property(
                &mod_api::RegistrationContext::new(),
                &mut properties,
            );
            let (_, kept) = item_render::bind_range_property_factories(properties);
            assert_eq!(kept, ["example:fill"]);
        });
    }

    /// The resolver reads a mod's number for its property, 0 for another's or with no mod.
    #[test]
    #[ignore = "daily"]
    fn item_model_range_property_reads_the_mods_number() {
        use mod_api::__private::game::*;
        use sunrise_assets::item_model::Context;
        let stack = StackView::new("minecraft:stick", 16);
        let ctx = Context {
            display: DisplayContext::Gui,
            owner: None,
            level: None,
            seed: 0,
        };
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("fill", Box::new(Fill))].into();
        mod_platform::testing::with_mods(mods, || {
            let mut properties = Vec::new();
            sunrise_gameplay_common::game::native_hooks::native_register_range_select_item_model_property(
                &mod_api::RegistrationContext::new(),
                &mut properties,
            );
            let (factories, _) = item_render::bind_range_property_factories(properties);
            let bound = item_render::decode_range_property(
                Some(&factories),
                "example:fill",
                &serde_json::Map::new(),
            )
            .unwrap()
            .expect("registered range property");
            assert!(
                item_render::decode_range_property(
                    Some(&factories),
                    "other:fill",
                    &serde_json::Map::new(),
                )
                .unwrap()
                .is_none()
            );
            super::super::native_entity_refs::tests::with_client_game(|game| {
                let env = Env { game };
                assert_eq!(env.item_model_range_property(&bound, &stack, &ctx), 0.25);
            });
        });
        mod_platform::testing::with_mods(Vec::new().into(), || {
            let mut properties = Vec::new();
            sunrise_gameplay_common::game::native_hooks::native_register_range_select_item_model_property(
                &mod_api::RegistrationContext::new(),
                &mut properties,
            );
            let (factories, _) = item_render::bind_range_property_factories(properties);
            assert!(
                item_render::decode_range_property(
                    Some(&factories),
                    "example:fill",
                    &serde_json::Map::new(),
                )
                .unwrap()
                .is_none()
            );
        });
    }

    struct Mode;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Mode {
        fn register_select_item_model_property(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut mod_api::RegisterSelectItemModelPropertyEvent<'_>,
        ) {
            event.register(mod_api::Identifier::parse("example:mode").unwrap(), |_| {
                Ok((
                    |when: &sunrise_core::nbt::Tag| match when {
                        sunrise_core::nbt::Tag::String(s) => Ok(s.clone()),
                        _ => Err("expected string case".into()),
                    },
                    |_: &dyn mod_api::ClientHost, input: &mod_api::ItemModelPropertyInput<'_>| {
                        Some(
                            if input.stack.count() > 1 {
                                "build"
                            } else {
                                "dig"
                            }
                            .into(),
                        )
                    },
                ))
            });
        }
    }

    /// A mod registers its categorical property once at start.
    #[test]
    fn a_mod_registers_its_select_item_model_property() {
        use mod_api::__private::game::*;
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("mode", Box::new(Mode))].into();
        mod_platform::testing::with_mods(mods, || {
            let mut properties = Vec::new();
            sunrise_gameplay_common::game::native_hooks::native_register_select_item_model_property(
                &mod_api::RegistrationContext::new(),
                &mut properties,
            );
            let (_, kept) = item_render::bind_select_property_factories(properties);
            assert_eq!(kept, ["example:mode"]);
        });
    }

    /// The resolver reads a mod's value for its property, none for another's or with no mod.
    #[test]
    #[ignore = "daily"]
    fn item_model_select_property_reads_the_mods_value() {
        use mod_api::__private::game::*;
        use sunrise_assets::item_model::Context;
        let ctx = Context {
            display: DisplayContext::Gui,
            owner: None,
            level: None,
            seed: 0,
        };
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("mode", Box::new(Mode))].into();
        mod_platform::testing::with_mods(mods, || {
            let mut properties = Vec::new();
            sunrise_gameplay_common::game::native_hooks::native_register_select_item_model_property(
                &mod_api::RegistrationContext::new(),
                &mut properties,
            );
            let (factories, _) = item_render::bind_select_property_factories(properties);
            let bound = item_render::decode_select_property(
                Some(&factories),
                "example:mode",
                &serde_json::Map::new(),
            )
            .unwrap()
            .expect("registered select property");
            assert!(
                item_render::decode_select_property(
                    Some(&factories),
                    "other:mode",
                    &serde_json::Map::new(),
                )
                .unwrap()
                .is_none()
            );
            super::super::native_entity_refs::tests::with_client_game(|game| {
                let env = Env { game };
                let one = StackView::new("minecraft:stick", 1);
                let two = StackView::new("minecraft:stick", 2);
                let dig = bound.decode_case(&serde_json::json!("dig")).unwrap();
                let build = bound.decode_case(&serde_json::json!("build")).unwrap();
                let keys = [(0, &dig), (1, &build)];
                assert_eq!(
                    env.item_model_select_property(&bound, &one, &ctx, &mut keys.into_iter()),
                    Some(0)
                );
                assert_eq!(
                    env.item_model_select_property(&bound, &two, &ctx, &mut keys.into_iter()),
                    Some(1)
                );
            });
        });
        mod_platform::testing::with_mods(Vec::new().into(), || {
            let mut properties = Vec::new();
            sunrise_gameplay_common::game::native_hooks::native_register_select_item_model_property(
                &mod_api::RegistrationContext::new(),
                &mut properties,
            );
            let (factories, _) = item_render::bind_select_property_factories(properties);
            assert!(
                item_render::decode_select_property(
                    Some(&factories),
                    "example:mode",
                    &serde_json::Map::new(),
                )
                .unwrap()
                .is_none()
            );
        });
    }

    /// `EntityRenderer.shouldRender` culls with `getBoundingBoxForCulling(entity,
    /// partialTick)`, which is `Entity.getInterpolatedBoundingBox(partialTick)`:
    /// an item's 0.25 box at the position the frame draws, grown by 0.5. The box
    /// was built from the tick's own position, so an item crossing the frustum
    /// edge mid-tick disappeared for the frames before it got there.
    #[test]
    fn a_dropped_item_is_culled_at_the_frame_s_position() {
        let camera = glam::DVec3::ZERO;
        let previous = glam::DVec3::new(0.0, 64.0, 0.0);
        let position = glam::DVec3::new(1.0, 64.0, 0.0);
        let bounds = |alpha: f64| {
            let (lo, hi) = dropped_item_cull_box(previous.lerp(position, alpha), camera);
            (lo.to_array(), hi.to_array())
        };
        // The frame's start: the box stands at the item's previous position.
        assert_eq!(bounds(0.0), ([-0.625, 63.5, -0.625], [0.625, 64.75, 0.625]));
        // Halfway through the tick the box has travelled half the block.
        assert_eq!(bounds(0.5), ([-0.125, 63.5, -0.625], [1.125, 64.75, 0.625]));
        // And at its end it is where the old code put it on every frame.
        assert_eq!(bounds(1.0), ([0.375, 63.5, -0.625], [1.625, 64.75, 0.625]));
        // A falling item moves in y the same way: halfway down a 66 -> 65 drop
        // the box's floor is at 65.0, where the old code had the tick's 64.5.
        let (lo, hi) = dropped_item_cull_box(
            glam::DVec3::new(0.0, 66.0, 0.0).lerp(glam::DVec3::new(0.0, 65.0, 0.0), 0.5),
            camera,
        );
        assert!((lo.y - 65.0).abs() < 1e-6);
        assert!((hi.y - 66.25).abs() < 1e-6);
    }

    struct Charge;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Charge {
        fn register_item_tint_sources(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut mod_api::RegisterItemTintSourcesEvent<'_>,
        ) {
            event.register(
                mod_api::Identifier::parse("example:charge").unwrap(),
                |config| {
                    let low = match config.get("low") {
                        Some(sunrise_core::nbt::Tag::Long(v)) => *v as u32,
                        _ => 0,
                    };
                    Ok(
                        move |_: &dyn ClientHost, input: &mod_api::ItemTintInput<'_>| match input
                            .stack
                            .count()
                        {
                            1 => low,
                            _ => 0xFF11_2233,
                        },
                    )
                },
            );
        }
    }

    /// A mod's item model type: a cell that draws `gui` in a screen and `ground` elsewhere, `full`
    /// or `empty` by the stack, and reports the seed, the owner's type and the level it was told,
    /// as names the test reads back.
    struct Cell;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Cell {
        fn register_item_models(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut mod_api::RegisterItemModelsEvent<'_>,
        ) {
            event.register(
                mod_api::Identifier::parse("cellmod:cell").unwrap(),
                |_, children| {
                    children.add_named_models()?;
                    Ok(
                        |_: &dyn ClientHost,
                         input: &mod_api::ItemModelUpdateInput<'_>,
                         layers: &mut Vec<mod_api::ItemModelLayer>| {
                            let place = match input.context.display {
                                mod_api::ItemDisplayContext::Gui => "gui",
                                _ => "ground",
                            };
                            let fill = if input.stack.count() > 1 {
                                "full"
                            } else {
                                "empty"
                            };
                            layers.push(mod_api::ItemModelLayer::new(format!("{place}/{fill}")));
                            layers.push(
                                mod_api::ItemModelLayer::new(format!(
                                    "seed {}",
                                    input.context.seed
                                ))
                                .with_item_transform(mod_api::ItemTransformMode::None),
                            );
                            if let Some(owner) = &input.context.owner {
                                layers.push(mod_api::ItemModelLayer::new(format!(
                                    "owner {}",
                                    owner.entity_type
                                )));
                            }
                            if let Some(level) = &input.context.level {
                                layers.push(mod_api::ItemModelLayer::new(format!(
                                    "level {}",
                                    level.dimension
                                )));
                            }
                        },
                    )
                },
            );
        }
    }

    /// A mod registers its item model type once at start, and the game's resolver gives it what
    /// `ItemModel.update` reads: the stack, where the item is drawn, its owner and level when it
    /// has them, and the seed. Another type, or no mod, draws nothing.
    #[test]
    #[ignore = "daily"]
    fn a_mods_item_model_type_answers_with_the_draws_context() {
        use mod_api::__private::game::*;
        use sunrise_assets::item_model::{Context, DisplayContext, LevelState, OwnerState};
        let fields = serde_json::json!({"type": "cellmod:cell"})
            .as_object()
            .cloned()
            .unwrap();
        let owner = OwnerState {
            entity_type: "minecraft:zombie".into(),
            ..OwnerState::default()
        };
        let level = LevelState {
            dimension: "minecraft:the_nether".into(),
            ..LevelState::default()
        };
        let in_screen = Context {
            display: DisplayContext::Gui,
            owner: None,
            level: None,
            seed: 7,
        };
        let on_ground = Context {
            display: DisplayContext::Ground,
            owner: Some(&owner),
            level: Some(&level),
            seed: 42,
        };
        let drawn =
            |env: &Env<'_, '_>, model: &RegisteredItemModel, count: i32, ctx: &Context<'_>| {
                let mut models = Vec::new();
                env.item_model_update(
                    model,
                    &StackView::new("minecraft:stick", count),
                    ctx,
                    &mut models,
                );
                models
                    .into_iter()
                    .map(|choice| (choice.model, choice.transform == LayerTransform::None))
                    .collect::<Vec<_>>()
            };
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("cellmod", Box::new(Cell))].into();
        mod_platform::testing::with_mods(mods, || {
            let mut types = Vec::new();
            sunrise_gameplay_common::game::native_hooks::native_register_item_models(
                &mod_api::RegistrationContext::new(),
                &mut types,
            );
            let (factories, kept) = item_render::bind_item_model_factories(types);
            assert_eq!(kept, ["cellmod:cell"]);
            let bound = item_render::decode_item_model(
                Some(&factories),
                "cellmod:cell",
                &fields,
                &sunrise_assets::item_model::ItemModelNode::decode,
            )
            .unwrap()
            .expect("registered item model")
            .model;
            assert!(
                item_render::decode_item_model(
                    Some(&factories),
                    "othermod:cell",
                    &fields,
                    &sunrise_assets::item_model::ItemModelNode::decode,
                )
                .unwrap()
                .is_none()
            );
            super::super::native_entity_refs::tests::with_client_game(|game| {
                let env = Env { game };
                assert_eq!(
                    drawn(&env, &bound, 1, &in_screen),
                    [("gui/empty".to_owned(), false), ("seed 7".to_owned(), true)]
                );
                assert_eq!(
                    drawn(&env, &bound, 2, &on_ground),
                    [
                        ("ground/full".to_owned(), false),
                        ("seed 42".to_owned(), true),
                        ("owner minecraft:zombie".to_owned(), false),
                        ("level minecraft:the_nether".to_owned(), false)
                    ]
                );
            });
        });
        mod_platform::testing::with_mods(Vec::new().into(), || {
            let mut types = Vec::new();
            sunrise_gameplay_common::game::native_hooks::native_register_item_models(
                &mod_api::RegistrationContext::new(),
                &mut types,
            );
            let (factories, _) = item_render::bind_item_model_factories(types);
            assert!(
                item_render::decode_item_model(
                    Some(&factories),
                    "cellmod:cell",
                    &fields,
                    &sunrise_assets::item_model::ItemModelNode::decode,
                )
                .unwrap()
                .is_none()
            );
        });
    }

    /// A mod registers its item tint source once at start.
    #[test]
    fn a_mod_registers_its_item_tint_source() {
        use mod_api::__private::game::*;
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("charge", Box::new(Charge))].into();
        mod_platform::testing::with_mods(mods, || {
            let mut sources = Vec::new();
            sunrise_gameplay_common::game::native_hooks::native_register_item_tint_sources(
                &mod_api::RegistrationContext::new(),
                &mut sources,
            );
            let (_, kept) = item_render::bind_item_tint_factories(sources);
            assert_eq!(kept, ["example:charge"]);
        });
    }

    /// The resolver reads a mod's colour per stack, white for another's or with no mod.
    #[test]
    #[ignore = "daily"]
    fn item_tint_source_reads_the_mods_colour() {
        use mod_api::__private::game::*;
        let fields = serde_json::json!({"type": "example:charge", "low": 0xFFAA_BBCC_u32})
            .as_object()
            .cloned()
            .unwrap();
        let one = StackView::new("minecraft:stick", 1);
        let two = StackView::new("minecraft:stick", 2);
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("charge", Box::new(Charge))].into();
        mod_platform::testing::with_mods(mods, || {
            let mut sources = Vec::new();
            sunrise_gameplay_common::game::native_hooks::native_register_item_tint_sources(
                &mod_api::RegistrationContext::new(),
                &mut sources,
            );
            let (factories, _) = item_render::bind_item_tint_factories(sources);
            let bound =
                item_render::decode_tint_source(Some(&factories), "example:charge", &fields)
                    .unwrap()
                    .expect("registered tint source");
            assert!(
                item_render::decode_tint_source(Some(&factories), "other:charge", &fields)
                    .unwrap()
                    .is_none()
            );
            super::super::native_entity_refs::tests::with_client_game(|game| {
                let env = Env { game };
                let color = |stack| env.item_tint_source(&bound, stack) as u32;
                assert_eq!(color(&one), 0xFFAA_BBCC);
                assert_eq!(color(&two), 0xFF11_2233);
            });
        });
        mod_platform::testing::with_mods(Vec::new().into(), || {
            let mut sources = Vec::new();
            sunrise_gameplay_common::game::native_hooks::native_register_item_tint_sources(
                &mod_api::RegistrationContext::new(),
                &mut sources,
            );
            let (factories, _) = item_render::bind_item_tint_factories(sources);
            assert!(
                item_render::decode_tint_source(Some(&factories), "example:charge", &fields)
                    .unwrap()
                    .is_none()
            );
        });
    }

    #[test]
    fn civil_dates_follow_the_gregorian_calendar() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(19_716), (2023, 12, 25));
        assert_eq!(civil(11_016), (2000, 2, 29));
    }

    #[test]
    fn a_shield_is_used_for_the_vanilla_s_time() {
        // Item.getUseDuration: an item with `blocks_attacks` (a shield) is
        // used for 72000 ticks, so its use animation (the arm held up)
        // runs instead of ending at once (#2289).
        let mut shield = Components::new();
        shield.insert("minecraft:blocks_attacks".into(), json!({}));
        assert_eq!(use_duration(ComponentsView::of(&shield)), 72_000);
        // Food is used for as long as it takes to eat it.
        let mut food = Components::new();
        food.insert(
            "minecraft:consumable".into(),
            json!({"consume_seconds": 1.6}),
        );
        assert_eq!(use_duration(ComponentsView::of(&food)), 32);
        // The bow keeps its own use time.
        let mut bow = Components::new();
        bow.insert("minecraft:item_model".into(), json!("minecraft:bow"));
        assert_eq!(use_duration(ComponentsView::of(&bow)), 72_000);
        // Anything else is not used at all.
        assert_eq!(use_duration(ComponentsView::of(&Components::new())), 0);
    }

    /// 26.3 parity: `Game::submit_item_world`, `Game::submit_item_unlit`, and
    /// `Game::draw_item_model` emit bit-exact vanilla glint UVs for genpvp EID 83245
    /// (`minecraft:end_crystal`, `DisplayContext::Gui` & `Ground`, `GLINT_TEXTURING` 8.0 +
    /// `items` atlas UV) and for an enchanted shield (`ENTITY_GLINT_TEXTURING` 0.5 +
    /// `shield_patterns` atlas UV).
    #[test]
    #[ignore = "daily"]
    fn item_display_end_crystal_world_submit_uses_vanilla_glint_scale_and_atlas_uv_26_3() {
        run_item_display_end_crystal_world_submit_uses_vanilla_glint_scale_and_atlas_uv_26_3();
    }

    pub(in crate::game::client_side) fn run_item_display_end_crystal_world_submit_uses_vanilla_glint_scale_and_atlas_uv_26_3()
     {
        sunrise_gameplay_common::game::in_own_process(
            "game::client_side::item_draw::tests::item_display_end_crystal_world_submit_uses_vanilla_glint_scale_and_atlas_uv_26_3",
            || {
                let witness_path = std::env::var_os("SUNRISE_ITEM_GLINT_WITNESS")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| {
                        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                            .join("../../.local/witness/26.3/glint/witness.json")
                    });
                let witness_text = std::fs::read_to_string(&witness_path).unwrap_or_else(|e| {
                    panic!(
                        "failed to read item glint witness {} (generate with tools/test_glint_parity.py --record-only): {e}",
                        witness_path.display()
                    )
                });
                let witness: Value =
                    serde_json::from_str(&witness_text).expect("valid 26.3 item_glint witness");
                let root = std::env::var_os("SUNRISE_TEST_RESOURCES")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| {
                        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                            .join("../../.local/sunrise-client/26.3")
                    });
                fn parse_pos(v: &Value) -> [[u32; 3]; 4] {
                    let arr = v.as_array().unwrap();
                    std::array::from_fn(|i| {
                        let p = arr[i].as_array().unwrap();
                        [
                            p[0].as_u64().unwrap() as u32,
                            p[1].as_u64().unwrap() as u32,
                            p[2].as_u64().unwrap() as u32,
                        ]
                    })
                }
                fn parse_quad(v: &Value) -> [[u32; 2]; 4] {
                    let arr = v.as_array().unwrap();
                    std::array::from_fn(|i| {
                        let p = arr[i].as_array().unwrap();
                        [p[0].as_u64().unwrap() as u32, p[1].as_u64().unwrap() as u32]
                    })
                }
                fn assert_extruded_match(
                    verts: &[Vertex],
                    quads: &[item_render::ItemQuad],
                    expected: &[Value],
                    label: &str,
                ) {
                    assert_eq!(verts.len(), expected.len() * 4, "{label} vertex count");
                    assert_eq!(quads.len(), expected.len(), "{label} quad count");
                    let extract = |idx: usize| {
                        let vq = &verts[idx * 4..(idx + 1) * 4];
                        let q = &quads[idx];
                        let pos: [[u32; 3]; 4] = std::array::from_fn(|i| {
                            let bits = vq[i].pos.map(f32::to_bits);
                            assert_eq!(q.pos[i].to_array().map(f32::to_bits), bits);
                            assert_eq!(q.uv[i].map(f32::to_bits), vq[i].uv.map(f32::to_bits));
                            bits
                        });
                        let uv0 = q.gui_glint_uv.unwrap().map(|p| p.map(f32::to_bits));
                        let glint_uv: [[u32; 2]; 4] =
                            std::array::from_fn(|i| vq[i].glint_uv.map(f32::to_bits));
                        (pos, uv0, glint_uv)
                    };
                    // Front/back faces `[0..2]` match by index.
                    for i in 0..2 {
                        let (pos, uv0, glint_uv) = extract(i);
                        assert_eq!(
                            pos,
                            parse_pos(&expected[i]["pos_bits"]),
                            "{label} q[{i}] pos"
                        );
                        assert_eq!(
                            uv0,
                            parse_quad(&expected[i]["uv0_bits"]),
                            "{label} q[{i}] uv0"
                        );
                        assert_eq!(
                            glint_uv,
                            parse_quad(&expected[i]["glint_uv_t123456_bits"]),
                            "{label} q[{i}] glint_uv"
                        );
                    }
                    // Side faces `[2..]` match 1-to-1 by `(pos_bits, uv0_bits)`; fail on dup/unmatched.
                    let mut witness_sides: HashMap<([[u32; 3]; 4], [[u32; 2]; 4]), &Value> =
                        HashMap::with_capacity(expected.len() - 2);
                    for w in &expected[2..] {
                        let key = (parse_pos(&w["pos_bits"]), parse_quad(&w["uv0_bits"]));
                        assert!(
                            witness_sides.insert(key, w).is_none(),
                            "{label} duplicate witness side key"
                        );
                    }
                    let mut seen: std::collections::HashSet<([[u32; 3]; 4], [[u32; 2]; 4])> =
                        std::collections::HashSet::with_capacity(quads.len() - 2);
                    for i in 2..quads.len() {
                        let (pos, uv0, glint_uv) = extract(i);
                        let key = (pos, uv0);
                        assert!(seen.insert(key), "{label} duplicate Sunrise side key");
                        let w = witness_sides
                            .remove(&key)
                            .unwrap_or_else(|| panic!("{label} unmatched Sunrise side key"));
                        assert_eq!(pos, parse_pos(&w["pos_bits"]), "{label} side pos");
                        assert_eq!(uv0, parse_quad(&w["uv0_bits"]), "{label} side uv0");
                        assert_eq!(
                            glint_uv,
                            parse_quad(&w["glint_uv_t123456_bits"]),
                            "{label} side glint_uv"
                        );
                    }
                    assert!(
                        witness_sides.is_empty(),
                        "{label} {} unmatched witness side faces",
                        witness_sides.len()
                    );
                }
                super::super::native_entity_refs::tests::with_game_role_client(
                    root,
                    super::super::execution_role::ExecutionRole::Client,
                    |game| {
                        game.client
                            .client_pack
                            .item_renderer
                            .prepare_glint_atlas(16384, (0, 2))
                            .unwrap();
                        game.client.glint_frame = item_render::GlintFrame {
                            millis: 123_456,
                            speed: 0.5,
                            alpha: 0.75,
                            sprite: true,
                        };
                        let crystal = ItemStack::new("minecraft:end_crystal", 1);
                        // Exact `DisplayRenderer.ItemDisplayRenderer` pose for EID 83245 (`CENTER` billboard).
                        let mut eid_p = joml::Mat4::IDENTITY;
                        eid_p.rotate(joml::Quat::rotation_yxz(
                            (-std::f64::consts::PI / 180.0) as f32 * -180.0,
                            0.0,
                            0.0,
                        ));
                        eid_p.translate(-0.945, 0.3885, -4.2);
                        eid_p.rotate(joml::Quat {
                            x: 0.0,
                            y: 1.0,
                            z: 0.0,
                            w: 0.0,
                        });
                        eid_p.scale(0.525, 0.525, 0.525);
                        eid_p.rotate_y(std::f32::consts::PI);
                        let eid_pose = eid_p.to_glam();
                        let eid_quads = witness["end_crystal_item_display"]["quads"]
                            .as_array()
                            .unwrap();

                        let state_gui =
                            game.resolve_item(&crystal, DisplayContext::Gui, None, 83245);
                        let mut world_out = Vec::new();
                        game.submit_item_world(&state_gui, eid_pose, 0xF0F0, &mut world_out);
                        assert!(world_out.iter().all(|v| (v.layer & 0xFFFF) == 7));
                        let mut world_quads = Vec::new();
                        state_gui.submit(
                            &game.client.client_pack.item_renderer,
                            &game.client.gui.atlas.sprites,
                            &game.item_host(),
                            eid_pose,
                            &mut world_quads,
                        );
                        assert_extruded_match(
                            &world_out,
                            &world_quads,
                            eid_quads,
                            "26.3 Game::submit_item_world end_crystal EID 83245",
                        );

                        let ground_quads_want =
                            witness["contexts"]["ground"]["quads"].as_array().unwrap();
                        let state_ground =
                            game.resolve_item(&crystal, DisplayContext::Ground, None, 83245);
                        let mut unlit_out = Vec::new();
                        game.submit_item_unlit(
                            &state_ground,
                            glam::Mat4::IDENTITY,
                            0xF0F0,
                            &mut unlit_out,
                        );
                        let mut unlit_quads = Vec::new();
                        state_ground.submit(
                            &game.client.client_pack.item_renderer,
                            &game.client.gui.atlas.sprites,
                            &game.item_host(),
                            glam::Mat4::IDENTITY,
                            &mut unlit_quads,
                        );
                        assert_extruded_match(
                            &unlit_out,
                            &unlit_quads,
                            ground_quads_want,
                            "26.3 Game::submit_item_unlit end_crystal ground",
                        );

                        // GUI draw_item_model must not lose millis precision via `/ 1000.0 * 1000.0`.
                        let mut batch = HudBatch::scratch([64, 64]);
                        game.draw_item_model(&mut batch, &crystal, 0.0, 0.0, 0);
                        let want_front = parse_quad(&eid_quads[0]["glint_uv_t123456_bits"]);
                        let mut seen = [false; 4];
                        for v in batch.vertices.iter().filter(|v| v.uv[0] >= 2.0) {
                            let corner = match v.pos {
                                [0.0, 0.0] => 0,
                                [0.0, 16.0] => 1,
                                [16.0, 16.0] => 2,
                                [16.0, 0.0] => 3,
                                p => panic!("unexpected end_crystal GUI corner {p:?}"),
                            };
                            assert_eq!(
                                v.glint_uv.map(f32::to_bits),
                                want_front[corner],
                                "26.3 Game::draw_item_model end_crystal GUI corner[{corner}]"
                            );
                            seen[corner] = true;
                        }
                        assert!(seen.into_iter().all(|ok| ok));

                        // Enchanted shield in world submit must keep ENTITY_GLINT_TEXTURING (0.5).
                        let mut shield = ItemStack::new("minecraft:shield", 1);
                        shield.data_mut().set_other(
                            "minecraft:enchantment_glint_override",
                            Some(sunrise_core::nbt::Tag::Byte(1)),
                        );
                        let state_shield =
                            game.resolve_item(&shield, DisplayContext::Ground, None, 0);
                        let mut shield_out = Vec::new();
                        game.submit_item_world(
                            &state_shield,
                            glam::Mat4::IDENTITY,
                            0xF0F0,
                            &mut shield_out,
                        );
                        let mut want_shield: Vec<[[u32; 2]; 4]> = witness["special_models"]["shield_plain"]
                    ["ground"]["models"][0]["quads"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|q| parse_quad(&q["glint_uv_t123456_bits"]))
                    .collect();
                        want_shield.sort();
                        let mut got_shield: Vec<[[u32; 2]; 4]> = shield_out
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .map(|quad| std::array::from_fn(|i| quad[i].glint_uv.map(f32::to_bits)))
                            .collect();
                        got_shield.sort();
                        assert_eq!(
                            got_shield, want_shield,
                            "26.3 Game::submit_item_world enchanted shield"
                        );
                    },
                );
            },
        );
    }
}
