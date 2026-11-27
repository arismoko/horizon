#[cfg(any(test, feature = "test-support"))]
pub mod tests {

    use sunrise_sim::mobs::equip;

    pub fn armor(points: f64, binding: bool) -> equip::Facts {
        equip::Facts {
            armor: points,
            armor_toughness: 0.0,
            attack_damage: 0.0,
            enchantments: 0,
            damage: 0,
            custom_name: false,
            prevents_armor_change: binding,
        }
    }
}
