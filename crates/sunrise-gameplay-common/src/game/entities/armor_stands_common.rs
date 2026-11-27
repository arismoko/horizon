use sunrise_core::geometry::Aabb;
use sunrise_items::items::ItemStack;

pub const TYPE_ID: &str = "minecraft:armor_stand";

pub fn overlaps(a: Aabb, b: Aabb) -> bool {
    let (a0, a1, b0, b1) = (
        a.min_position(),
        a.max_position(),
        b.min_position(),
        b.max_position(),
    );
    a0.x < b1.x && b0.x < a1.x && a0.y < b1.y && b0.y < a1.y && a0.z < b1.z && b0.z < a1.z
}
/// What `Equippable.swapWithEquipmentSlot` leaves once it has passed its
/// checks: the piece worn in the slot, the hand, and the old piece bound for
/// the inventory.
#[derive(Debug, PartialEq)]
pub struct ArmorSwap {
    pub worn: ItemStack,
    pub hand: Option<ItemStack>,
    pub to_inventory: Option<ItemStack>,
}
/// `Equippable.swapWithEquipmentSlot` after its checks. A single item swaps
/// with the worn piece: the hand gets the worn piece (`copyAndClear`), or
/// keeps the item in creative when nothing was worn; creative wears a copy
/// (`copy` instead of `copyAndClear`). From a larger stack one item is worn
/// (`consumeAndReturn`, which takes none in creative) and the worn piece goes
/// to the inventory.
pub fn swap_with_equipment_slot(
    stack: &ItemStack,
    current: Option<ItemStack>,
    creative: bool,
) -> ArmorSwap {
    let one = ItemStack {
        count: 1,
        ..stack.clone()
    };
    if stack.count <= 1 {
        let hand = match current {
            Some(old) => Some(old),
            None if creative => Some(stack.clone()),
            None => None,
        };
        return ArmorSwap {
            worn: one,
            hand,
            to_inventory: None,
        };
    }
    let mut held = stack.clone();
    if !creative {
        held.count -= 1;
    }
    ArmorSwap {
        worn: one,
        hand: Some(held),
        to_inventory: current,
    }
}

/// `AbstractSkullBlock.getType` of a worn item: vanilla's skulls by item, then a native mod's
/// skull, named by the `minecraft:head` kind of its item model (`create_skull_models`), as the
/// game knows no mod block as a skull.
pub fn skull_kind<'a>(
    models: &'a sunrise_assets::item_model::ItemModels,
    item: &str,
) -> Option<&'a str> {
    skull_kind_among(models, item, sunrise_assets::item_model::is_skull_kind)
}
/// [`skull_kind`] with the registered skull kinds named by `registered`.
fn skull_kind_among<'a>(
    models: &'a sunrise_assets::item_model::ItemModels,
    item: &str,
    registered: impl Fn(&str) -> bool,
) -> Option<&'a str> {
    vanilla_skull_kind(item).or_else(|| models.head_kind(item).filter(|kind| registered(kind)))
}
/// `AbstractSkullBlock.getType` of vanilla's skull items.
fn vanilla_skull_kind(item: &str) -> Option<&'static str> {
    Some(match item {
        "minecraft:skeleton_skull" => "skeleton",
        "minecraft:wither_skeleton_skull" => "wither_skeleton",
        "minecraft:zombie_head" => "zombie",
        "minecraft:creeper_head" => "creeper",
        "minecraft:dragon_head" => "dragon",
        "minecraft:piglin_head" => "piglin",
        "minecraft:player_head" => "player",
        _ => return None,
    })
}

#[cfg(test)]
mod worn_skull_tests {
    use super::skull_kind_among;
    use serde_json::json;
    use std::collections::BTreeMap;
    use sunrise_assets::item_model::{
        ClientItem, ItemModelNode, ItemModels, SpecialKind, SpecialModel,
    };

    /// A worn skull's kind (#5969): vanilla's skulls by item, then a mod's skull by the
    /// `minecraft:head` kind its item model names, once a mod registered that kind
    /// (`create_skull_models`); a plain item, or a head model naming an unregistered kind, is
    /// no skull. The registered kinds are this test's own, not the process-wide table.
    #[test]
    fn a_mods_skull_item_resolves_to_its_registered_kind() {
        let registered = |kind: &str| kind == "worntest:goblin";
        // Decoding checks a head's kind against the process-wide table, so decode a vanilla
        // head and name the kind afterwards rather than register a mod kind for the process.
        let head = |named: &str| {
            let mut item = ClientItem::decode(&json!({"model": {
                "type": "minecraft:special",
                "base": "minecraft:item/template_skull",
                "model": {"type": "minecraft:head", "kind": "zombie"}
            }}))
            .unwrap();
            let ItemModelNode::Special {
                model:
                    SpecialModel {
                        kind: SpecialKind::Head { kind, .. },
                    },
                ..
            } = &mut item.model
            else {
                panic!("a head model");
            };
            *kind = named.to_owned();
            item
        };
        let prototype =
            |model: &str| BTreeMap::from([("minecraft:item_model".to_owned(), json!(model))]);
        let models = ItemModels {
            definitions: BTreeMap::from([
                ("worntest:goblin_head".to_owned(), head("worntest:goblin")),
                ("worntest:zombie_look".to_owned(), head("zombie")),
            ]),
            prototypes: std::sync::Arc::new(BTreeMap::from([
                (
                    "worntest:goblin_head".to_owned(),
                    prototype("worntest:goblin_head"),
                ),
                (
                    "worntest:zombie_look".to_owned(),
                    prototype("worntest:zombie_look"),
                ),
            ])),
        };
        assert_eq!(
            skull_kind_among(&models, "worntest:goblin_head", registered),
            Some("worntest:goblin")
        );
        assert_eq!(
            skull_kind_among(&models, "minecraft:zombie_head", registered),
            Some("zombie")
        );
        assert_eq!(
            skull_kind_among(&models, "minecraft:stick", registered),
            None
        );
        assert_eq!(
            skull_kind_among(&models, "worntest:zombie_look", registered),
            None,
            "a vanilla kind on a mod item is no registered mod skull"
        );
    }
}
