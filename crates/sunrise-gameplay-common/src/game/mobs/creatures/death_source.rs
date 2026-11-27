use super::*;

/// A synthetic loot/death-tail fallback when no accepted hit record exists.
/// Real killing sources and their weapons are retained on the victim.
pub fn death_hurt_source(damage_type: &str, causing: u32) -> HurtSource<'_> {
    let direct = if sunrise_sim::health::names_its_projectile(damage_type) {
        0
    } else {
        causing as i32
    };
    HurtSource {
        damage_type,
        causing: causing as i32,
        direct,
        position: None,
        direct_type: None,
    }
}
