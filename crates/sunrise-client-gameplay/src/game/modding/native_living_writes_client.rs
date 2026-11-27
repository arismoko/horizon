use super::commands_common::Target;
use mod_api::LivingRef;

impl crate::game::ClientGame<'_> {
    pub(in crate::game) fn client_living_read_target(&self, living: LivingRef) -> Option<Target> {
        match self.native_client_entity_target(living.entity())? {
            Target::Player => Some(Target::Player),
            _ => None,
        }
    }

    pub(in crate::game) fn client_living_target_alive(&self, _id: u32) -> bool {
        false
    }

    pub(in crate::game) fn client_living_absorption(&self, living: LivingRef) -> Option<f32> {
        let Target::Player = self.client_living_read_target(living)? else {
            return None;
        };
        Some(self.common.player.as_ref()?.health.absorption())
    }

    pub(in crate::game) fn client_living_attribute_value(
        &self,
        living: LivingRef,
        id: &str,
    ) -> Option<f64> {
        let Target::Player = self.client_living_read_target(living)? else {
            return None;
        };
        let player = self.common.player.as_ref()?;
        let key = player.attributes.key(id)?;
        player
            .attributes
            .value(Some(key), sunrise_world::attribute_map::ValueKind::Computed)
            .ok()
    }

    pub(in crate::game) fn client_living_equipment_stack(
        &self,
        living: LivingRef,
        slot: mod_api::EquipmentSlot,
    ) -> Option<sunrise_sim::mobs::finalize::EquipmentRead<'_>> {
        use mod_api::EquipmentSlot;
        let Target::Player = self.client_living_read_target(living)? else {
            return None;
        };
        let stack = match slot {
            EquipmentSlot::MainHand => self.common.inventory.get(self.common.selected)?.as_ref(),
            EquipmentSlot::OffHand => self.common.offhand.as_ref(),
            EquipmentSlot::Head => self.common.armor[0].as_ref(),
            EquipmentSlot::Chest => self.common.armor[1].as_ref(),
            EquipmentSlot::Legs => self.common.armor[2].as_ref(),
            EquipmentSlot::Feet => self.common.armor[3].as_ref(),
            EquipmentSlot::Body | EquipmentSlot::Saddle => self.common.equipment.get(slot as usize),
        }?;
        Some(sunrise_sim::mobs::finalize::EquipmentRead::resident(stack))
    }

    pub(in crate::game) fn client_living_use_ticks(
        &self,
        living: LivingRef,
    ) -> Option<Option<u32>> {
        let Target::Player = self.client_living_read_target(living)? else {
            return None;
        };
        Some(self.common.using.map(|(_, ticks)| ticks))
    }

    pub(in crate::game) fn client_living_used_item_hand(
        &self,
        living: LivingRef,
    ) -> Option<mod_api::client::first_person::Hand> {
        let Target::Player = self.client_living_read_target(living)? else {
            return None;
        };
        self.common.used_hand()
    }

    pub(in crate::game) fn client_item_combat_actor(
        &self,
        actor: LivingRef,
    ) -> Option<(bool, bool, f64, bool)> {
        let Target::Player = self.client_living_read_target(actor)? else {
            return None;
        };
        let p = self.common.player.as_ref()?;
        Some((true, self.common.creative, p.fall_distance, p.fall_flying))
    }
}
