use crate::game::api_values::TryIntoApi;
use mod_api::ResidentBlockEntityReadOnly;
use sunrise_core::nbt::Compound;
use sunrise_level::containers::ModBlockEntity as EngineBlockEntity;

/// Shared read-only resident view backing for client and level-load block-entity scopes.
pub struct ResidentReadBlockEntity<'a> {
    pub entity: &'a EngineBlockEntity,
    pub slot_count: usize,
}

impl<'a> ResidentReadBlockEntity<'a> {
    /// Lend exactly the modeled slots in a stored row, even if it has left its level.
    pub fn stored(entity: &'a EngineBlockEntity) -> Self {
        Self {
            entity,
            slot_count: entity.items.len(),
        }
    }
}

impl ResidentBlockEntityReadOnly for ResidentReadBlockEntity<'_> {
    fn id(&self) -> &str {
        self.entity.id()
    }
    fn data_slots(&self) -> &[i32] {
        &self.entity.data
    }
    fn nbt(&self) -> &Compound {
        &self.entity.nbt
    }
    fn item_slots(&self) -> usize {
        self.slot_count
    }
    fn item_slot(&self, slot: u32) -> Option<(&str, u32)> {
        let index = slot as usize;
        if index >= self.slot_count {
            return None;
        }
        let stack = self.entity.items.get(index)?.as_ref()?;
        Some((&stack.item, stack.count))
    }
    fn copy_item_slot(&self, slot: u32) -> Option<mod_api::ItemStack> {
        self.entity
            .items
            .get(slot as usize)?
            .as_ref()?
            .try_into_api()
            .ok()
    }
}
