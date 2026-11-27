//! Stack count admission at the mod API conversion boundary.
use super::*;
use sunrise_items::items::ItemInfo;

/// Validate an output while retaining its count for a later placement policy.
/// Item identity, components, maxima and damage retain the ordinary write checks.
pub trait TryIntoEnginePreservingCount {
    type Engine;
    fn try_into_engine_preserving_count(
        &self,
        items: &Items,
    ) -> Result<Self::Engine, StackWriteError>;
}

impl TryIntoEnginePreservingCount for ApiStack {
    type Engine = EngineStack;

    fn try_into_engine_preserving_count(
        &self,
        items: &Items,
    ) -> Result<EngineStack, StackWriteError> {
        let mut single = self.clone();
        single
            .set_count(1)
            .map_err(|_| StackWriteError::EmptyEngineStack)?;
        let mut stack = single.try_into_engine(items)?;
        stack.count = self.count();
        Ok(stack)
    }
}

impl<T: TryIntoEnginePreservingCount> TryIntoEnginePreservingCount for [T] {
    type Engine = Vec<T::Engine>;

    fn try_into_engine_preserving_count(
        &self,
        items: &Items,
    ) -> Result<Self::Engine, StackWriteError> {
        self.iter()
            .map(|value| value.try_into_engine_preserving_count(items))
            .collect()
    }
}

impl TryIntoEngine for ApiStack {
    type Engine = EngineStack;
    type Error = StackWriteError;

    fn try_into_engine(&self, items: &Items) -> Result<EngineStack, Self::Error> {
        let (id, definition) = stack_item_definition(self.item(), items)?;
        let mut stack = encode_stack(self, id.clone())?;
        admit_stack_limits(&mut stack, definition, self.damage_component())?;
        Ok(stack)
    }
}

/// Resolve an API item to the registry's shared identity and definition.
pub(in crate::game) fn stack_item_definition(
    item: mod_api::ItemKey,
    items: &Items,
) -> Result<(&Arc<str>, &ItemInfo), StackWriteError> {
    let id = mod_api::__private::item_key_id(item).ok_or(StackWriteError::UnknownItem)?;
    items
        .items
        .get_key_value(id)
        .ok_or(StackWriteError::UnknownItem)
}

/// Check a decoded stack's effective limits and canonicalize a max-damage override.
/// Pass explicit damage before signed narrowing; `None` reads effective damage from the stack
/// and item definition.
pub(in crate::game) fn admit_stack_limits(
    stack: &mut EngineStack,
    definition: &ItemInfo,
    damage: Option<u32>,
) -> Result<(), StackWriteError> {
    let Some(maximum @ 1..=99) = stack.effective_max_stack_size(Some(definition)) else {
        return Err(StackWriteError::InvalidMaximum);
    };
    let max_damage = "minecraft:max_damage";
    let carries_max_damage = stack.carries_component(Some(definition), max_damage);
    if !sunrise_world::item_template::durability_allows_stack_size(carries_max_damage, maximum) {
        return Err(StackWriteError::StackableDamageable);
    }
    let maximum = maximum as u32; // The range match above proves this fits.
    if stack.count > maximum {
        return Err(StackWriteError::CountExceeded { maximum });
    }
    if carries_max_damage {
        match stack.data().and_then(|data| data.other(max_damage)) {
            Some(raw) => {
                let decoded = sunrise_core::nbt::numeric_int(raw)
                    .filter(|&value| value > 0)
                    .ok_or(StackWriteError::InvalidMaximum)?;
                if !matches!(raw, Tag::Int(_)) {
                    stack
                        .data_mut()
                        .set_other(max_damage, Some(Tag::Int(decoded)));
                }
            }
            None if !(1..=i32::MAX as u32).contains(&definition.max_damage) => {
                return Err(StackWriteError::InvalidMaximum);
            }
            None => {}
        }
    }
    let damage = damage.map_or_else(
        || {
            u32::try_from(stack.effective_damage(Some(definition))).map_err(|_| {
                StackWriteError::InvalidComponents {
                    reason: mod_api::StackError::InvalidDamage,
                }
            })
        },
        Ok,
    )?;
    if i32::try_from(damage).is_err() {
        return Err(StackWriteError::InvalidComponents {
            reason: mod_api::StackError::InvalidDamage,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_counts_but_keeps_other_write_validation() {
        let items = crate::game::api_values::stack_limit_tests::catalog();
        let key = |name| mod_api::__private::item_key(name).unwrap();
        let oversized = ApiStack::new(key("minecraft:stick"), u32::MAX).unwrap();
        assert_eq!(
            oversized
                .try_into_engine_preserving_count(&items)
                .unwrap()
                .count,
            u32::MAX
        );
        assert_eq!(
            oversized.try_into_engine(&items),
            Err(StackWriteError::CountExceeded { maximum: 64 })
        );
        let mut sword = ApiStack::new(key("minecraft:iron_sword"), 100).unwrap();
        sword.set_damage(11);
        let preserved = sword.try_into_engine_preserving_count(&items).unwrap();
        assert_eq!((preserved.count, preserved.damage), (100, 11));
        sword.set_damage(u32::MAX);
        assert_eq!(
            sword.try_into_engine_preserving_count(&items),
            Err(StackWriteError::InvalidComponents {
                reason: mod_api::StackError::InvalidDamage,
            })
        );
        let mut zero_max_damage = ApiStack::new(key("minecraft:iron_sword"), 100).unwrap();
        zero_max_damage
            .set_components([(JavaString::from("minecraft:max_damage"), Tag::Int(0))].into())
            .unwrap();
        assert_eq!(
            zero_max_damage.try_into_engine_preserving_count(&items),
            Err(StackWriteError::InvalidMaximum)
        );
        let mut malformed = oversized;
        malformed
            .set_components([(JavaString::from("minecraft:max_stack_size"), Tag::Int(-1))].into())
            .unwrap();
        assert_eq!(
            malformed.try_into_engine_preserving_count(&items),
            Err(StackWriteError::InvalidMaximum)
        );
    }
}
