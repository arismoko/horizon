#[cfg(any(test, feature = "test-support"))]
pub mod tests {

    use crate::game::dragon_common::{Dragon, Phase};
    use sunrise_core::geometry::{Aabb, Vec3};
    use sunrise_sim::health::HurtSource;

    pub fn player_box(feet: [f64; 3]) -> (Aabb, Vec3) {
        (
            Aabb::new(
                Vec3::new(feet[0] - 0.3, feet[1], feet[2] - 0.3),
                Vec3::new(feet[0] + 0.3, feet[1] + 1.8, feet[2] + 0.3),
            ),
            Vec3::new(feet[0], feet[1], feet[2]),
        )
    }
    /// The damage type of the hits below, which count.
    pub const HIT: Option<HurtSource> = Some(HurtSource {
        damage_type: "minecraft:player_attack",
        causing: 0,
        direct: 0,
        position: None,
        direct_type: None,
    });
    pub fn dragon(phase: Phase) -> Dragon {
        let mut d = Dragon::new(Vec3::new(0.0, 70.0, 0.0), 0.0, 1);
        d.phase = phase;
        d
    }
    pub fn flying() -> Phase {
        Phase::Holding {
            path: None,
            target: None,
            clockwise: false,
        }
    }
    /// The dragon hits the hook of these tests saw: entity, damage type,
    /// causing and direct entities, amount.
    type Seen = (String, i32, i32, f32);
    static SEEN: std::sync::Mutex<Vec<(i32, Seen)>> = std::sync::Mutex::new(Vec::new());
    pub fn seen(entity: i32) -> Vec<Seen> {
        let seen = SEEN.lock().unwrap();
        seen.iter()
            .filter(|s| s.0 == entity)
            .map(|s| s.1.clone())
            .collect()
    }
    /// Sets the native damage hook of these tests: it records the dragon
    /// hits of entities 9001 to 9005, cancels 9001's and doubles 9002's.
    pub fn hooks() -> sunrise_sim::health::DamageHooks {
        use sunrise_sim::health::{DamageContainer, DamageHooks, DamageLevel, Hurting, Victim};
        fn hook(
            _: DamageLevel<'_>,
            hurting: Hurting<'_>,
            _: Victim,
            container: &mut DamageContainer,
        ) -> bool {
            let Hurting::Dragon(entity @ 9001..=9005, s) = hurting else {
                return false;
            };
            let amount = container.new_damage();
            let hit = (s.damage_type.to_owned(), s.causing, s.direct, amount);
            SEEN.lock().unwrap().push((entity, hit));
            if entity == 9002 {
                container.record_new_damage(amount * 2.0);
            }
            entity == 9001
        }
        DamageHooks {
            incoming: Some(hook),
            ..DamageHooks::NONE
        }
    }
    pub fn dragon_as(entity: u32) -> Dragon {
        let mut d = dragon(flying());
        d.id = entity;
        d
    }
}
