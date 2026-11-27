use mod_api::HurtSource;
use std::sync::Arc;
use sunrise_items::items::ItemStack;
use sunrise_sim::health::AcceptedHit;

/// The source a player's death hooks hear (`living_death`, `living_drops`,
/// `living_death_post`): the accepted hit retained on the body, else the
/// death's own source, `generic_kill` for `/kill`.
pub struct FatalSource {
    pub accepted: Option<Arc<AcceptedHit>>,
    pub damage_type: String,
    pub causing: u32,
}

impl FatalSource {
    pub fn source(&self) -> HurtSource<'_> {
        self.accepted.as_ref().map_or(
            HurtSource {
                damage_type: &self.damage_type,
                causing: self.causing as i32,
                direct: self.causing as i32,
                position: None,
                direct_type: None,
            },
            |hit| hit.source(),
        )
    }

    pub fn weapon(&self) -> Option<&ItemStack> {
        self.accepted.as_ref().and_then(|hit| hit.weapon.as_ref())
    }
}
