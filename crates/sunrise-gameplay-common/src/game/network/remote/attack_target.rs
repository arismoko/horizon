use super::*;

impl crate::game::CommonGame<'_> {
    /// `AttackRange.isInRange` measures from the player's current eyes, not the pick ray's origin.
    pub fn attack_hit_distance(&self, [x, y, z]: [f64; 3]) -> Option<f64> {
        let [eye_x, eye_y, eye_z] = self.eye_position()?;
        Some(Vec3::new(x, y, z).distance(Vec3::new(eye_x, eye_y, eye_z)))
    }
}

#[cfg(any())]
#[cfg(test)]
pub(in crate::game) mod tests {

    #[test]
    #[ignore = "daily"]
    fn a_sprint_hit_on_a_boat_minecart_crystal_or_bullet_slows_the_attacker_and_stops_its_sprint() {
        // The resource loader sets process-wide tables (the cauldron interactions): own process.
        crate::game::in_own_process(
            "game::remote::attack_target::tests::a_sprint_hit_on_a_boat_minecart_crystal_or_bullet_slows_the_attacker_and_stops_its_sprint",
            sprint_hits_isolated,
        );
    }

    #[allow(unused_imports)]
    pub(in crate::game) use crate::game::client_side::remote_client::attack_target::tests::sprint_hits_isolated;
}
