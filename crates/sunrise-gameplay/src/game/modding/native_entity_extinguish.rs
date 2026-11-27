//! `Entity.extinguishFire`, distinct from the silent `clearFire` write.
use super::{commands::Target, powder_snow::extinguish_pitch};
use mod_api::{EntityRef, SoundSource};

impl crate::game::ServerGame<'_> {
    pub(in crate::game) fn entity_extinguish_fire(&mut self, entity: EntityRef) -> bool {
        match self.native_entity_target(entity) {
            Some(target @ (Target::Player | Target::Guest(_))) => self
                .for_player(target, |game| {
                    let Some(player) = game.player.as_ref() else {
                        return false;
                    };
                    let ticks = player.fire_ticks;
                    let at = [player.position.x, player.position.y, player.position.z];
                    if player.is_on_fire() {
                        let pitch = extinguish_pitch(&mut game.server.effect_rng);
                        game.server_play_sound_on(
                            SoundSource::Players.index(),
                            "minecraft:entity.generic.extinguish_fire",
                            at,
                            0.7,
                            pitch,
                        );
                    }
                    if let Some(player) = game.player.as_mut() {
                        player.set_remaining_fire_ticks(ticks.min(0));
                    }
                    true
                })
                .unwrap_or(false),
            Some(Target::Mob(id)) => {
                let Some(mob) = self.common.mobs.iter_mut().find(|mob| mob.id == id) else {
                    return false;
                };
                // Own random, effective fire immunity, sound outbox, then clearFire.
                mob.extinguish_fire();
                mob.needs_sync = true;
                true
            }
            Some(Target::Stand(id)) => {
                let Some(index) = self
                    .common
                    .armor_stands
                    .iter()
                    .position(|stand| stand.id == id)
                else {
                    return false;
                };
                let stand = &self.common.armor_stands[index];
                let ticks = stand.fire_ticks;
                let at = [stand.position.x, stand.position.y, stand.position.z];
                if stand.is_on_fire() {
                    let pitch = extinguish_pitch(&mut self.server.effect_rng);
                    self.server_play_sound_on(
                        SoundSource::Neutral.index(),
                        "minecraft:entity.generic.extinguish_fire",
                        at,
                        0.7,
                        pitch,
                    );
                }
                self.common.armor_stands[index].fire_ticks = ticks.min(0);
                true
            }
            Some(Target::Item(id)) => {
                let Some(index) = self.common.dropped.iter().position(|item| item.id == id) else {
                    return false;
                };
                let immune = self
                    .stack_can_be_hurt_by(&self.common.dropped[index].stack, "minecraft:in_fire")
                    == Some(false);
                let sound = {
                    let item = &mut self.common.dropped[index];
                    let play = (item.fire_ticks > 0 && !immune).then(|| {
                        let at = [item.position.x, item.position.y, item.position.z];
                        let pitch = extinguish_pitch(&mut item.random);
                        (at, pitch)
                    });
                    item.fire_ticks = item.fire_ticks.min(0);
                    play
                };
                if let Some((at, pitch)) = sound {
                    self.server_play_sound_on(
                        SoundSource::Ambient.index(),
                        "minecraft:entity.generic.extinguish_fire",
                        at,
                        0.7,
                        pitch,
                    );
                }
                true
            }
            Some(target) => {
                let Some(view) = self.read_entity_base(target) else {
                    return false;
                };
                let burning = view.base.fire_ticks > 0 && !view.fire_immune;
                let at = self.target_position(target);
                let pitch = burning.then(|| extinguish_pitch(&mut self.server.effect_rng));
                if !self.update_entity_base(target, |b| b.fire_ticks = b.fire_ticks.min(0)) {
                    return false;
                }
                if let (Some(at), Some(pitch)) = (at, pitch) {
                    self.server_play_sound_on(
                        SoundSource::Neutral.index(),
                        "minecraft:entity.generic.extinguish_fire",
                        at,
                        0.7,
                        pitch,
                    );
                }
                true
            }
            None => false,
        }
    }
}

use sunrise_gameplay_common::game::native_components::NativeComponentsGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;
