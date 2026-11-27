use crate::game::ClientGame;
use sunrise_client::hud::HudBatch;
use sunrise_sim::mobs::Species;

impl ClientGame<'_> {
    /// `LocalPlayer.jumpableVehicle`: controlled vehicle when it can jump now.
    pub(in crate::game) fn jumpable_mount(&self) -> Option<u32> {
        if !self
            .client
            .remote
            .as_ref()
            .is_some_and(|r| r.first_passenger)
        {
            return None;
        }
        let mount = &self.common.mobs[self.common.mount_index()?];
        let can_jump = sunrise_gameplay_common::game::CommonGame::jumps(mount.species())
            && mount.extra.saddled()
            && !(matches!(mount.species(), Species::Camel | Species::CamelHusk)
                && sunrise_sim::mobs::camel::refuse_to_move(mount));
        can_jump.then_some(mount.id)
    }

    /// The jump bar when it is the contextual bar.
    pub(in crate::game) fn draw_jump_bar(&self, batch: &mut HudBatch, w: i32, h: i32) {
        let Some(i) = self.common.mount_index() else {
            return;
        };
        let m = &self.common.mobs[i];
        // JumpableVehicleBar: `getJumpCooldown`, which only the camel's and
        // the nautilus' dash have; a horse always shows the progress.
        let cooldown = i32::from(
            self.common.mount.jump_ticks < 0
                && (matches!(m.species(), Species::Camel | Species::CamelHusk)
                    || sunrise_sim::mobs::nautilus::is_nautilus(m.species())),
        );
        let draws =
            sunrise_client::hud_overlays::jump_bar(w, h, cooldown, self.common.mount.jump_scale);
        sunrise_client::hud_layout::paint(batch, &self.client.gui.atlas, &draws);
    }
    /// `willPrioritizeJumpInfo`: charging a jump or recharging.
    pub(in crate::game) fn mount_jumping(&self) -> bool {
        self.common.mount.jump_scale > 0.0 || self.common.mount.jump_ticks < 0
    }
    pub(in crate::game) fn client_own_passenger_index(&self, _vehicle: u32) -> usize {
        self.client.remote.as_ref().map_or(0, |r| r.passenger_index)
    }
}
