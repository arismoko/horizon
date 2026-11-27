use sunrise_sim::mobs::{MobEvent, PlayerView, Who};

/// The guests mobs can see this tick, by connection.
pub type GuestViews = Vec<(u32, PlayerView)>;

/// The events of a mob's tick that act on the player it sees.
#[inline]
pub fn acts_on_player(e: &MobEvent) -> bool {
    matches!(
        e,
        MobEvent::AttackPlayer(..)
            | MobEvent::CubeHurtPlayer { .. }
            | MobEvent::AttackPlayerWithEffect { .. }
            | MobEvent::Sting {
                target: Who::Player,
                ..
            }
            | MobEvent::DamagePlayer { .. }
            | MobEvent::SonicBoom {
                target: Who::Player,
                ..
            }
            | MobEvent::Push {
                target: Who::Player,
                ..
            }
            | MobEvent::Knockback {
                target: Who::Player,
                ..
            }
            | MobEvent::StabDismount {
                target: Who::Player,
            }
            | MobEvent::StabMemory {
                target: Who::Player,
                ..
            }
            | MobEvent::Ignite(Who::Player, _)
            | MobEvent::ZombieIgnite(Who::Player, _)
            | MobEvent::IronGolemHit {
                target: Who::Player,
                ..
            }
            | MobEvent::EffectPlayer { .. }
            | MobEvent::AxolotlSupportPlayer { .. }
            | MobEvent::ParrotToShoulder { .. }
    )
}
