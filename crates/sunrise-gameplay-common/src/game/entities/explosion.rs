#[cfg(any(test, feature = "test-support"))]
pub mod tests {

    /// A zombie's type with only what a hurt and a push read.
    pub fn zombie_type() -> std::sync::Arc<sunrise_sim::mobs::MobType> {
        use sunrise_sim::mobs::{Anim, Brain, MobType, Species};
        std::sync::Arc::new(MobType {
            type_key: Default::default(),
            id: "minecraft:zombie".into(),
            species: Species::Zombie,
            width: 0.6,
            height: 1.95,
            eye_height: 1.74,
            category: "monster".into(),
            max_health: 20.0,
            speed: 0.23,
            flying_speed: 0.02,
            attack_damage: 3.0,
            attack_knockback: 0.0,
            follow_range: 35.0,
            step_height: 0.6,
            armor: 2.0,
            armor_toughness: 0.0,
            knockback_resistance: 0.0,
            jump_strength: 0.42,
            attributes: Default::default(),
            item_stack_limits: Default::default(),
            attribute_bounds: Default::default(),
            gravity: 0.08,
            safe_fall_distance: 3.0,
            fall_damage_multiplier: 1.0,
            water_efficiency: 0.0,
            water_slowdown: 0.8,
            air_speed: 0.02,
            baby_scale: 0.5,
            head_rot_speed: 10.0,
            max_head_x_rot: 40.0,
            max_head_y_rot: 75.0,
            ambient_interval: 80,
            flags: Default::default(),
            brain: Brain::Passive,
            anim: Anim::Humanoid { zombie_arms: true },
            sounds: ["", "", ""],
            sound_overrides: Default::default(),
            food: Default::default(),
            tempt: Default::default(),
            passenger_attachments: Vec::new(),
        })
    }
}
