//! Typed adaptations for upstream mob knockback and breeding draws.
use super::{Damage, HurtSource, Who, source_damage_type};
use sunrise_core::random::RandomSource;
use sunrise_sim::{lan::data::RegistryRef, mobs::variants::WolfSoundVariant};

impl crate::game::ServerGame<'_> {
    pub(in crate::game) fn mob_pushes_player(
        &mut self,
        power: f64,
        x: f64,
        z: f64,
        source: Damage,
        apply_blocking: Option<f32>,
    ) {
        let mut power = power;
        let attack = self.attack_of(match source.attacker {
            Some(Who::Mob(id)) => id as i32,
            _ => 0,
        });
        if let Some(damage) = apply_blocking
            && let Some(blocking) =
                self.player_blocking(source.from, source_damage_type(source), false)
            && let blocked = blocking.blocked(damage)
            && blocked > 0.0
        {
            let attacker = match source.attacker {
                Some(Who::Mob(id)) => Some(id),
                _ => None,
            };
            self.after_block(
                sunrise_sim::health::BlockOutcome {
                    blocked,
                    fully_blocked: blocked >= damage,
                    ..Default::default()
                },
                (attacker, false),
                HurtSource {
                    damage_type: source_damage_type(source),
                    causing: attacker.map_or(0, |id| id as i32),
                    direct: 0,
                    position: source.from,
                    direct_type: None,
                },
                &attack,
            );
            power *= 0.5;
        }
        let source = HurtSource {
            damage_type: source_damage_type(source),
            causing: 0,
            direct: 0,
            position: None,
            direct_type: None,
        };
        self.knock_back_player(power, x, z, source, &attack, 0.0);
    }
    pub(in crate::game) fn bred_wolf_voice(
        &mut self,
        parent: u32,
    ) -> Option<RegistryRef<WolfSoundVariant>> {
        let len = self.world_variants().wolf_sound.len();
        if len == 0 {
            return None;
        }
        self.common
            .mobs
            .iter_mut()
            .find(|m| m.id == parent)
            .and_then(|m| m.random.next_int_bound(len as i32).ok())
            .map(|i| RegistryRef::new(i as u32))
    }
}
