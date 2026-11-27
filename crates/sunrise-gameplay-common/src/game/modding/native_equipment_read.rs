use crate::game::api_values::{StackReadMetadata, projected_stack, stack_read_metadata};
use mod_api::client::first_person::Hand;
use mod_api::{EquipmentSlot, ItemStackView};
use sunrise_sim::mobs::finalize::EquipmentRead;

/// Lend one equipment slot to the API, applying overlays and encoding components only on demand.
pub fn lend_equipment(
    stack: EquipmentRead<'_>,
    read: &mut dyn for<'a> FnMut(ItemStackView<'a>),
) -> Option<()> {
    let metadata = stack_read_metadata(stack.base()).ok()?;
    with_equipment_view(&stack, metadata, |view, _| read(view));
    Some(())
}

/// Use checked slot metadata to lend the same lazy view to readers and component editors.
pub fn with_equipment_view<R>(
    stack: &EquipmentRead<'_>,
    metadata: StackReadMetadata,
    read: impl for<'a> FnOnce(ItemStackView<'a>, &'a dyn Fn(&str) -> Option<mod_api::Tag>) -> R,
) -> R {
    use sunrise_core::nbt::{JavaString, Tag};
    let read_override = |id: &str| {
        // The dye is read on the render path: its typed field and removal marker answer
        // directly, without encoding a patch compound to read one integer back.
        match id {
            "minecraft:dyed_color" => {
                return stack.base().extra.as_ref()?.dyed_color.map(Tag::Int);
            }
            "!minecraft:dyed_color" => {
                return stack.base().extra.as_ref()?.other(id).cloned();
            }
            _ => {}
        }
        let bare = id.strip_prefix('!').unwrap_or(id);
        let Tag::Compound(mut patch) = stack.extra_components_matching(|name| name == bare)? else {
            return None;
        };
        patch.remove(&JavaString::from(id))
    };
    let project =
        |metadata| projected_stack(metadata, stack.extra_components_matching(|_| true)).ok();
    read(
        ItemStackView::resident_with_components(metadata, &project, &read_override),
        &read_override,
    )
}

/// The equipment slot of a hand (`LivingEntity.getItemInHand`/`setItemInHand`).
pub fn hand_slot(hand: Hand) -> EquipmentSlot {
    match hand {
        Hand::Main => EquipmentSlot::MainHand,
        Hand::Off => EquipmentSlot::OffHand,
    }
}
