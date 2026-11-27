//! Client crack state belongs to one level and expires independently of new updates.
#[cfg(feature = "client")]
use super::Mining;

#[cfg(feature = "client")]
pub struct ClientCrack {
    pub pos: [i32; 3],
    pub stage: u8,
    pub updated_at: i64,
    pub positions: Vec<[i32; 3]>,
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    pub fn expire_client_cracks(&mut self) {
        let now = self.common.world.game_time;
        if now % 20 != 0 {
            return;
        }
        self.client
            .destroying
            .retain(|_, crack| now.saturating_sub(crack.updated_at) <= 400);
    }

    #[allow(dead_code)]
    #[inline(always)]
    pub(in crate::game) fn client_expire_cracks(&mut self) {
        self.expire_client_cracks();
    }

    pub fn reset_client_world(&mut self, seed: i64) {
        self.install_client_world(sunrise_gameplay_common::game::world_for_mods(
            self.common.res.facts().air,
            seed,
        ));
    }

    pub(in crate::game) fn install_client_world(&mut self, world: sunrise_level::world::World) {
        self.client.destroying.clear();
        self.client.mining_crack = None;
        self.common.mining = None;
        self.common.world = world;
    }

    /// Preserve the stage's stored display answer across prediction updates.
    pub fn set_client_mining(&mut self, mining: Option<Mining>) {
        let keep = mining.as_ref().is_some_and(|next| {
            self.common
                .mining
                .as_ref()
                .is_some_and(|old| old.pos == next.pos)
        });
        if !keep {
            self.client.mining_crack = None;
        }
        self.common.mining = mining;
        self.update_local_crack();
    }

    pub(in crate::game) fn update_local_crack(&mut self) {
        let Some((pos, stage)) = self
            .common
            .mining
            .as_ref()
            .and_then(|mining| Some((mining.pos, mining.stage()?)))
        else {
            self.client.mining_crack = None;
            return;
        };
        if self
            .client
            .mining_crack
            .as_ref()
            .is_some_and(|crack| crack.pos == pos && crack.stage == stage)
        {
            return;
        }
        // Mark this stage before dispatch, so a reentrant update cannot ask twice.
        self.client.mining_crack = Some(ClientCrack {
            pos,
            stage,
            updated_at: self.common.world.game_time,
            positions: Vec::new(),
        });
        self.with_break_progress_positions(pos, stage, |game, positions| {
            if let Some(mining) = &game.mining
                && mining.pos == pos
                && mining.stage() == Some(stage)
                && let Some(crack) = &mut game.client.mining_crack
                && crack.pos == pos
                && crack.stage == stage
            {
                crack.positions = positions;
            }
        });
    }
}
