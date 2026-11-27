use std::sync::Arc;
use sunrise_core::geometry::Vec3;
use sunrise_sim::mobs::{Anim, Brain, Mob, MobType, Species};

/// A mob of `species` with the box of its vanilla type, built without the
/// prepared resources a test machine may not have.
pub fn mob(id: u32, entity: &str, species: Species, width: f64, height: f64) -> Mob {
    let kind = MobType {
        type_key: Default::default(),
        id: entity.into(),
        species,
        width,
        height,
        eye_height: height * 0.6,
        category: "creature".into(),
        max_health: 10.0,
        speed: 0.2,
        flying_speed: 0.02,
        attack_damage: 1.0,
        attack_knockback: 0.0,
        follow_range: 16.0,
        step_height: 0.6,
        armor: 0.0,
        armor_toughness: 0.0,
        knockback_resistance: 0.0,
        jump_strength: 0.42,
        attributes: Default::default(),
        item_stack_limits: Default::default(),
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
        anim: Anim::Humanoid { zombie_arms: false },
        sounds: ["entity.cow.hurt", "entity.cow.hurt", "entity.cow.death"],
        food: Default::default(),
        tempt: Default::default(),
        attribute_bounds: Default::default(),
        sound_overrides: Default::default(),
        passenger_attachments: Vec::new(),
    };
    Mob::new(id, Arc::new(kind), Vec3::new(0.0, 64.0, 0.0), 0.0)
}
