//! Beds' `occupied` (`BedBlock.OCCUPIED`): the head of a bed someone sleeps
//! in, which the foot copies (`updateShape`). `LivingEntity.startSleeping`
//! sets it and `stopSleeping` clears it. A villager's come with its mob
//! events (`MobEvent::BedOccupied`); a player's bed is set when it lies
//! down (`try_sleep`) and freed in the level's tick once nobody sleeps in it
//! any more, whichever way they got up: waking, the Leave Bed button, death,
//! a broken bed, or leaving the game.
use super::Dim;
use sunrise_game_data::Result;
use sunrise_gameplay_common::game::native_hooks::SleepProblem;

impl crate::game::ServerGame<'_> {
    /// `BedRule.canSleep` of the current dimension's bed rule.
    pub fn bed_can_sleep_now(&self) -> bool {
        self.dimension_type()
            .bed_rule
            .can_sleep
            .test(self.dark_outside())
    }
    /// What `CanContinueSleepingEvent.getProblem` holds for a sleeper: its bed gone
    /// (`checkBedExists`), or for a player (`Player.tick`) a bed rule that no longer lets
    /// them sleep.
    pub(in crate::game::server_side) fn sleep_problem(
        &self,
        bed_exists: bool,
        player: bool,
    ) -> Option<SleepProblem> {
        if !bed_exists {
            Some(SleepProblem::NotPossibleHere)
        } else if player && !self.bed_can_sleep_now() {
            Some(SleepProblem::NotPossibleNow)
        } else {
            None
        }
    }
    /// `setBlockAndUpdate(head, state.setValue(OCCUPIED, occupied))`, when
    /// the block at `head` is still a bed and not already so.
    pub fn set_bed_occupied(&mut self, head: [i32; 3], occupied: bool) {
        let registry = &self.common.pack.assets.registry;
        let Some(mut state) = registry
            .state(self.common.world.block(head[0], head[1], head[2]))
            .cloned()
        else {
            return;
        };
        let value = if occupied { "true" } else { "false" };
        if state.properties.get("occupied").is_none_or(|v| v == value) {
            return;
        }
        state.properties.insert("occupied".into(), value.into());
        let Some(id) = registry.id(&state) else {
            return;
        };
        if let Err(e) = self.level_set(head, id) {
            log::warn!("could not mark the bed at {head:?} occupied={value}: {e}");
        }
    }

    /// `LivingEntity.startSleeping` for the player in place (the host, or
    /// a guest during its turn): the bed it lies in is occupied.
    pub(in crate::game::server_side) fn occupy_bed(&mut self, head: [i32; 3]) {
        self.set_bed_occupied(head, true);
        if !self.server.player_beds.contains(&(self.common.dim, head)) {
            self.server.player_beds.push((self.common.dim, head));
        }
    }

    /// Whether a player in the level in place still sleeps in `head`.
    fn player_in_bed(&self, head: [i32; 3]) -> bool {
        self.common.sleeping == Some(head)
            || self.server.lan.as_ref().is_some_and(|h| {
                h.hub
                    .guests
                    .iter()
                    .any(|g| g.bed == Some(head) && g.dimension == self.common.dim.id())
            })
    }

    /// `LivingEntity.stopSleeping` for the players who got up: the beds of
    /// this level nobody sleeps in any more are free again. Runs in each
    /// level's tick, after the players' packets and sleep ticks, never inside
    /// a guest's turn.
    pub fn free_left_beds(&mut self) {
        let dim = self.common.dim;
        let left: Vec<[i32; 3]> = self
            .server
            .player_beds
            .iter()
            .filter(|(d, head)| *d == dim && !self.player_in_bed(*head))
            .map(|(_, head)| *head)
            .collect();
        for head in left {
            self.server.player_beds.retain(|b| *b != (dim, head));
            self.set_bed_occupied(head, false);
            // AbstractBedBlock.onStopSleeping: the Overworld's straw_bed_rule
            // has destroy_on_leave, so a straw bed breaks when its sleeper gets up.
            let bed = self.common.world.block(head[0], head[1], head[2]);
            let straw = &*self.common.res.facts().get(bed).name == "minecraft:straw_bed";
            if straw
                && self
                    .common
                    .dimension_type_of(dim)
                    .straw_bed_rule
                    .destroy_on_leave
                && let Err(e) = self.destroy_straw_bed(head)
            {
                log::warn!("could not break the straw bed at {head:?}: {e}");
            }
        }
    }

    /// The levels with a bed a player marked occupied: they tick until the
    /// bed is freed.
    pub(in crate::game::server_side) fn player_bed_levels(&self) -> impl Iterator<Item = Dim> + '_ {
        self.server.player_beds.iter().map(|(d, _)| *d)
    }

    /// `PlayerList.removeAll` before the server's last save: every player
    /// leaves (`ServerPlayer.disconnect` stops its sleep), so no bed is
    /// saved occupied by a player who is not in it when the world opens
    /// again. Each bed is freed in its own level.
    pub(in crate::game::server_side) fn free_player_beds(&mut self) -> Result<()> {
        if self.server.player_beds.is_empty() {
            return Ok(());
        }
        let back = self.common.dim;
        for (dim, head) in std::mem::take(&mut self.server.player_beds) {
            self.enter_level(dim)?;
            // A client holds no other levels (`enter_level`).
            if self.common.dim == dim {
                self.set_bed_occupied(head, false);
            }
        }
        self.enter_level(back)
    }
}

use sunrise_gameplay_common::game::weather::WeatherGameExt as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;
