//! `xp_orb_targeting` (NeoForge's `XpOrbTargetingEvent`): the player an
//! experience orb goes for.
use crate::game::ServerGame;

/// A player an orb may follow: the feet, the point the orb homes on and the
/// player's UUID (None for a host with no saved UUID).
pub(in crate::game) type OrbCandidate = ([f64; 3], [f64; 3], Option<u128>, u32);

/// `ExperienceOrb.followNearbyPlayer`'s scan distance.
const SCAN_DISTANCE: f64 = 8.0;

impl ServerGame<'_> {
    /// The point orb `i` homes on this tick (`ExperienceOrb.followNearbyPlayer`):
    /// the nearest of `followed` within 8 blocks, unless the mods choose
    /// another player or none.
    pub fn orb_target(&mut self, i: usize, followed: &[OrbCandidate]) -> Option<[f64; 3]> {
        let o = &self.common.orbs[i];
        let (id, at, join) = (o.id, o.position, o.join);
        let distance = |feet: &[f64; 3]| -> f64 {
            (feet[0] - at.x).powi(2) + (feet[1] - at.y).powi(2) + (feet[2] - at.z).powi(2)
        };
        let kept = o
            .following
            .and_then(|id| followed.iter().find(|c| c.3 == id))
            .filter(|c| distance(&c.0) <= SCAN_DISTANCE * SCAN_DISTANCE);
        let nearest = kept.or_else(|| {
            followed
                .iter()
                .filter(|c| distance(&c.0) < SCAN_DISTANCE * SCAN_DISTANCE)
                .min_by(|a, b| distance(&a.0).total_cmp(&distance(&b.0)))
        });
        self.common.orbs[i].following = nearest.map(|c| c.3);
        if !self.native_authoritative()
            || !mod_api::__private::listened(mod_api::HookId::XpOrbTargeting)
        {
            return nearest.map(|c| c.1);
        }
        let Some(epoch) = self.common.native_lifecycle.epoch() else {
            return nearest.map(|c| c.1);
        };
        let orb = mod_api::EntityRef::new(epoch, id);
        let found = nearest.and_then(|c| c.2).and_then(|uuid| self.player(uuid));
        let mut following = found;
        self.native_xp_orb_targeting(orb, SCAN_DISTANCE, &mut following);
        let i = self.common.orbs.locate(join)?;
        if following == found {
            return nearest.map(|c| c.1);
        }
        // Vanilla keeps the answer only for a living player who isn't a
        // spectator: the candidates are exactly those.
        let chosen = following.and_then(|p| followed.iter().find(|c| c.2 == Some(p.uuid())));
        self.common.orbs[i].following = chosen.map(|c| c.3);
        chosen.map(|c| c.1)
    }
}

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
