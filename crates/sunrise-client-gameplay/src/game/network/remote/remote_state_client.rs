use super::*;

impl Remote {
    pub(in crate::game) fn clock_time(&self, id: Option<i32>) -> i64 {
        id.and_then(|id| self.clocks.get(&id))
            .map_or(0, |c| c.total_ticks)
    }
    /// The profile of the player entity `id`: this window's own player, or
    /// another one by the UUID it was added with and its profile name.
    pub fn level_player(&self, id: i32) -> Option<(u128, Option<&str>)> {
        if id == self.entity_id {
            return Some((self.profile, Some(self.profile_name.as_str())));
        }
        let uuid = *self.level_players.get(&id)?;
        Some((uuid, self.level_player_names.get(&id).map(String::as_str)))
    }
    /// Listed UUIDs always have a tab entry; removal clears both together.
    pub(in crate::game) fn listed_player_count(&self) -> usize {
        self.listed.len()
    }
    /// The synchronized clock supplying a time-based environment layer.
    pub(in crate::game) fn named_clock_time(&self, clock: &str) -> i64 {
        self.clock_time(self.registries.id("minecraft:world_clock", clock))
    }
    /// `ClientPacketListener.initializeChatSession` for `sender`'s entry: a session the services'
    /// keys vouch for makes a keyed validator, anything else the fallback of
    /// `enforcesSecureChat()` (`canValidateProfileKeys() && serverEnforcesSecureChat`). `false`
    /// while the keys are not known yet.
    pub(in crate::game::client_side) fn new_chat_validator(
        &self,
        sender: u128,
    ) -> (secure_chat::MessageValidator, bool) {
        let keys = self.services_keys.lock().unwrap_or_else(|e| e.into_inner());
        let certificate_keys = match &*keys {
            ServicesKeys::Pending => {
                return (secure_chat::MessageValidator::fallback(false), false);
            }
            ServicesKeys::Ready(keys) => &keys.player_certificate_keys,
        };
        let can_validate = !certificate_keys.is_empty();
        let fallback =
            secure_chat::MessageValidator::fallback(can_validate && self.enforces_secure_chat);
        let Some(Some((id, expires_at, key, signature))) = self.chat_sessions.get(&sender) else {
            return (fallback, true);
        };
        if !can_validate {
            log::warn!(
                "Ignoring chat session from {sender:032x} due to missing Services public key"
            );
            return (fallback, true);
        }
        let profile_key = secure_chat::ProfileKey {
            expires_at: *expires_at,
            key: key.clone(),
            signature: signature.clone(),
        };
        match secure_chat::ChatSession::validate(*id, profile_key, certificate_keys, sender) {
            Ok(session) => (secure_chat::MessageValidator::keyed(session), true),
            Err(why) => {
                log::error!("Failed to validate profile key for player {sender:032x}: {why}");
                (fallback, true)
            }
        }
    }
    /// The entity id of a player in the level, by profile UUID.
    pub(in crate::game) fn player_entity(&self, uuid: u128) -> Option<u32> {
        self.level_players
            .iter()
            .find(|(_, u)| **u == uuid)
            .map(|(id, _)| *id as u32)
    }
    /// Resolve registry keys only when registries or the dimension change.
    pub(in crate::game::client_side) fn resolve_clocks(&mut self) {
        self.overworld_clock = self
            .registries
            .id("minecraft:world_clock", "minecraft:overworld");
        self.default_clock = self
            .dimension
            .default_clock
            .as_deref()
            .and_then(|name| self.registries.id("minecraft:world_clock", name));
        for id in [self.overworld_clock, self.default_clock]
            .into_iter()
            .flatten()
        {
            self.clocks.entry(id).or_insert(play::ClockState {
                clock: id,
                total_ticks: 0,
                partial_tick: 0.0,
                rate: 1.0,
            });
        }
    }
    /// How far the server sends chunks: its view distance and the ring
    /// around (`ChunkTrackingView`), at least two.
    pub(in crate::game::client_side) fn sent_radius(&self) -> i32 {
        self.view_distance.max(2).saturating_add(1)
    }
    pub(in crate::game::client_side) fn set_tracked_position(
        &mut self,
        id: i32,
        position: [f64; 3],
    ) {
        if let Some(entity) = self.entities.get_mut(&id) {
            entity.position = position;
        }
    }
    /// The interpolation a boat or other object `id` had toward the server's
    /// position, taken out: its target, rotation and steps to go.
    pub(in crate::game::client_side) fn take_object_lerp(
        &mut self,
        id: u32,
    ) -> Option<([f64; 3], Option<(f32, f32)>, u32)> {
        self.lerps
            .remove(&(id as i32))
            .map(|l| (l.target, l.rotation, l.steps))
    }
    /// `PlayerInfo.getMessageValidator().updateAndValidate(message)` for `message.sender`.
    pub(in crate::game::client_side) fn validate_chat(
        &mut self,
        message: &secure_chat::Incoming,
        now: i64,
    ) -> Option<secure_chat::Validated> {
        let sender = message.sender;
        if let Some(validator) = self.chat_validators.get_mut(&sender) {
            return validator.update_and_validate(message, now);
        }
        let (mut validator, settled) = self.new_chat_validator(sender);
        let validated = validator.update_and_validate(message, now);
        // While the services' keys are still on their way the validator is a stopgap, not kept.
        if settled {
            self.chat_validators.insert(sender, validator);
        }
        validated
    }
}
