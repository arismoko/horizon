//! Saved data from older versions upgraded for mods: one block entity's or entity's NBT through the datafixers the
//! chunk loaders run (`upgrade_block_entity_nbt`, `upgrade_entity_nbt`).
use super::Game;
use horizon_core::nbt::Compound;
use horizon_world::datafix;

impl Game<'_> {
    /// A block entity's NBT saved at `data_version`, upgraded as the chunk loader upgrades it, or the refusal.
    pub(in crate::game) fn upgrade_block_entity_nbt(
        &self,
        data_version: i32,
        nbt: Compound,
    ) -> Result<Compound, String> {
        datafix::upgrade_block_entity(data_version, nbt).map_err(|error| error.to_string())
    }

    /// An entity's NBT saved at `data_version`, upgraded as the entity chunk loader upgrades it, or the refusal.
    pub(in crate::game) fn upgrade_entity_nbt(
        &self,
        data_version: i32,
        nbt: Compound,
    ) -> Result<Compound, String> {
        datafix::upgrade_entity(data_version, nbt).map_err(|error| error.to_string())
    }
}
