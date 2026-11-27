//! The item model's surroundings (`item_model::Context`) passed to bound item model providers
//! (`register_item_models`, `register_range_select_item_model_property`,
//! `register_select_item_model_property`): where the item is drawn, its owner, and its level.
use mod_api::{
    BlockPos, GlobalPos, Identifier, ItemDisplayContext, ItemModelLevel, ItemModelOwner,
};
use sunrise_assets::item_model::{
    DisplayContext, GlobalPos as EngineGlobalPos, LevelState, OwnerState,
};

pub(in crate::game) fn display_context(
    display: &DisplayContext,
) -> Result<ItemDisplayContext, std::convert::Infallible> {
    Ok(match display {
        DisplayContext::None => ItemDisplayContext::None,
        DisplayContext::ThirdPersonLeftHand => ItemDisplayContext::ThirdPersonLeftHand,
        DisplayContext::ThirdPersonRightHand => ItemDisplayContext::ThirdPersonRightHand,
        DisplayContext::FirstPersonLeftHand => ItemDisplayContext::FirstPersonLeftHand,
        DisplayContext::FirstPersonRightHand => ItemDisplayContext::FirstPersonRightHand,
        DisplayContext::Head => ItemDisplayContext::Head,
        DisplayContext::Gui => ItemDisplayContext::Gui,
        DisplayContext::Ground => ItemDisplayContext::Ground,
        DisplayContext::Fixed => ItemDisplayContext::Fixed,
        DisplayContext::OnShelf => ItemDisplayContext::OnShelf,
    })
}

/// A position in a dimension; `None` for a dimension key that isn't an identifier.
fn global_pos(pos: &EngineGlobalPos) -> Option<GlobalPos> {
    let [x, y, z] = pos.pos;
    Some(GlobalPos::new(
        Identifier::parse(&pos.dimension).ok()?,
        BlockPos::new(x, y, z),
    ))
}

/// The entity type isn't an identifier.
pub(in crate::game) fn owner_state(owner: &OwnerState) -> Result<ItemModelOwner, ()> {
    Ok(ItemModelOwner {
        position: owner.position,
        visual_rotation_y_degrees: owner.visual_rotation_y_degrees,
        entity_type: Identifier::parse(&owner.entity_type).map_err(|_| ())?,
        main_arm_left: owner.main_arm_left,
        using_this_item: owner.using_this_item,
        use_item_remaining_ticks: owner.use_item_remaining_ticks,
        use_duration: owner.use_duration,
        crossbow_charge_duration: owner.crossbow_charge_duration,
        fishing_cast_with_this_item: owner.fishing_cast_with_this_item,
        is_selected: owner.is_selected,
        is_carried: owner.is_carried,
        is_view_entity: owner.is_view_entity,
        team_color: owner.team_color,
        cooldown_percent: owner.cooldown_percent,
        is_local_player: owner.is_local_player,
        last_death_location: owner.last_death_location.as_ref().and_then(global_pos),
    })
}

/// The dimension key isn't an identifier.
pub(in crate::game) fn level_state(level: &LevelState) -> Result<ItemModelLevel, ()> {
    Ok(ItemModelLevel {
        dimension: Identifier::parse(&level.dimension).map_err(|_| ())?,
        game_time: level.game_time,
        sun_angle: level.sun_angle,
        moon_phase: level.moon_phase,
        respawn: level.respawn.as_ref().and_then(global_pos),
        runs_normally: level.runs_normally,
    })
}
