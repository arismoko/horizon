use super::super::Result;
use super::respawn_attributes_state::{Carried, RespawnAttributes};
use sunrise_protocol::packets::play::AttributeSnapshot;
use sunrise_sim::{lan::attributes::apply, motion::PlayerMotion};

impl crate::game::ClientGame<'_> {
    /// `ClientPacketListener.handleUpdateAttributes` of the player's own entity.
    pub(in crate::game::client_side) fn apply_own_attributes(
        &mut self,
        values: Vec<AttributeSnapshot>,
    ) {
        if let Some(player) = self.common.player.as_mut() {
            apply(&mut player.attributes, &values);
        } else if let Some(carried) = self.client.respawn_attributes.carried.as_mut() {
            carried.early.push(values);
        }
    }
    /// The old player goes (`handleRespawn`, `handleLogin`): after a respawn its attributes are
    /// kept for the next one; a login starts from nothing.
    pub fn drop_remote_player(&mut self) {
        let respawn = &mut self.client.respawn_attributes;
        match (respawn.keep_modifiers.take(), self.common.player.as_ref()) {
            (Some(keep_modifiers), Some(old)) => {
                respawn.carried = Some(Carried {
                    keep_modifiers,
                    instances: old
                        .attributes
                        .instances()
                        .map(|(_, key, instance)| (key.name().to_owned(), instance.clone()))
                        .collect(),
                    early: Vec::new(),
                });
            }
            // A second respawn before the position: the first one's attributes are still the
            // ones to carry.
            (Some(_), None) => {}
            (None, _) => respawn.carried = None,
        }
        self.common.equipment = Default::default();
        self.remove_local_player_body();
    }
    /// The new player of a respawn takes the old one's attributes, then what the server sent.
    pub(in crate::game::client_side) fn take_carried_attributes(
        &mut self,
        player: &mut PlayerMotion,
    ) -> Result<()> {
        let Some(carried) = self.client.respawn_attributes.carried.take() else {
            return Ok(());
        };
        for (name, old) in &carried.instances {
            let Some(key) = player.attributes.key(name) else {
                continue;
            };
            let Some(handle) = player
                .attributes
                .get_instance(Some(key))
                .map_err(|e| format!("movement attribute: {e:?}"))?
            else {
                continue;
            };
            player
                .attributes
                .edit(&handle, |value, observer| {
                    if carried.keep_modifiers {
                        // `AttributeInstance.replaceFrom`: the base and every modifier.
                        value.replace_from(old, observer)
                    } else {
                        value.set_base_value(old.base_value(), observer)
                    }
                })
                .map_err(|e| format!("movement attribute: {e:?}"))?;
        }
        for values in &carried.early {
            apply(&mut player.attributes, values);
        }
        Ok(())
    }
}

impl RespawnAttributes {
    /// `KEEP_ATTRIBUTE_MODIFIERS` of the respawn packet being handled.
    pub(in crate::game) fn respawn(&mut self, keep_modifiers: bool) {
        self.keep_modifiers = Some(keep_modifiers);
    }
}
