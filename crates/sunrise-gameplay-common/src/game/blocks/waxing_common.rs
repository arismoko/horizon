use crate::game::TransformKind;
use sunrise_world::registry::BlockStateId;

/// The use a tool ability makes outside the transform table: `new`, the ability's sound
/// and particle, no loot, one point of damage and no transform type.
pub(in crate::game) fn tool_ability_result(
    new: BlockStateId,
    ability: mod_api::ToolAbility,
) -> (
    BlockStateId,
    String,
    Option<String>,
    u32,
    String,
    TransformKind,
) {
    use mod_api::ToolAbility::*;
    let (sound, particle) = match ability {
        AxeStrip => ("minecraft:item.axe.strip", "none"),
        AxeScrape => ("minecraft:item.axe.scrape", "scrape"),
        AxeWaxOff => ("minecraft:item.axe.wax_off", "wax_off"),
        ShovelFlatten => ("minecraft:item.shovel.flatten", "none"),
        HoeTill => ("minecraft:item.hoe.till", "none"),
        ShovelDouse => unreachable!(),
        _ => unreachable!("ToolAbility: unhandled variant"),
    };
    (
        new,
        sound.to_owned(),
        None,
        1,
        particle.to_owned(),
        TransformKind::default(),
    )
}
