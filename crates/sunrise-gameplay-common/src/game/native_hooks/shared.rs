#[cfg(any(test, feature = "test-support"))]
#[path = "shared/item_fall_on_tests.rs"]
pub mod item_fall_on_tests;
#[path = "shared/level_helpers.rs"]
pub mod level_helpers;
#[cfg(any(test, feature = "test-support"))]
pub use level_helpers::block_entity_for_block;
#[cfg(test)]
#[path = "shared/block_property_tests.rs"]
pub mod block_property_tests;
#[cfg(test)]
#[path = "shared/gameplay_hook_tests.rs"]
pub mod gameplay_hook_tests;
#[path = "shared/item_fallthrough.rs"]
pub mod item_fallthrough;
#[cfg(test)]
#[path = "shared/label_tests.rs"]
pub mod label_tests;
#[cfg(test)]
#[path = "shared/live_damage_tests.rs"]
pub mod live_damage_tests;
#[path = "shared/loot.rs"]
pub mod loot;
#[cfg(test)]
#[path = "shared/packet_tests.rs"]
pub mod packet_tests;
#[cfg(any(test, feature = "test-support"))]
#[path = "shared/slot_icon_tests.rs"]
pub mod slot_icon_tests;
#[cfg(any(test, feature = "test-support"))]
#[path = "shared/slot_tooltip_tests.rs"]
pub mod slot_tooltip_tests;
#[path = "shared/system_helpers.rs"]
pub mod system_helpers;
#[path = "shared/world_helpers.rs"]
pub mod world_helpers;

pub use crate::game::api_values::{TryIntoApi, TryIntoEngine};
pub use crate::game::item_particle_send::supported;
#[cfg(test)]
pub use screen_chars::chars_to_type;

/// Write back a `menu_button` cursor edit without revalidating an untouched stack.
pub fn apply_menu_button_cursor(
    items: &sunrise_items::items::Items,
    original: &Option<mod_api::ItemStack>,
    cursor: &Option<mod_api::ItemStack>,
    carried: &mut Option<ItemStack>,
) {
    if cursor == original {
        return;
    }
    match cursor.try_into_engine(items) {
        Ok(updated) => *carried = updated,
        Err(error) => {
            log::warn!("mod menu_button write refused: {error:?}");
        }
    }
}

/// `Level.setBlock`'s flags, for the `native_set_block_property` helpers:
/// `flags::UPDATE_CLIENTS` as a vanilla crop grows, `flags::UPDATE_ALL` as a
/// furnace sets its `lit`.
#[allow(unused_imports)]
pub use sunrise_world::level::flags;

#[allow(unused_imports)]
pub use camera_math::hand_pose_matrix;

/// How the panel lays out a recipe of `native_panel_recipes`.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeRecipeLayout {
    /// A crafting grid of `width` x `height` slots, an arrow and the result.
    Grid,
    /// One input over a flame, an arrow and the result (furnaces, smokers,
    /// campfires).
    Cooking,
    /// One input, an arrow and the result (the stonecutter).
    Single,
    /// Template, base and addition, an arrow and the result.
    Smithing,
    /// A potion and an ingredient, an arrow and the result.
    Brewing,
}

/// One result of a recipe the panel shows (`native_panel_recipes`): a slot
/// cycling through `stacks`, one a second, and the chance in percent the recipe
/// gives it (`Some(25.0)` draws "25%"); `None` gives it every time.
#[derive(Clone, Debug, PartialEq)]
pub struct NativeRecipeResult {
    pub stacks: Vec<mod_api::Stack>,
    pub chance: Option<f32>,
}

/// A recipe the panel shows (`native_panel_recipes`). Each slot, and each
/// result, cycles through its stacks, one a second; an empty slot shows
/// nothing. A crafting recipe has one result and no chance.
#[derive(Clone, Debug, PartialEq)]
pub struct NativeRecipeView {
    pub layout: NativeRecipeLayout,
    /// The grid's size (`Grid` only), at most 3 x 3; slots past it continue
    /// in rows under it, 27 in all.
    pub width: u32,
    pub height: u32,
    pub slots: Vec<Vec<mod_api::Stack>>,
    /// Up to four, left to right after the arrow.
    pub results: Vec<NativeRecipeResult>,
    /// Fluids the recipe takes and gives, `amount` in millibuckets, each in
    /// a well with its amount under it; at most eight each. A well shows the
    /// fluid's client look (`data/<mod>/sunrise/fluid/`: sprite and tint), a
    /// flat square of vanilla's colour for water and lava and else grey when
    /// the fluid has none.
    pub fluid_inputs: Vec<FluidStack>,
    pub fluid_outputs: Vec<FluidStack>,
    /// A short line under the recipe (a cooking time), or "".
    pub note: String,
    /// The action button beside it: none, enabled, or shown greyed.
    pub action: Option<bool>,
}

/// `native_living_use_totem` for the damage code: whether it
/// canceled the totem. The entity and the source are named as the damage hook names them.
pub fn use_totem(
    level: DamageLevel<'_>,
    hurting: Hurting<'_>,
    hand: Hand,
    stack: &ItemStack,
) -> bool {
    if !mod_api::__private::listened(mod_api::HookId::LivingUseTotem) {
        return false;
    }
    let (entity, kind, source) = hurt_server_args(hurting, level.attack());
    let mut cancel = false;
    HitLevel(level).native_living_use_totem(entity, kind, &source, hand, stack, &mut cancel);
    cancel
}

/// `native_living_death_protection` for the damage code: a mod's protector from outside the
/// hands, once neither hand protected.
pub fn death_protection_source(level: DamageLevel<'_>, hurting: Hurting<'_>) -> Option<ItemStack> {
    if !mod_api::__private::listened(mod_api::HookId::LivingDeathProtection) {
        return None;
    }
    let (entity, kind, source) = hurt_server_args(hurting, level.attack());
    let mut protector = None;
    HitLevel(level).native_living_death_protection(entity, kind, &source, &mut protector);
    protector
}

/// The level a hit or a landing happens in, as its hooks' level host: its blocks, its random and its
/// own attachments. Not `GameLevel`, which a hit can't build: a mob's hit lands in the mob
/// tick, where the world is read by every mob and its random is lent out, and building one
/// per hit would allocate its entity boxes.
pub struct HitLevel<'a>(pub DamageLevel<'a>);

impl<'a> HitLevel<'a> {
    /// The item table a mod's stack is checked against as it comes back into the hit
    /// (`living_death_protection`'s protector): the authoritative world's, as for the hit's reads.
    pub fn stack_boundary_items(&self) -> Option<&'a sunrise_items::items::Items> {
        self.0.tags()
    }
}

/// Let listeners override the vanilla invulnerability decision for this hit.
pub fn invulnerability_check(level: DamageLevel<'_>, hurting: Hurting<'_>, vanilla: bool) -> bool {
    if !mod_api::__private::listened(mod_api::HookId::EntityInvulnerabilityCheck) {
        return vanilla;
    }
    let (entity, kind, source) = hurt_server_args(hurting, level.attack());
    let mut invulnerable = vanilla;
    HitLevel(level).native_entity_invulnerability_check(
        entity,
        kind,
        &source,
        vanilla,
        &mut invulnerable,
    );
    invulnerable
}

pub fn engine_reduction(reduction: mod_api::Reduction) -> sunrise_world::living_damage::Reduction {
    match reduction {
        mod_api::Reduction::Invulnerability => {
            sunrise_world::living_damage::Reduction::Invulnerability
        }
        mod_api::Reduction::Armor => sunrise_world::living_damage::Reduction::Armor,
        mod_api::Reduction::Enchantments => sunrise_world::living_damage::Reduction::Enchantments,
        mod_api::Reduction::MobEffects => sunrise_world::living_damage::Reduction::MobEffects,
        mod_api::Reduction::Absorption => sunrise_world::living_damage::Reduction::Absorption,
        mod_api::Reduction::InnateResistance => {
            sunrise_world::living_damage::Reduction::InnateResistance
        }
        _ => unreachable!("unknown API reduction"),
    }
}

pub struct LiveDamageRead<'a>(pub &'a sunrise_world::living_damage::DamageState);

impl mod_api::__private::DamageContainerRead for LiveDamageRead<'_> {
    fn original(&self) -> f32 {
        self.0.original()
    }
    fn new_damage(&self) -> f32 {
        self.0.new_damage()
    }
    fn blocked(&self) -> f32 {
        self.0.blocked()
    }
    fn shield_damage(&self) -> f32 {
        self.0.shield_damage()
    }
    fn post_attack_invulnerability_ticks(&self) -> i32 {
        self.0.post_attack_invulnerability_ticks()
    }
    fn should_cause_side_effects(&self) -> bool {
        self.0.should_cause_side_effects()
    }
    fn inflicted(&self) -> f32 {
        self.0.inflicted()
    }
    fn health_damage(&self) -> f32 {
        self.0.health_damage()
    }
    fn reduction(&self, reduction: mod_api::Reduction) -> f32 {
        self.0.reduction(engine_reduction(reduction))
    }
}

pub struct LiveDamageContainer<'a>(pub(in crate::game) &'a mut DamageContainer);

impl mod_api::__private::DamageContainerRead for LiveDamageContainer<'_> {
    fn original(&self) -> f32 {
        self.0.original()
    }
    fn new_damage(&self) -> f32 {
        self.0.new_damage()
    }
    fn blocked(&self) -> f32 {
        self.0.blocked()
    }
    fn shield_damage(&self) -> f32 {
        self.0.shield_damage()
    }
    fn post_attack_invulnerability_ticks(&self) -> i32 {
        self.0.post_attack_invulnerability_ticks()
    }
    fn should_cause_side_effects(&self) -> bool {
        self.0.should_cause_side_effects()
    }
    fn inflicted(&self) -> f32 {
        self.0.inflicted()
    }
    fn health_damage(&self) -> f32 {
        self.0.health_damage()
    }
    fn reduction(&self, reduction: mod_api::Reduction) -> f32 {
        self.0.reduction(engine_reduction(reduction))
    }
}

impl mod_api::__private::DamageContainerBackend for LiveDamageContainer<'_> {
    fn set_new_damage(&mut self, value: f32) {
        self.0.record_new_damage(value);
    }
    fn set_post_attack_invulnerability_ticks(&mut self, value: i32) {
        self.0.record_post_attack_invulnerability_ticks(value);
    }
    fn add_reduction_modifier(
        &mut self,
        reduction: mod_api::Reduction,
        owner: Option<&'static str>,
        modifier: Box<dyn FnMut(mod_api::DamageContainerView<'_>, f32) -> f32>,
    ) {
        let mut wrapped = mod_api::__private::wrap_reduction_modifier(owner, modifier);
        self.0.add_reduction_modifier(
            engine_reduction(reduction),
            Box::new(move |engine_container: &DamageContainer, amount: f32| {
                wrapped(
                    mod_api::DamageContainerView::new(&LiveDamageRead(engine_container)),
                    amount,
                )
            }),
        );
    }
}

/// `native_living_incoming_damage` for the damage code: whether it
/// canceled the hit.
pub fn incoming_damage(
    level: DamageLevel<'_>,
    hurting: Hurting<'_>,
    victim: Victim,
    container: &mut DamageContainer,
) -> bool {
    let listens_incoming = mod_api::__private::listened(mod_api::HookId::LivingIncomingDamage);
    if listens_incoming
        || mod_api::__private::listened(mod_api::HookId::LivingDamagePre)
        || mod_api::__private::listened(mod_api::HookId::LivingDamagePost)
    {
        container.enable_live_reductions();
    }
    if !listens_incoming {
        return false;
    }
    let (entity, kind, source) = hurt_server_args(hurting, level.attack());
    let mut canceled = false;
    let attack = level.attack();
    let weapon = attack.weapon_of(source.direct);
    let causing_weapon = (source.causing != 0 && source.causing == attack.attacker)
        .then_some(attack.held[0].as_ref())
        .flatten();
    with_hit_weapons(weapon, causing_weapon, |weapon, causing_weapon| {
        let mut live = LiveDamageContainer(container);
        HitLevel(level.caused_by(source.causing)).native_living_incoming_damage(
            mod_api::__private::LivingIncomingDamageCall {
                entity,
                kind,
                source: &source,
                victim: &victim,
                weapon,
                causing_weapon,
                container: mod_api::DamageContainer::new(&mut live),
                canceled: &mut canceled,
            },
        );
    });
    canceled
}

/// One melee weapon can be both the direct and the causing weapon. Feed both fields the
/// same view so reading either doesn't encode that stack's components again.
fn with_hit_weapons(
    direct: Option<&ItemStack>,
    causing: Option<&ItemStack>,
    dispatch: impl FnOnce(Option<&mod_api::ItemStackView<'_>>, Option<&mod_api::ItemStackView<'_>>),
) {
    crate::game::api_values::with_stack_view(direct, |direct_view| {
        if direct.zip(causing).is_some_and(|(a, b)| std::ptr::eq(a, b)) {
            dispatch(direct_view.as_ref(), direct_view.as_ref());
        } else {
            crate::game::api_values::with_stack_view(causing, |causing_view| {
                dispatch(direct_view.as_ref(), causing_view.as_ref());
            });
        }
    });
}

/// `native_living_damage_pre` for the damage code.
pub fn damage_pre(level: DamageLevel<'_>, hurting: Hurting<'_>, container: &mut DamageContainer) {
    if !mod_api::__private::listened(mod_api::HookId::LivingDamagePre) {
        return;
    }
    container.enable_live_reductions();
    let (entity, kind, source) = hurt_server_args(hurting, level.attack());
    let mut live = LiveDamageContainer(container);
    HitLevel(level.caused_by(source.causing)).native_living_damage_pre(
        mod_api::__private::LivingDamagePreCall {
            entity,
            kind,
            source: &source,
            container: mod_api::DamageContainer::new(&mut live),
        },
    );
}

/// `native_living_damage_post` for the damage code.
pub fn damage_post(level: DamageLevel<'_>, hurting: Hurting<'_>, container: &DamageContainer) {
    if !mod_api::__private::listened(mod_api::HookId::LivingDamagePost) {
        return;
    }
    let (entity, kind, source) = hurt_server_args(hurting, level.attack());
    let live = LiveDamageRead(container);
    HitLevel(level.caused_by(source.causing)).native_living_damage_post(
        entity,
        kind,
        &source,
        mod_api::DamageContainerView::new(&live),
    );
}

/// `native_living_shield_block` for the damage code. Its one
/// caller. The hurt entity and the source are named as the damage hook names them.
pub fn shield_block(
    level: DamageLevel<'_>,
    hurting: Hurting<'_>,
    damage: f32,
    original_blocked: bool,
    block: &mut ShieldBlock,
) {
    let (entity, kind, source) = hurt_server_args(hurting, level.attack());
    HitLevel(level).native_living_shield_block(
        entity,
        kind,
        &source,
        damage,
        original_blocked,
        block,
    );
}

/// `native_living_knockback` for the knockback code: whether it
/// canceled the push. Its one caller. The pushed entity and the source are named as the
/// damage hook names them.
pub fn knockback(level: DamageLevel<'_>, pushed: Hurting<'_>, knock: &mut Knock) -> bool {
    if !mod_api::__private::listened(mod_api::HookId::LivingKnockback) {
        return false;
    }
    let attack = level.attack();
    let attacker = match &pushed {
        Hurting::Mob(mob, damage) => match damage.attacker.or(mob.ai.last_hurt_by) {
            Some(Who::Mob(id)) => id as i32,
            _ => attack.attacker,
        },
        Hurting::Player(_, source)
        | Hurting::Dragon(_, source)
        | Hurting::ArmorStand(_, source)
        | Hurting::Entity(_, _, source) => match source.causing {
            0 => attack.attacker,
            causing => causing,
        },
    };
    let (level, kill_credit, credit_main) = level.split_knockback_credit(&pushed);
    let (entity, kind, source) = hurt_server_args(pushed, attack);
    let mut canceled = false;
    let [main, off] = &attack.held;
    let read_metadata = |slot: &str, stack: Option<&sunrise_items::items::ItemStack>| {
        let stack = stack?;
        if stack.count == 0 || &*stack.item == "minecraft:air" {
            return None;
        }
        match crate::game::api_values::stack_read_metadata(stack) {
            Ok(metadata) => Some(metadata),
            Err(error) => {
                log::warn!(
                    "living_knockback on entity {entity} ({kind}): omitting unrepresentable {slot} '{}' (count {}): {error:?}",
                    stack.item,
                    stack.count,
                );
                None
            }
        }
    };
    let main_metadata = read_metadata("attacker_main", main.as_ref());
    let off_metadata = read_metadata("attacker_off", off.as_ref());
    let credit_metadata = read_metadata("kill_credit_main", credit_main);
    let project_main = |metadata| {
        main.as_ref()
            .and_then(|stack| crate::game::api_values::project_stack(stack, metadata).ok())
    };
    let project_off = |metadata| {
        off.as_ref()
            .and_then(|stack| crate::game::api_values::project_stack(stack, metadata).ok())
    };
    let project_credit = |metadata| {
        credit_main.and_then(|stack| crate::game::api_values::project_stack(stack, metadata).ok())
    };
    let attacker_main =
        main_metadata.map(|metadata| mod_api::ItemStackView::resident(metadata, &project_main));
    let attacker_off =
        off_metadata.map(|metadata| mod_api::ItemStackView::resident(metadata, &project_off));
    let kill_credit_main =
        credit_metadata.map(|metadata| mod_api::ItemStackView::resident(metadata, &project_credit));
    // NeoForge `LivingEntity.knockback` posts `(float) strength` and widens the result back.
    let mut strength = knock.strength as f32;
    HitLevel(level).native_living_knockback(mod_api::__private::LivingKnockbackCall {
        entity,
        kind,
        source: &source,
        attacker,
        attacker_main,
        attacker_off,
        kill_credit,
        kill_credit_main,
        strength: &mut strength,
        x: &mut knock.x,
        z: &mut knock.z,
        canceled: &mut canceled,
    });
    knock.strength = f64::from(strength);
    canceled
}

/// The damage hooks every level of the game lends to its hits (`Game.damage_hooks`).
pub const DAMAGE_HOOKS: DamageHooks = DamageHooks {
    invulnerable: Some(invulnerability_check),
    incoming: Some(incoming_damage),
    pre: Some(damage_pre),
    post: Some(damage_post),
    owner_post_listened: Some(|| mod_api::__private::listened(mod_api::HookId::LivingDamagePost)),
    knockback: Some(knockback),
    totem: Some(use_totem),
    protector: Some(death_protection_source),
    shield: Some(shield_block),
    accepted: Some(crate::game::native_hooks::living_death::capture_hit),
};
/// The contacts of a simulating entity with a block, as the simulation's own code (client-lib)
/// reports them.
pub const BLOCK_CONTACT_HOOKS: sunrise_sim::block_contact::Hooks =
    sunrise_sim::block_contact::Hooks {
        step_listens: |state| {
            crate::game::api_values::block_state::block_listens(mod_api::HookId::StepOn, state)
        },
        step_on: |level, step, movement| {
            use self::bounce_restitution::StepOnView;
            use sunrise_sim::block_contact::StepLevel;
            match level {
                StepLevel::Server(level) => StepOnView(mod_api::LevelRandomSide::Server(
                    &mut HitLevel(level.reborrow()),
                ))
                .native_step_on(step, movement),
                StepLevel::Client(level) => StepOnView(mod_api::LevelRandomSide::Client(
                    &crate::game::native_surface::FrictionView(level, None),
                ))
                .native_step_on(step, movement),
            }
        },
        any_inside: || mod_api::__private::has_scoped_hook(mod_api::HookId::EntityInside),
        inside_listens: |state| {
            crate::game::api_values::block_state::block_listens(
                mod_api::HookId::EntityInside,
                state,
            )
        },
        pass_listens: |state| {
            crate::game::api_values::block_state::block_listens(
                mod_api::HookId::MinecartPass,
                state,
            )
        },
        fall_listens: |state| {
            crate::game::api_values::block_state::block_listens(mod_api::HookId::FallOn, state)
        },
        bounce_listens: |state| {
            crate::game::api_values::block_state::block_listens(mod_api::HookId::BounceOn, state)
        },
        inside_fire: |level, dimension, entity, kind, state, pos, precise, effects| {
            HitLevel(level.reborrow()).native_entity_inside(
                crate::game::native_hooks::fire::EntityInsideCall {
                    dimension,
                    entity,
                    kind,
                    state,
                    pos,
                    precise,
                    effects,
                },
            );
        },
        state_to_api: |id| id.try_into_api().ok(),
        state_to_engine: |state| state.try_into_engine(&()).ok(),
        fall_on: |dimension, level, c, distance, damage_multiplier, writes| {
            HitLevel(level.reborrow()).native_fall_on(
                crate::game::native_hooks::fire::FallOnCall {
                    dimension,
                    entity: c.entity,
                    kind: c.kind,
                    state: c.state,
                    pos: c.position,
                    distance,
                    damage_multiplier,
                    writes,
                },
            );
        },
        bounce_on: |dimension, level, c, fall_distance, movement| {
            let mut answer = [movement.x, movement.y, movement.z];
            HitLevel(level.reborrow()).native_bounce_on(
                crate::game::native_hooks::fire::BounceOnCall {
                    dimension,
                    entity: c.entity,
                    kind: c.kind,
                    state: c.state,
                    pos: c.position,
                    fall_distance,
                    movement: &mut answer,
                },
            );
            *movement = sunrise_core::geometry::Vec3::new(answer[0], answer[1], answer[2]);
        },
    };
#[allow(unused_imports)]
pub use living_death::hurt_server_args;
#[allow(unused_imports)]
pub use sunrise_sim::health::mob_source;

/// Whether a native mod's particles can be sent. Block particle types need
/// a known state (`state_ok`), item particles a validated stack; trail and vibration
/// aren't supported by the network dispatch. A mod's own type must be namespaced.
pub fn mod_particles_ok(
    options: &ParticleOptions,
    state: Option<BlockStateId>,
    state_ok: impl Fn(BlockStateId) -> bool,
    positions: &[[f64; 3]],
    spread: [f64; 3],
    speed: f64,
) -> bool {
    use ParticleType as T;
    // A native mod's own type: the name has to be namespaced, and a client
    // draws it only when its pack has the sprite set.
    if let Some(name) = &options.mod_particle {
        let namespaced = name
            .split_once(':')
            .is_some_and(|(ns, path)| !ns.is_empty() && !path.is_empty());
        if !namespaced {
            return false;
        }
    }
    let supported = match options.ty {
        T::Block | T::BlockMarker | T::FallingDust | T::DustPillar | T::BlockCrumble => {
            state.is_some_and(state_ok)
        }
        T::Item | T::Trail | T::Vibration => supported(options),
        T::Geyser | T::GeyserPlume | T::GeyserBase | T::GeyserPoof => options
            .geyser
            .is_some_and(|(_, impulse)| impulse.is_finite()),
        _ => true,
    };
    supported
        && positions.iter().flatten().all(|v| v.is_finite())
        && spread
            .into_iter()
            .all(|v| v.is_finite() && v.abs() <= f32::MAX as f64)
        && speed.is_finite()
        && speed.abs() <= f32::MAX as f64
        && options.color.into_iter().all(f32::is_finite)
        && options.to_color.into_iter().all(f32::is_finite)
        && options.scale.is_finite()
}

pub fn dimension_id(dimension: LevelDimension) -> &'static str {
    match dimension {
        LevelDimension::Overworld => "minecraft:overworld",
        LevelDimension::Nether => "minecraft:the_nether",
        LevelDimension::End => "minecraft:the_end",
        LevelDimension::Custom(key) => key.id(),
    }
}

/// A mob's kind as the hook points name it (`minecraft:zombie`).
pub fn mob_kind(mob: &sunrise_sim::mobs::Mob) -> String {
    crate::game::native_namespaced_id::namespaced(&mob.kind.id)
}

/// The level behaviour's answer, or `None` to try `useWithoutItem`.
/// `ServerPlayerGameMode.useItemOn` lets both FAIL and PASS go to the item.
/// Both success swing sources swing on this server path.
pub fn block_use_result(
    answer: mod_api::InteractionResult,
    side: LogicalSide,
) -> Option<sunrise_world::level::InteractionResult> {
    use mod_api::InteractionResult as Answer;
    use sunrise_world::level::InteractionResult;
    match answer {
        Answer::SuccessServer if side == LogicalSide::Client => Some(InteractionResult::Consume),
        Answer::Success | Answer::SuccessServer => Some(InteractionResult::Success),
        Answer::Consume => Some(InteractionResult::Consume),
        Answer::TryWithEmptyHand => None,
        _ => Some(InteractionResult::Pass),
    }
}

/// The captured target or hand changed during client block-use prediction.
#[derive(Debug, PartialEq, Eq)]
pub struct StaleBlockClick;

/// What the mods answered a right click on a block (`right_click_block`).
pub enum BlockClickAnswer {
    /// Cancelled: this answers the click instead of the block and the item.
    Canceled(mod_api::InteractionResult),
    /// The block's use and the item's `useOn` run as these say.
    Go {
        use_block: TriState,
        use_item: TriState,
    },
}

/// What `BonemealEvent` settled for one use of bone meal.
pub struct BonemealAsk {
    /// `Some(successful)` when a mod canceled the event: `growCrop`'s answer.
    pub crop: Option<bool>,
    /// `isValidBonemealTarget` as the event asked it, when a mod listens:
    /// `growCrop` takes it instead of asking again.
    pub check: Option<BonemealCheck>,
}

/// Where the game calls the hook points.
impl crate::game::CommonGame<'_> {
    /// A mod block's container at `key` opens its menu: its slots grown to
    /// the menu's. False for any other block.
    pub fn open_mod_menu(&mut self, key: Pos) -> bool {
        let state = self.world.block(key.0, key.1, key.2).0;
        let Some(c) = self.world.block_entities.mod_container(state) else {
            return false;
        };
        let slots = c.slots;
        if c.menu.is_none() {
            return false;
        }
        let Some(BlockEntity::Mod(m)) = self.world.block_entities.get_mut(&key) else {
            return false;
        };
        if m.items.len() < slots {
            m.items.resize(slots, None);
        }
        true
    }
    /// The menu type of the mod container at `key`, if it has one.
    pub fn mod_menu_at(&self, key: Pos) -> Option<&'static str> {
        let state = self.world.block(key.0, key.1, key.2).0;
        let c = self.world.block_entities.mod_container(state)?;
        mod_menu(c.menu.as_deref()?).map(|(menu, _)| menu)
    }
}

/// Where the game calls the hook points.
pub trait SharedGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// The chosen phase's broadcast event, before its scoped behaviour.
    fn use_item_on_block(
        &mut self,
        hit: &super::Hit,
        hand: Hand,
        stack: Option<&ItemStack>,
        phase: UsePhase,
    ) -> Option<mod_api::InteractionResult> {
        if !mod_api::__private::listened(mod_api::HookId::UseItemOnBlock) {
            return None;
        }
        let snapshot = stack.try_into_api().ok()?;
        let click = self.block_click(hit, hand, snapshot.as_ref());
        let (mut result, mut canceled) = (mod_api::InteractionResult::TryWithEmptyHand, false);
        self.native_use_item_on_block(&click, phase, &mut result, &mut canceled);
        if !canceled {
            return None;
        }
        if phase == UsePhase::ItemAfterBlock
            && result.consumes_action()
            && let Some(stack) = stack
        {
            self.with_hand(hand, |game| {
                game.side_award_stat("used", &stack.item, 1);
                game.side_apply_after_use_components(stack);
            });
        }
        Some(result)
    }

    /// `ItemStack.onItemUseFirst` for a click on a block: the broadcast
    /// event's `ItemBeforeBlock` phase, then the held item's Part. Canceling skips that Part.
    /// Only `Pass` continues to the block.
    fn item_use_first(
        &mut self,
        hit: &super::Hit,
        hand: Hand,
        stack: &ItemStack,
    ) -> Result<Option<mod_api::InteractionResult>, StaleBlockClick> {
        if let Some(answer) =
            self.use_item_on_block(hit, hand, Some(stack), UsePhase::ItemBeforeBlock)
        {
            return Ok(Some(answer));
        }
        if !mod_api::__private::item_listens(mod_api::HookId::OnItemUseFirst, &stack.item) {
            return Ok(None);
        }
        let Ok(snapshot) = stack.try_into_api() else {
            return Ok(None);
        };
        let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        let click = self.block_click(hit, hand, Some(&snapshot));
        self.with_block_click_part(&click, state, Some(stack), |game, answer| {
            crate::game::native_hooks::on_item_use_first::GameHooks::native_on_item_use_first(
                game, &snapshot, &click, answer,
            );
        })
    }

    /// Run the captured block's selected behaviour, only if that key subscribes.
    fn predict_block_part(
        &mut self,
        hit: &super::Hit,
        state: BlockStateId,
        hand: Hand,
        held: Option<&ItemStack>,
        branch: sunrise_world::level::UseBranch,
    ) -> Result<Option<mod_api::InteractionResult>, StaleBlockClick> {
        use sunrise_world::level::UseBranch;
        let hook = match branch {
            UseBranch::ItemOn => mod_api::HookId::UseItemOn,
            UseBranch::WithoutItem => mod_api::HookId::UseWithoutItem,
        };
        if !super::api_values::block_state::block_listens(hook, state) {
            return Ok(None);
        }
        let Ok(snapshot) = held.try_into_api() else {
            return Ok(None);
        };
        let click = self.block_click(hit, hand, snapshot.as_ref());
        self.with_block_click_part(&click, state, held, |game, answer| match branch {
            UseBranch::ItemOn => game.native_use_item_on(state, &click, answer),
            UseBranch::WithoutItem => game.native_use_without_item(state, &click, answer),
        })
    }

    /// A refused payload defers to the template. A stale decision must not
    /// be applied or retried against a different block or hand. The server's
    /// Parts may write and deliberately do not have this prediction guard.
    fn with_block_click_part(
        &mut self,
        click: &BlockClick<'_>,
        state: BlockStateId,
        held: Option<&ItemStack>,
        dispatch: impl FnOnce(&mut Self, &mut Option<mod_api::InteractionResult>),
    ) -> Result<Option<mod_api::InteractionResult>, StaleBlockClick> {
        let predicting = click.side == LogicalSide::Client;
        if predicting
            && (!click.hit.point.into_iter().all(f64::is_finite)
                || !Direction::ALL
                    .into_iter()
                    .any(|face| face.normal() == click.hit.face))
        {
            return Ok(None);
        }
        let [x, y, z] = click.pos;
        let selected = self.selected;
        let epoch = self.native_lifecycle.epoch();
        let current = |game: &Self| {
            game.side_has_remote()
                && game.native_lifecycle.epoch() == epoch
                && game.dim.level() == click.dimension
                && game.world.block(x, y, z) == state
                && match click.hand {
                    Hand::Main => game.selected == selected && game.stack_in(selected) == held,
                    Hand::Off => game.offhand.as_ref() == held,
                }
        };
        if predicting && !current(self) {
            return Err(StaleBlockClick);
        }
        let mut answer = None;
        dispatch(self, &mut answer);
        if predicting && !current(self) {
            return Err(StaleBlockClick);
        }
        Ok(answer)
    }

    /// Tells the mods of the chunks that came into this level.
    fn native_chunks_loaded(&mut self) {
        self.side_chunks_loaded();
    }
    /// Tells the mods of the chunks leaving this level, after their last save.
    fn native_chunks_unloaded(&mut self, unloaded: &[std::sync::Arc<sunrise_level::chunk::Chunk>]) {
        self.side_chunks_unloaded(unloaded);
    }
    /// `EntityBlock.newBlockEntity` of block `name`: Sunrise's own, else a
    /// native mod's, made as a write of the level creates it
    /// (`GameLevel::new_block_entity`). Only a declared mod type builds the
    /// level.
    #[cfg(any(test, feature = "test-support"))]
    fn new_block_entity(&mut self, name: &str) -> Option<BlockEntity> {
        if sunrise_game_data::block_entity_types::type_for(name).is_none() {
            return block_entity_for_block(
                self.pack.assets.registry,
                self.res.facts(),
                name,
                Compound::default(),
            );
        }
        self.with_level(|_, level, _| level.new_block_entity(name))
    }
    /// Compile static state-factory ownership once per level, including on clients.
    fn ensure_stateful_entity_table(&mut self) {
        if self.world.block_entities.has_stateful_entity_table() {
            return;
        }
        let registry = self.pack.assets.registry;
        let mut blocks = FxHashMap::default();
        let states = (0..registry.len() as u32)
            .map(|id| {
                let Some(state) = registry.state(BlockStateId(id)) else {
                    return false;
                };
                if !mod_api::__private::is_content_block(&state.name) {
                    return false;
                }
                *blocks.entry(state.name.as_str()).or_insert_with(|| {
                    let mut stateful = false;
                    self.native_stateful_block_entity(&state.name, &mut stateful);
                    stateful
                })
            })
            .collect();
        self.world.block_entities.set_stateful_entities(states);
    }
    /// Create a block entity from an actual state using the level's creation hook.
    fn new_block_entity_for_state(&mut self, state: BlockStateId) -> Option<BlockEntity> {
        self.with_level(|_, level, _| level.new_block_entity_for_state(state))
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> SharedGameExt1<'__sg> for G {}

/// A supported type for a block's container menu. A transient container is never block-backed.
pub fn mod_menu(name: &str) -> Option<(&'static str, usize)> {
    MOD_MENUS
        .into_iter()
        .find(|(menu, _)| *menu == name)
        .or_else(|| {
            sunrise_level::containers::mod_menu_type(name)
                .filter(|menu| menu.owned_slots.is_none())
                .map(|menu| (menu.id, menu.slots))
        })
}

/// `state` with its property `key` set to `value`; none when the block has
/// no such property or value.
pub fn with_property(
    registry: &BlockRegistry,
    state: BlockStateId,
    key: &str,
    value: &str,
) -> Option<BlockStateId> {
    let current = registry.state(state)?;
    if current.properties.get(key)? == value {
        return Some(state);
    }
    let mut s = current.clone();
    s.properties.insert(key.into(), value.into());
    registry.resolve(&s).ok()
}

/// The state a mod's property write changes `state` to; none when the block
/// has no such property or value, or already has it: `LevelChunk.setBlockState`
/// returns null for the same state, so `Level.setBlock` returns false.
pub fn property_change(
    registry: &BlockRegistry,
    state: BlockStateId,
    key: &str,
    value: &str,
) -> Option<BlockStateId> {
    with_property(registry, state, key, value).filter(|&id| id != state)
}

/// `config/` in the game folder, beside `options.txt`, made when missing.
pub fn config_dir(game_dir: &Path) -> std::io::Result<PathBuf> {
    let dir = game_dir.join("config");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn client_packet_view(
    packet: &sunrise_protocol::packets::play::Clientbound,
) -> ClientPacket<'_> {
    use mod_api::Identifier;
    use std::sync::LazyLock;
    use sunrise_protocol::packets::play::Clientbound;

    static TYPES: LazyLock<Vec<Identifier>> =
        LazyLock::new(|| packet_types(sunrise_protocol::ids::play::clientbound::NAMES));
    let payload = match packet {
        Clientbound::CustomPayload(payload) => Some(payload),
        _ => None,
    };
    packet_view(&TYPES, packet.id(), payload)
}

/// `client_packet_send`'s view of a packet the game sends: as `client_packet_view`.
pub fn serverbound_packet_view(
    packet: &sunrise_protocol::packets::play::Serverbound,
) -> ClientPacket<'_> {
    use mod_api::Identifier;
    use std::sync::LazyLock;
    use sunrise_protocol::packets::play::Serverbound;

    static TYPES: LazyLock<Vec<Identifier>> =
        LazyLock::new(|| packet_types(sunrise_protocol::ids::play::serverbound::NAMES));
    let payload = match packet {
        Serverbound::CustomPayload(payload) => Some(payload),
        _ => None,
    };
    packet_view(&TYPES, packet.id(), payload)
}

/// One direction's packet type identifiers, by protocol id.
pub fn packet_types(names: &[&str]) -> Vec<mod_api::Identifier> {
    names
        .iter()
        .map(|name| mod_api::Identifier::parse(name).expect("protocol packet type identifier"))
        .collect()
}

/// A packet's type, `sunrise:unknown` outside the protocol's table, and its custom payload.
pub fn packet_view<'a>(
    types: &'static [mod_api::Identifier],
    id: i32,
    payload: Option<&'a sunrise_protocol::packets::configuration::CustomPayload>,
) -> ClientPacket<'a> {
    use mod_api::Identifier;
    use std::sync::LazyLock;

    static UNKNOWN: LazyLock<Identifier> =
        LazyLock::new(|| Identifier::parse("sunrise:unknown").unwrap());
    let id = usize::try_from(id)
        .ok()
        .and_then(|id| types.get(id))
        .unwrap_or(&UNKNOWN);
    let payload = payload.map(|p| ClientPayload::new(p.channel.as_str(), p.data.as_slice()));
    ClientPacket::new(id, payload)
}

use crate::game::side_ops::events::EventsSideGameExt as _;

use crate::game::GameGameExt1 as _;

use crate::game::held_items::HeldItemsGameExt1 as _;

use crate::game::native_hooks::block_use_item_on::GameHooks as _;
use crate::game::native_hooks::block_use_without_item::GameHooks as _;
use crate::game::native_hooks::stateful_block_entity::GameHooks as _;
use crate::game::native_hooks::use_item_on_block::GameHooks as _;
