use super::*;

/// Apply the full entry's physical push.
pub fn apply_player_knockback(
    p: &mut sunrise_sim::motion::PlayerMotion,
    level: &mut sunrise_sim::health::DamageLevel<'_>,
    power: f64,
    (xd, zd): (f64, f64),
    source: HurtSource<'_>,
) {
    let knock = Knock {
        strength: power,
        x: xd,
        z: zd,
    };
    // A clone is the owner's state, so it goes back whether or not the push lands.
    let mut random = p.random.clone();
    let pushed = player_knockback_velocity(p, &mut random, level, knock, source);
    p.random = random;
    if let Some(velocity) = pushed {
        p.velocity = velocity;
    }
}

/// The full player path, shared with calls that borrow a guest's body in its own level.
/// `random` is the player's own (`Entity.random`), which a short direction's retries draw from;
/// the caller writes it back to the player.
pub fn player_knockback_velocity(
    player: &sunrise_sim::motion::PlayerMotion,
    random: &mut sunrise_core::random::EntityRandom,
    level: &mut sunrise_sim::health::DamageLevel<'_>,
    mut knock: Knock,
    source: HurtSource<'_>,
) -> Option<Vec3> {
    let pushed = Hurting::Player(player.entity_id, source);
    if level.knockback_canceled(pushed, &mut knock) {
        return None;
    }
    let resistance = player
        .attributes
        .value(
            player.attributes.key("minecraft:knockback_resistance"),
            sunrise_world::attribute_map::ValueKind::Computed,
        )
        .unwrap_or(0.0);
    let mut velocity = player.velocity;
    knock
        .apply(&mut velocity, resistance, player.ground.on_ground, random)
        .then_some(velocity)
}
