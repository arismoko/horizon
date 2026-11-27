use crate::game::ServerGame;
use sunrise_gameplay_common::game::held_items::OFFHAND_SLOT;
use sunrise_gameplay_common::game::totem_common::TOTEM_EVENT;
use sunrise_items::items::ItemStack;
use sunrise_sim::{health::HealthEvent, motion::MotionEvent};

impl ServerGame<'_> {
    /// The `death_effects` of the stack's `minecraft:death_protection`.
    pub(in crate::game) fn death_effects(
        &self,
        s: &ItemStack,
    ) -> Vec<sunrise_items::items::ConsumeEffect> {
        s.death_protection(self.items().get(&s.item))
            .unwrap_or_default()
            .to_vec()
    }
    /// Finish a hit before its caller can hurt the acting player again. Protection and
    /// `ServerPlayer.die` both run on the current body, not its between-turn projection.
    pub(in crate::game) fn finish_guest_damage(&mut self) {
        let _ = self.finish_guest_damage_before_resume();
    }
    /// Broadcast the guest's activation before the hit's Post callback.
    fn apply_totem(&mut self, entity: i32, protection: sunrise_sim::health::PlayerProtection) {
        let slot = protection.hand.map(|hand| match hand {
            mod_api::Hand::Main => self.common.selected,
            mod_api::Hand::Off => OFFHAND_SLOT,
        });
        let protector = protection.stack;
        if self
            .common
            .player
            .as_ref()
            .filter(|p| p.entity_id == entity)
            .is_none()
        {
            return;
        }
        // A mod's protector from outside the hands is its own to take.
        if let Some(slot) = slot {
            self.consume_player_slot(slot, 1);
        }
        self.award_stat("used", &protector.item, 1);
        self.trigger_item("minecraft:used_totem", &protector);
        let Some(feet) = self
            .common
            .player
            .as_ref()
            .filter(|p| p.entity_id == entity)
            .map(|p| [p.position.x, p.position.y, p.position.z])
        else {
            return;
        };
        // ItemStack.causeUseVibration: the totem's use effects make one.
        let ctx = self.player_context(None);
        self.post_game_event("minecraft:item_interact_finish", feet, ctx);
        let Some(p) = self
            .common
            .player
            .as_mut()
            .filter(|p| p.entity_id == entity)
        else {
            return;
        };
        let _ = p.set_health(1.0, false);
        // `DeathProtection.applyEffects` reads the component from the accepted copy.
        let user = self.consuming_player_entity();
        for effect in self.death_effects(&protector) {
            self.apply_consume_effect(&protector, &effect, None, user);
        }
        if self.server.lan_acting.is_some()
            && let Some(host) = &self.server.lan
        {
            host.hub.entity_event(entity, TOTEM_EVENT);
        }
    }
    /// Settle the hit, returning the remaining events instead of resuming if the acting body is gone.
    pub(in crate::game) fn finish_guest_damage_before_resume(
        &mut self,
    ) -> Result<(), Vec<MotionEvent>> {
        let (Some(conn), Some(player)) = (self.server.lan_acting, self.common.player.as_mut())
        else {
            return Err(Vec::new());
        };
        let Some(hit) = player.health.damage_completion.take() else {
            return Ok(());
        };
        let entity = player.entity_id;
        // Keep queued motion and packet effects with this body across the owning callbacks.
        let mut events = std::mem::take(&mut player.events);
        if let Some(protection) = hit.protection {
            self.apply_totem(entity, protection);
        } else if let Some(index) = self.server.lan.as_ref().and_then(|host| {
            host.hub
                .guests
                .connection_index(conn)
                .filter(|&i| !host.hub.guests[i].dead)
        }) {
            // The killing blow and its weapon must reach the combat tracker before die reads it.
            // Packet and motion events still belong to the normal turn drain.
            let combat: Vec<_> = events
                .extract_if(.., |event| {
                    matches!(
                        event,
                        MotionEvent::Health(
                            HealthEvent::ArmorHurt { .. }
                                | HealthEvent::Criterion { .. }
                                | HealthEvent::MobHurt
                                | HealthEvent::RecordedDamage { .. }
                                | HealthEvent::Statistic { .. }
                        )
                    )
                })
                .collect();
            if !combat.is_empty() {
                self.record_combat(&combat);
                self.wear_armor(&combat);
            }
            self.guest_die(index, conn);
        }
        // The death or protection callback may change or remove the body. Post is still for
        // this hit, not for whatever state the callback left in its place.
        if let Some(post) = hit.post {
            self.finish_damage_post(entity, "minecraft:player", post);
        }
        if self.server.lan_acting == Some(conn)
            && let Some(player) = self
                .common
                .player
                .as_mut()
                .filter(|p| p.entity_id == entity)
        {
            events.append(&mut player.events);
            player.events = events;
            Ok(())
        } else {
            Err(events)
        }
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;
