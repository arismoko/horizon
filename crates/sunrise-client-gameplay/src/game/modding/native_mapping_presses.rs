//! The live vanilla mapping slots shared by device input, ClientHost and the keybind tick.
use crate::game::ClientGame;
use mod_api::{MappingError, VanillaKeyMappingRef};
use sunrise_client::keybinds::VanillaMappingSlot;
use sunrise_client::keymapping::toggle_index;
use winit::keyboard::KeyCode;

impl ClientGame<'_> {
    pub fn set_mapping_held(&mut self, name: &str, down: bool) {
        if let Some(slot) = VanillaMappingSlot::by_name(name) {
            self.client.vanilla_mappings.state_mut(slot).is_down = down;
        }
    }

    pub fn queue_mapping_click(&mut self, name: &str) {
        if let Some(slot) = VanillaMappingSlot::by_name(name) {
            self.client.vanilla_mappings.click(slot);
        }
    }

    pub(in crate::game::client_side) fn consume_mapping_click(&mut self, name: &str) -> bool {
        VanillaMappingSlot::by_name(name)
            .is_some_and(|slot| self.client.vanilla_mappings.consume_click(slot))
    }

    pub fn pending_mapping_clicks(&self, name: &str) -> u32 {
        VanillaMappingSlot::by_name(name).map_or(0, |slot| {
            self.client.vanilla_mappings.pending_clicks(slot) as u32
        })
    }

    /// An explicit write always reaches its mapping, including behind a screen during ClientSetup.
    pub(in crate::game) fn vanilla_key_mapping_set_down(
        &mut self,
        mapping: VanillaKeyMappingRef,
        down: bool,
    ) -> Result<(), MappingError> {
        let slot = self.vanilla_mapping_slot(mapping)?;
        let enabled = toggle_index(slot).is_some_and(|i| self.toggle_options()[i]);
        self.client.vanilla_mappings.set_down(slot, down, enabled);
        if let ("movement", Some(Ok(key))) = (slot.definition().2, slot.default_input()) {
            // The analog stick (which does not generate digital key presses) also honors
            // explicit writes until a new edge for this direction.
            if matches!(
                key,
                KeyCode::KeyW | KeyCode::KeyS | KeyCode::KeyA | KeyCode::KeyD
            ) {
                self.client.keys.override_down(key, down);
            }
        }
        Ok(())
    }

    /// Record one click. Only the keybind phase decides whether and when it has an action.
    pub(in crate::game) fn vanilla_key_mapping_click(
        &mut self,
        mapping: VanillaKeyMappingRef,
    ) -> Result<(), MappingError> {
        let slot = self.vanilla_mapping_slot(mapping)?;
        self.client.vanilla_mappings.click(slot);
        Ok(())
    }
}
