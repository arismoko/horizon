use crate::game::ClientGame;
use std::convert::Infallible;
use sunrise_core::random::EntityRandom;
use sunrise_gameplay_common::game::item_use_common::ItemConsumable;
use sunrise_items::items::ItemStack;
use sunrise_level::particle_options::{ParticleOptions, ParticleType};
use sunrise_world::consumable::EmissionWorld;

struct ClientEmitter<'a, 'g>(&'a mut crate::game::ClientGame<'g>);

impl EmissionWorld for ClientEmitter<'_, '_> {
    type Error = Infallible;
    type Stack = ItemStack;
    type Sound = String;
    type Random = ();
    type Source = EntityRandom;
    fn random_source(&mut self, (): ()) -> &mut EntityRandom {
        &mut self
            .0
            .common
            .player
            .as_mut()
            .expect("emitting player")
            .random
    }
    fn spawn_item_particles(&mut self, stack: ItemStack, count: i32) -> Result<(), Infallible> {
        self.0.eat_particles(&stack, count.max(0) as u32);
        Ok(())
    }
    fn overrides_consume_sound(&self) -> bool {
        false
    }
    fn consume_sound(&mut self, _: ItemStack) -> Result<String, Infallible> {
        unreachable!("players play the consumable's own sound")
    }
    fn play_sound(&mut self, sound: String, volume: f32, pitch: f32) -> Result<(), Infallible> {
        self.0.play_client_player_sound(&sound, volume, pitch);
        Ok(())
    }
}

impl ClientGame<'_> {
    /// Client-side `Consumable.emitParticlesAndSounds`.
    pub(in crate::game) fn client_emit_consume(
        &mut self,
        consumable: &ItemConsumable,
        stack: &ItemStack,
        count: i32,
    ) {
        if self.common.player.is_some() {
            let Ok(()) = consumable.emit(&mut ClientEmitter(self), (), stack.clone(), count);
        }
    }

    /// `LivingEntity.handleEntityEvent(46)` for the entity the packet names:
    /// this player, or a mob the client is drawing.
    pub(in crate::game::client_side) fn entity_teleport_particles(&mut self, id: i32) {
        if self
            .client
            .remote
            .as_ref()
            .is_some_and(|r| r.entity_id == id)
        {
            self.portal_particles();
            return;
        }
        let spawned = {
            let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id as u32) else {
                return;
            };
            let from = [m.previous.x, m.previous.y, m.previous.z];
            let to = [m.position.x, m.position.y, m.position.z];
            let (width, height) = (f64::from(m.width()), f64::from(m.height()));
            let mut spawned = Vec::with_capacity(128);
            sunrise_client::particles::living_teleport_particles(
                &mut m.random,
                from,
                to,
                width,
                height,
                |p, v| spawned.push((p, v)),
            );
            spawned
        };
        let options = ParticleOptions::simple(ParticleType::Portal);
        for (p, v) in spawned {
            self.client.particles.0.add(&options, p, v);
        }
    }
    /// `LivingEntity.handleEntityEvent(46)` for this client: 128 portal
    /// particles from the old position to the new one. The local player's
    /// random is the particle engine's.
    pub(in crate::game) fn portal_particles(&mut self) {}
}
