//! Admit only the staged component patch before a resolved slot is journaled or changed.
use super::content_declarations::validate_decoded_components;
use super::stack_count::{admit_stack_limits, stack_item_definition};
use super::{
    ApiStack, EngineStack, Items, decode_stack_record, projected_stack, stack_read_metadata,
};
use crate::game::native_equipment_read::with_equipment_view;
use mod_api::__private::game::{ComponentEdit, ItemComponentEdits, edit_item_components};
use mod_api::{ItemComponentEditor, ItemPlaceEditError, StackWriteError};
use sunrise_core::nbt::{Compound, JavaString, Tag};
use sunrise_sim::mobs::finalize::EquipmentRead;

const DAMAGE: &str = "minecraft:damage";
const REMOVED_DAMAGE: &str = "!minecraft:damage";
const ENCHANTMENTS: &str = "minecraft:enchantments";
const TRIM: &str = "minecraft:trim";

pub struct GearEdits {
    pub enchantments: bool,
    pub trim: bool,
}

pub struct PreparedComponentEdit {
    fields: EngineStack,
    edits: ItemComponentEdits,
}

impl PreparedComponentEdit {
    /// Write only admitted fields named by the pruned batch, not validation-only overrides.
    pub fn apply(self, resident: &mut EngineStack) {
        resident.apply_component_fields_from(self.fields, self.edits.names());
    }
}

/// Lend a lazy view of the resolved slot, then admit the net change without copying the slot.
pub fn prepare_component_edit(
    read: EquipmentRead<'_>,
    items: &Items,
    edit: &mut dyn for<'a> FnMut(&mut ItemComponentEditor<'a>),
) -> Result<Option<(PreparedComponentEdit, GearEdits)>, ItemPlaceEditError> {
    let resident = read.base();
    let metadata = stack_read_metadata(resident).map_err(|_| ItemPlaceEditError::NotHeld)?;
    // Check the original damage markers exactly as a full projection would. Do not lend
    // the editor scalars for a stack that the API cannot read.
    let original = projected_stack(metadata, read.extra_components_matching(|id| id == DAMAGE))
        .map_err(|_| ItemPlaceEditError::NotHeld)?;
    let original_damage = match original.damage_component() {
        Some(value) => ComponentEdit::Value(value),
        None if original.components().is_some() => ComponentEdit::Remove,
        None => ComponentEdit::Inherit,
    };
    let (item, count, _) = metadata;
    let mut edits = with_equipment_view(
        &read,
        (item, count, original.damage_component()),
        |view, read_override| edit_item_components(view, read_override, edit),
    );

    let compared = edits
        .components()
        .next()
        .and_then(|_| read.extra_components_matching(|id| edits.component(id).is_some()));
    edits.retain(
        |id, edit| {
            let current = compared.as_ref().and_then(|patch| patch.get(id));
            let removed = compared
                .as_ref()
                .and_then(|patch| patch.get(&format!("!{id}")));
            match edit {
                ComponentEdit::Value(value) => current != Some(value) || removed.is_some(),
                ComponentEdit::Remove => {
                    current.is_some()
                        || !matches!(removed, Some(Tag::Compound(marker)) if marker.is_empty())
                }
                ComponentEdit::Inherit => current.is_some() || removed.is_some(),
            }
        },
        |edit| edit != original_damage,
    );
    drop(compared);
    if edits.names().next().is_none() {
        return Ok(None);
    }

    let (id, definition) = stack_item_definition(item, items)?;
    // Keep only the resident scalars used for limit validation. Retained opaque components
    // use their live decode caches below, not a fresh trip through the NBT codec.
    let mut patch = match read.extra_components_matching(|name| {
        matches!(name, "minecraft:max_stack_size" | "minecraft:max_damage")
            && edits.component(name).is_none()
    }) {
        Some(Tag::Compound(patch)) => patch,
        _ => Compound::new(),
    };
    for (name, edit) in edits.components() {
        match edit {
            ComponentEdit::Value(value) => {
                patch.insert(JavaString::from(name), value.clone());
            }
            ComponentEdit::Remove => {
                patch.insert(
                    JavaString::from(format!("!{name}").as_str()),
                    Tag::Compound(Compound::new()),
                );
            }
            ComponentEdit::Inherit => (),
        }
    }
    let damage = match edits.damage().unwrap_or(original_damage) {
        ComponentEdit::Value(value) => Some(value),
        ComponentEdit::Remove => {
            patch.insert(
                JavaString::from(REMOVED_DAMAGE),
                Tag::Compound(Compound::new()),
            );
            None
        }
        ComponentEdit::Inherit => None,
    };
    let mut record = ApiStack::from_scalars(item, count, damage);
    if !patch.is_empty() {
        record
            .set_components(patch)
            .map_err(|reason| StackWriteError::InvalidComponents { reason })?;
    }
    let mut fields = decode_stack_record(&record, id.clone());
    validate_decoded_components(|name| match edits.component(name) {
        Some(ComponentEdit::Value(_)) => Some(&fields),
        _ => None,
    })?;
    admit_stack_limits(&mut fields, definition, record.damage_component())?;

    let gear = GearEdits {
        enchantments: edits.component(ENCHANTMENTS).is_some(),
        trim: edits.component(TRIM).is_some(),
    };
    Ok(Some((PreparedComponentEdit { fields, edits }, gear)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_edit_canonicalizes_positive_short_max_damage() {
        let items = crate::game::api_values::stack_limit_tests::catalog();
        let definition = items.items.get("minecraft:iron_sword").unwrap();
        let mut resident = EngineStack::new("minecraft:iron_sword", 1);
        let (prepared, _) =
            prepare_component_edit(EquipmentRead::resident(&resident), &items, &mut |editor| {
                editor
                    .apply_components(
                        &[(JavaString::from("minecraft:max_damage"), Tag::Short(20))].into(),
                    )
                    .unwrap();
            })
            .unwrap()
            .expect("max_damage edit should be staged");
        prepared.apply(&mut resident);
        assert_eq!(resident.max_damage(Some(definition)), 20);
        assert_eq!(
            resident
                .data()
                .and_then(|data| data.other("minecraft:max_damage")),
            Some(&Tag::Int(20)),
        );

        let zero =
            prepare_component_edit(EquipmentRead::resident(&resident), &items, &mut |editor| {
                editor
                    .apply_components(
                        &[(JavaString::from("minecraft:max_damage"), Tag::Short(0))].into(),
                    )
                    .unwrap();
            });
        assert!(matches!(
            zero,
            Err(ItemPlaceEditError::Stack(StackWriteError::InvalidMaximum))
        ));
    }

    #[test]
    fn refused_registered_effect_edit_keeps_the_resident_component() {
        use std::sync::Arc;
        use sunrise_world::component_value::consume_effects::{self, Type};

        consume_effects::with_test_registry(|| {
            consume_effects::install(
                [(
                    Arc::from("effect_edit:charge"),
                    Type {
                        schema: 0,
                        load: Box::new(|fields| {
                            if !matches!(fields.get(&"points".into()), Some(Tag::Int(_))) {
                                return Err("points must be an int".into());
                            }
                            Ok((fields.clone(), Vec::new()))
                        }),
                        read: Box::new(|_, _| unreachable!("no stream used in this test")),
                    },
                )]
                .into(),
            )
            .unwrap();
            let items = crate::game::api_values::stack_limit_tests::catalog();
            let component = |list, points| {
                Tag::compound([(
                    list,
                    Tag::List(vec![
                        Tag::compound([
                            ("type", Tag::String("effect_edit:charge".into())),
                            ("points", points),
                        ]),
                        Tag::compound([(
                            "type",
                            Tag::String("minecraft:clear_all_effects".into()),
                        )]),
                    ]),
                )])
            };
            let edit = |resident: &EngineStack, id: &str, value: Tag| {
                prepare_component_edit(EquipmentRead::resident(resident), &items, &mut |editor| {
                    editor
                        .apply_components(&[(JavaString::from(id), value.clone())].into())
                        .unwrap();
                })
            };
            for id in [
                "minecraft:consumable",
                "minecraft:bundle_contents",
                "minecraft:charged_projectiles",
                "minecraft:container",
                "minecraft:pot_decorations",
            ] {
                let value = |points| {
                    if id == "minecraft:consumable" {
                        return component("on_consume_effects", points);
                    }
                    let template = Tag::compound([
                        ("id", Tag::String("minecraft:stone".into())),
                        (
                            "components",
                            Tag::compound([(
                                "minecraft:death_protection",
                                component("death_effects", points),
                            )]),
                        ),
                    ]);
                    match id {
                        "minecraft:bundle_contents" | "minecraft:charged_projectiles" => {
                            Tag::List(vec![template])
                        }
                        "minecraft:container" => Tag::List(vec![Tag::compound([
                            ("slot", Tag::Int(0)),
                            ("item", template),
                        ])]),
                        _ => Tag::compound([("back", template)]),
                    }
                };
                let mut resident = EngineStack::new("minecraft:stick", 1);
                let (prepared, _) = edit(&resident, id, value(Tag::Int(7))).unwrap().unwrap();
                prepared.apply(&mut resident);
                let original = sunrise_items::item_data::component_snapshot(&resident).unwrap();
                assert_eq!(original.get(id), Some(&value(Tag::Int(7))), "{id}");
                assert!(
                    matches!(
                        edit(&resident, id, value(Tag::String("bad".into()))),
                        Err(ItemPlaceEditError::Stack(
                            StackWriteError::InvalidEffectPayload
                        ))
                    ),
                    "{id}"
                );
                assert_eq!(
                    sunrise_items::item_data::component_snapshot(&resident),
                    Some(original)
                );
            }
        });
    }
}
