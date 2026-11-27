//! Callbacks into the executable for native trait mods' client-library hooks.
use crate::{health::LivingLend, items::ItemStack, motion::FrictionLevel};
use mod_api::{BlockSoundCause, BlockSoundType, MenuOpeningItem};
use std::sync::OnceLock;
use sunrise_core::position::BlockPos;
use sunrise_world::native_load_context::LoopContext;
use sunrise_world::registry::BlockStateId;

type MenuValidity =
    fn(&LoopContext, u128, &str, Option<[i32; 3]>, [f64; 3], Option<MenuOpeningItem>, &mut bool);

/// Client-library hooks, installed before any menu, entity or recipe is used.
pub struct NativeClientHooks {
    pub menu_still_valid: MenuValidity,
    pub menu_still_valid_listens: fn(&str) -> bool,
    pub living_heal: fn(&LoopContext, LivingLend<'_>, &mut f32, &mut bool),
    pub living_breathe_heard: fn() -> bool,
    pub has_mods: fn() -> bool,
    pub player_name_format: fn(&LoopContext, &crate::text::Text, &mut crate::text::Text),
    pub player_tab_list_name_format: fn(&LoopContext, u128, &str, &mut Option<crate::text::Text>),
    pub living_drown: fn(&LoopContext, u32, &str, &mut bool, &mut f32, &mut i32, &mut bool),
    pub living_fall: fn(&LoopContext, u32, &str, &mut f64, &mut f32, &mut bool),
    pub living_jump: fn(&LoopContext, u32, &str, [f64; 3]),
    pub living_jump_power: fn(&LoopContext, LivingLend<'_>, &mut f32),
    pub living_speed: fn(&LoopContext, LivingLend<'_>, &mut f32),
    pub living_flying_speed: fn(&LoopContext, LivingLend<'_>, &mut f32),
    pub block_jump_factor: fn(&LoopContext, BlockStateId, &mut f32),
    pub block_speed_factor: fn(&LoopContext, BlockStateId, &mut f32),
    pub is_ladder: fn(&LoopContext, u32, BlockPos, BlockStateId, &mut bool),
    pub block_interaction_range: fn(&LoopContext, u128, BlockPos, f64, &mut bool),
    pub makes_open_trapdoor_above_climbable:
        fn(&LoopContext, BlockStateId, BlockPos, BlockStateId, &mut bool),
    pub friction:
        fn(FrictionLevel<'_>, Option<(u32, &str)>, BlockPos, BlockStateId, &mut f32) -> bool,
    /// `BlockPart::collision_shape` of the state's block, for one collision query; `shape`
    /// arrives `None` and leaves checked.
    pub collision_shape: fn(
        &LoopContext,
        BlockPos,
        BlockStateId,
        &mod_api::CollisionQuery,
        &mut Option<sunrise_core::geometry::VoxelShape>,
    ),
    pub player_stays_on_ground_surface: fn(&LoopContext, u32, &mut bool),
    pub ender_entity_teleport: fn(&LoopContext, u32, &str, [f64; 3], &mut [f64; 3], &mut bool),
    pub fluid_flow: fn(&LoopContext, &str, BlockPos, &mut [f64; 3]),
    pub sound_type: fn(
        FrictionLevel<'_>,
        BlockPos,
        BlockStateId,
        Option<(u32, &str)>,
        BlockSoundCause,
        &BlockSoundType,
    ) -> Option<BlockSoundType>,
    /// `ItemPart::can_grindstone_repair` of the stack's item; `can_repair` arrives with
    /// vanilla's.
    pub can_grindstone_repair: fn(&LoopContext, &ItemStack, &mut bool),
    /// `ItemPart::is_combine_repairable` of the stack's item; `repairable` arrives with vanilla's.
    pub is_combine_repairable: fn(&LoopContext, &ItemStack, &mut bool),
    /// `ItemPart::can_fit_inside_container_items` of the stack's item; `fits` arrives with
    /// vanilla's.
    pub can_fit_inside_container_items: fn(&LoopContext, &ItemStack, &mut bool),
    /// `ItemPart::item_name` of the stack's item; `name` arrives as `None`, the default.
    pub item_name: fn(&LoopContext, &ItemStack, &mut Option<crate::text::Text>),
}

static HOOKS: OnceLock<NativeClientHooks> = OnceLock::new();

#[cfg(feature = "test-menu-validity")]
mod menu_validity_fixture {
    use super::MenuValidity;
    use std::{cell::Cell, marker::PhantomData, rc::Rc};

    type MenuValidityHooks = (MenuValidity, fn(&str) -> bool);
    thread_local! {
        pub(super) static HOOKS: Cell<Option<MenuValidityHooks>> = const { Cell::new(None) };
    }

    struct Restore(Option<MenuValidityHooks>, PhantomData<Rc<()>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            HOOKS.set(self.0);
        }
    }

    /// Use these callbacks on this thread until the returned guard is dropped.
    pub fn scoped_menu_validity_hooks(ask: MenuValidity, listens: fn(&str) -> bool) -> impl Drop {
        Restore(HOOKS.replace(Some((ask, listens))), PhantomData)
    }
}
#[cfg(feature = "test-menu-validity")]
pub use menu_validity_fixture::scoped_menu_validity_hooks;

#[cfg(test)]
#[path = "native_client_hooks_tests.rs"]
pub(crate) mod test_hooks;

/// Install the process-wide callbacks once; a second installation keeps the first and answers false.
pub fn install(hooks: NativeClientHooks) -> bool {
    HOOKS.set(hooks).is_ok()
}

/// Ask the installed adapter for a mod menu type's `stillValid` answer; with
/// none installed, `valid` keeps the game's answer.
pub(crate) fn menu_still_valid(
    context: &LoopContext,
    player: u128,
    menu: &str,
    pos: Option<[i32; 3]>,
    eye: [f64; 3],
    opening_item: Option<MenuOpeningItem>,
    valid: &mut bool,
) {
    #[cfg(feature = "test-menu-validity")]
    if let Some((ask, _)) = menu_validity_fixture::HOOKS.get() {
        ask(context, player, menu, pos, eye, opening_item, valid);
        return;
    }
    if let Some(hooks) = HOOKS.get() {
        (hooks.menu_still_valid)(context, player, menu, pos, eye, opening_item, valid);
    }
}

/// Whether a part on the menu type `menu` answers `menu_still_valid`.
pub(crate) fn menu_still_valid_listens(menu: &str) -> bool {
    #[cfg(feature = "test-menu-validity")]
    if let Some((_, listens)) = menu_validity_fixture::HOOKS.get() {
        return listens(menu);
    }
    HOOKS
        .get()
        .is_some_and(|hooks| (hooks.menu_still_valid_listens)(menu))
}

pub(crate) fn living_heal(
    context: &LoopContext,
    healed: LivingLend<'_>,
    amount: &mut f32,
    cancel: &mut bool,
) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.living_heal)(context, healed, amount, cancel);
    }
}

pub(crate) fn living_breathe_heard() -> bool {
    HOOKS
        .get()
        .is_some_and(|hooks| (hooks.living_breathe_heard)())
}

pub(crate) fn has_mods() -> bool {
    HOOKS.get().is_some_and(|hooks| (hooks.has_mods)())
}

pub(crate) fn player_name_format(
    context: &LoopContext,
    username: &crate::text::Text,
    displayname: &mut crate::text::Text,
) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.player_name_format)(context, username, displayname);
    }
}

pub(crate) fn player_tab_list_name_format(
    context: &LoopContext,
    player: u128,
    name: &str,
    display_name: &mut Option<crate::text::Text>,
) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.player_tab_list_name_format)(context, player, name, display_name);
    }
}

pub(crate) fn living_drown(
    context: &LoopContext,
    entity: u32,
    kind: &str,
    is_drowning: &mut bool,
    damage_amount: &mut f32,
    bubble_count: &mut i32,
    cancel: &mut bool,
) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.living_drown)(
            context,
            entity,
            kind,
            is_drowning,
            damage_amount,
            bubble_count,
            cancel,
        );
    }
}

pub(crate) fn living_fall(
    context: &LoopContext,
    entity: u32,
    kind: &str,
    distance: &mut f64,
    multiplier: &mut f32,
    cancel: &mut bool,
) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.living_fall)(context, entity, kind, distance, multiplier, cancel);
    }
}

pub(crate) fn living_jump(context: &LoopContext, entity: u32, kind: &str, velocity: [f64; 3]) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.living_jump)(context, entity, kind, velocity);
    }
}

pub(crate) fn living_jump_power(context: &LoopContext, living: LivingLend<'_>, power: &mut f32) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.living_jump_power)(context, living, power);
    }
}

pub(crate) fn living_speed(context: &LoopContext, living: LivingLend<'_>, speed: &mut f32) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.living_speed)(context, living, speed);
    }
}

pub(crate) fn living_flying_speed(context: &LoopContext, living: LivingLend<'_>, speed: &mut f32) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.living_flying_speed)(context, living, speed);
    }
}

pub(crate) fn block_jump_factor(context: &LoopContext, state: BlockStateId, factor: &mut f32) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.block_jump_factor)(context, state, factor);
    }
}

pub(crate) fn block_speed_factor(context: &LoopContext, state: BlockStateId, factor: &mut f32) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.block_speed_factor)(context, state, factor);
    }
}

pub(crate) fn is_ladder(
    context: &LoopContext,
    entity: u32,
    pos: BlockPos,
    state: BlockStateId,
    ladder: &mut bool,
) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.is_ladder)(context, entity, pos, state, ladder);
    }
}

pub(crate) fn block_interaction_range(
    context: &LoopContext,
    player: u128,
    pos: BlockPos,
    buffer: f64,
    within: &mut bool,
) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.block_interaction_range)(context, player, pos, buffer, within);
    }
}

pub(crate) fn makes_open_trapdoor_above_climbable(
    context: &LoopContext,
    state: BlockStateId,
    pos: BlockPos,
    trapdoor: BlockStateId,
    climbable: &mut bool,
) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.makes_open_trapdoor_above_climbable)(context, state, pos, trapdoor, climbable);
    }
}

pub(crate) fn friction(
    level: FrictionLevel<'_>,
    entity: Option<(u32, &str)>,
    pos: BlockPos,
    state: BlockStateId,
    friction: &mut f32,
) -> bool {
    HOOKS
        .get()
        .is_none_or(|hooks| (hooks.friction)(level, entity, pos, state, friction))
}

pub(crate) fn collision_shape(
    context: &LoopContext,
    pos: BlockPos,
    state: BlockStateId,
    query: &mod_api::CollisionQuery,
    shape: &mut Option<sunrise_core::geometry::VoxelShape>,
) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.collision_shape)(context, pos, state, query, shape);
    }
}

pub(crate) fn player_stays_on_ground_surface(
    context: &LoopContext,
    entity: u32,
    staying: &mut bool,
) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.player_stays_on_ground_surface)(context, entity, staying);
    }
}

pub(crate) fn ender_entity_teleport(
    context: &LoopContext,
    entity: u32,
    kind: &str,
    from: [f64; 3],
    target: &mut [f64; 3],
    cancel: &mut bool,
) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.ender_entity_teleport)(context, entity, kind, from, target, cancel);
    }
}

pub(crate) fn fluid_flow(context: &LoopContext, fluid: &str, pos: BlockPos, flow: &mut [f64; 3]) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.fluid_flow)(context, fluid, pos, flow);
    }
}

pub(crate) fn sound_type(
    level: FrictionLevel<'_>,
    pos: BlockPos,
    state: BlockStateId,
    entity: Option<(u32, &str)>,
    cause: BlockSoundCause,
    own: &BlockSoundType,
) -> Option<BlockSoundType> {
    let hooks = HOOKS.get()?;
    (hooks.sound_type)(level, pos, state, entity, cause, own)
}

/// Ask the installed adapter for the name the parts of `stack`'s item give it (`item_name`); with
/// none installed, `name` stays the default.
pub(crate) fn item_name(
    context: &LoopContext,
    stack: &ItemStack,
    name: &mut Option<crate::text::Text>,
) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.item_name)(context, stack, name);
    }
}

/// Ask the installed adapter for the parts of `stack`'s item (`can_grindstone_repair`); with none
/// installed, `can_repair` keeps vanilla's answer.
pub(crate) fn can_grindstone_repair(
    context: &LoopContext,
    stack: &ItemStack,
    can_repair: &mut bool,
) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.can_grindstone_repair)(context, stack, can_repair);
    }
}

/// Ask the installed adapter for the parts of `stack`'s item (`is_combine_repairable`); with none
/// installed, `repairable` keeps vanilla's answer.
pub(crate) fn is_combine_repairable(
    context: &LoopContext,
    stack: &ItemStack,
    repairable: &mut bool,
) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.is_combine_repairable)(context, stack, repairable);
    }
}

/// Ask the installed adapter for the parts of `stack`'s item (`can_fit_inside_container_items`);
/// with none installed, `fits` keeps vanilla's answer.
pub(crate) fn can_fit_inside_container_items(
    context: &LoopContext,
    stack: &ItemStack,
    fits: &mut bool,
) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.can_fit_inside_container_items)(context, stack, fits);
    }
}
