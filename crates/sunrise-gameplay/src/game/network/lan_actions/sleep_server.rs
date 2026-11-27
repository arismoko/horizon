use super::*;

impl crate::game::ServerGame<'_> {
    /// `PlayerList.broadcastSystemMessage` of the advancements guests made
    /// (their own actions, commands and ticks).
    pub(in crate::game) fn announce_guest_advancements(&mut self) {
        if self.server.lan.is_none() {
            return;
        }
        for text in std::mem::take(&mut self.common.adv.announced) {
            if let Some(host) = self.server.lan.as_mut() {
                host.hub
                    .system_message(sunrise_sim::lan::data::component_nbt(&text.component()));
            }
        }
    }
    /// `ServerLevel.updateSleepingPlayerList` and `announceSleepStatus`: when
    /// the players or the sleepers change, a published or dedicated world
    /// shows everyone "n/m players sleeping", or "Sleeping through this
    /// night" once enough are in bed, on the action bar.
    pub(in crate::game::server_side) fn announce_sleep_status(&mut self) {
        let (active, sleeping, _) = self.sleep_counts();
        let (old_active, old_sleeping) = self.server.sleep_seen;
        self.server.sleep_seen = (active, sleeping);
        let changed = (old_sleeping > 0 || sleeping > 0)
            && (old_active != active || old_sleeping != sleeping);
        let published = self.server.lan.is_some() || self.common.options.dedicated.is_some();
        if !changed || !published {
            return;
        }
        // ServerLevel.canSleepThroughNights.
        let percentage = self
            .common
            .ui
            .rules
            .int_at(sunrise_game_data::gamerules::vanilla::PLAYERS_SLEEPING_PERCENTAGE);
        if percentage > 100 {
            return;
        }
        let needed = self.sleepers_needed(active);
        let skipping_night = sleeping >= needed;
        let mut text = if skipping_night {
            sunrise_game_data::text::Text::translate("sleep.skipping_night", &[])
        } else {
            sunrise_game_data::text::Text::translate(
                "sleep.players_sleeping",
                &[
                    sunrise_game_data::text::Text::literal(sleeping.to_string()),
                    sunrise_game_data::text::Text::literal(needed.to_string()),
                ],
            )
        };
        if mod_api::__private::listened(mod_api::HookId::SleepStatusAnnounce) {
            let mut canceled = false;
            let dimension = self.common.dim.id();
            self.native_sleep_status_announce(
                dimension,
                sleeping,
                active,
                percentage,
                needed,
                skipping_night,
                &mut text,
                &mut canceled,
            );
            if canceled {
                return;
            }
        }
        if let Some(host) = self.server.lan.as_ref() {
            host.hub
                .overlay_message(sunrise_sim::lan::data::component_nbt(&text.component()));
        }
    }
    /// `SleepStatus.areEnoughSleeping` and `areEnoughDeepSleeping` over the
    /// host's player and the guests: of the players not in spectator mode,
    /// players_sleeping_percentage of them (at least one) are in bed, and as
    /// many have slept 100 ticks.
    pub fn enough_slept(&self) -> bool {
        let (active, sleeping, deep) = self.sleep_counts();
        let needed = self.sleepers_needed(active);
        sleeping >= needed && deep >= needed
    }
    /// `ServerLevel.tick`'s night skip: `SleepStatus.areEnoughSleeping` and
    /// `areEnoughDeepSleeping`, unless a mod's `sleep_skip_night` cancels it for this tick.
    pub(in crate::game) fn sleep_skips_night(&mut self) -> bool {
        let (active, sleeping, deep) = self.sleep_counts();
        let needed = self.sleepers_needed(active);
        if sleeping < needed || deep < needed {
            return false;
        }
        if !mod_api::__private::listened(mod_api::HookId::SleepSkipNight) {
            return true;
        }
        let percentage = self
            .common
            .ui
            .rules
            .int_at(sunrise_game_data::gamerules::vanilla::PLAYERS_SLEEPING_PERCENTAGE);
        let mut canceled = false;
        let dimension = self.common.dim.id();
        self.native_sleep_skip_night(dimension, percentage, sleeping, active, &mut canceled);
        // The listener's writes (`PlayerRef::set_sleep_timer`, a teleport) can leave too
        // few sleepers; the night is then not skipped, as the counts it answered are gone.
        !canceled && self.enough_slept()
    }
    /// Whether `SleepStatus.update` counts the guest: placed and playing, not
    /// a spectator, and, when a native mod ignored them, only once in bed.
    pub(in crate::game) fn guest_counts_for_sleep(
        &self,
        g: &sunrise_sim::lan::guest::Guest,
    ) -> bool {
        g.placed
            && g.participates_in_player_simulation()
            && !g.spectator()
            && (g.bed.is_some()
                || !self
                    .server
                    .lan
                    .as_ref()
                    .is_some_and(|host| host.sleep_ignored.contains(&g.uuid)))
    }
    /// `Player.tick`'s sleep counter, and `LivingEntity.tick`'s wake-up of a
    /// sleeper who died or whose bed is gone.
    pub fn guest_sleep_tick(&mut self, gi: usize, body: bool) {
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let (identity, bed, gone, uuid) = host.hub.guests.update(gi, |g| {
            let was_deep = g.sleep_timer >= 100;
            if body {
                g.sleep_timer =
                    sunrise_gameplay_common::game::tick_sleep_timer(g.sleep_timer, g.bed.is_some());
            }
            let deep_sleep = !was_deep && g.sleep_timer >= 100 && g.bed.is_some();
            let identity = deep_sleep.then(|| (g.uuid, g.name.clone()));
            (identity, g.bed, g.dead || g.health <= 0.0, g.uuid)
        });
        if let Some(head) = bed {
            let block = self.common.world.block(head[0], head[1], head[2]);
            let bed_exists = self.common.res.facts().get(block).name.ends_with("_bed");
            let modded = !mod_api::__private::mods().is_empty();
            let name = if modded && !gone {
                self.server
                    .lan
                    .as_ref()
                    .map(|host| host.hub.guests[gi].name.clone())
            } else {
                None
            };
            let mut keep_sleeping = !gone;
            let mut wake_immediately = gone;
            // Player.tick's bed-rule check precedes living-entity continuation.
            if keep_sleeping && bed_exists {
                let problem = self.sleep_problem(true, true);
                let mut may_continue = None;
                if let Some(name) = name.as_deref() {
                    self.native_can_continue_sleeping(
                        uuid,
                        name,
                        "minecraft:player",
                        head,
                        problem,
                        &mut may_continue,
                    );
                }
                keep_sleeping = may_continue.unwrap_or(problem.is_none());
            }
            if keep_sleeping {
                let problem = self.sleep_problem(bed_exists, false);
                let mut may_continue = None;
                if let Some(name) = name.as_deref() {
                    self.native_can_continue_sleeping(
                        uuid,
                        name,
                        "minecraft:player",
                        head,
                        problem,
                        &mut may_continue,
                    );
                }
                keep_sleeping = may_continue.unwrap_or(problem.is_none());
                wake_immediately = !keep_sleeping;
            }
            if !keep_sleeping {
                self.wake_guest(gi, wake_immediately, true);
            } else if let Some(host) = self.server.lan.as_mut() {
                // LivingEntity.tick: a sleeper's xRot is 0.
                host.hub.guests.update(gi, |g| {
                    g.pitch = 0.0;
                    if let Some(m) = g.motion.as_mut() {
                        m.pitch = 0.0;
                    }
                });
            }
        }
        if let Some((player, name)) = identity {
            self.native_player_deep_sleep(player, &name);
        }
    }
    /// `SleepStatus.update`'s counts over the host's player and the guests:
    /// the players not in spectator mode, how many are in bed, and how many
    /// of those have slept 100 ticks.
    pub fn sleep_counts(&self) -> (i64, i64, i64) {
        let (mut active, mut sleeping, mut deep) = (0i64, 0i64, 0i64);
        let mut count = |in_bed: bool, timer: i32| {
            active += 1;
            if in_bed {
                sleeping += 1;
                if timer >= 100 {
                    deep += 1;
                }
            }
        };
        if self.common.player.is_some() && !self.server.lan_acting_fake && !self.common.spectator()
        {
            count(self.common.sleeping.is_some(), self.common.sleep_timer);
        }
        if let Some(host) = self.server.lan.as_ref() {
            // A native mod's ignored players count only once in bed.
            for g in host
                .hub
                .guests
                .iter()
                .filter(|g| self.guest_counts_for_sleep(g))
            {
                count(g.bed.is_some(), g.sleep_timer);
            }
        }
        (active, sleeping, deep)
    }
    /// `SleepStatus.sleepersNeeded`, then a mod's `sleepers_needed` (at least 1).
    pub(in crate::game::server_side) fn sleepers_needed(&self, active: i64) -> i64 {
        let percentage = self
            .common
            .ui
            .rules
            .int_at(sunrise_game_data::gamerules::vanilla::PLAYERS_SLEEPING_PERCENTAGE);
        let mut needed = ((active * i64::from(percentage.max(0)) + 99) / 100).max(1);
        if percentage <= 100 && mod_api::__private::listened(mod_api::HookId::SleepersNeeded) {
            self.native_sleepers_needed(self.common.dim.id(), percentage, active, &mut needed);
            needed = needed.max(1);
        }
        needed
    }
    /// `ServerPlayer.stopSleepInBed` for guest `gi`, with the caller's flags.
    /// The mods hear it first, while the guest still lies in the bed.
    pub(in crate::game) fn wake_guest(
        &mut self,
        gi: usize,
        wake_immediately: bool,
        update_level: bool,
    ) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let g = &host.hub.guests[gi];
        let conn = g.conn;
        let player = g.uuid;
        if let Some(head) = g.bed
            && mod_api::__private::listened(mod_api::HookId::PlayerBedLeave)
        {
            let name = g.name.clone();
            self.native_player_bed_leave(player, &name, head, wake_immediately, update_level);
        }
        // The list may have changed, and a reconnect can share the departing guest's UUID.
        // Find the same connection; `wake` does nothing if the listener already woke it.
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let Some(gi) = host.hub.guests.connection_index(conn) else {
            return;
        };
        host.hub.wake(gi, wake_immediately);
        if host.hub.here(&host.hub.guests[gi])
            && let Some(head) = host.hub.guests.update(gi, |g| g.woke_from.take())
        {
            self.wake_guest_from(gi, head);
        }
    }
    /// `LivingEntity.stopSleeping` and `ServerPlayer.stopSleepInBed` for a
    /// guest who woke up from `head`: stand beside the bed facing it, and
    /// teleport the client there.
    pub(in crate::game::server_side) fn wake_guest_from(&mut self, gi: usize, head: [i32; 3]) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let yaw = host.hub.guests[gi].yaw;
        let Some((stand, yaw)) = self.bed_wake_pose(head, yaw) else {
            return;
        };
        let at = [stand.x, stand.y, stand.z];
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let alive = host.hub.guests.update(gi, |g| {
            g.position = at;
            g.previous_position = at;
            g.yaw = yaw;
            g.pitch = 0.0;
            g.moves.clear();
            if let Some(m) = g.motion.as_mut() {
                m.position = stand;
                m.velocity = Vec3::ZERO;
                m.yaw = yaw;
                m.pitch = 0.0;
                let _ = m.set_sleeping(false);
            }
            !g.dead
        });
        if alive {
            host.hub.teleport(gi, Some(at));
        }
    }
    /// `ServerLevel.wakeUpAllPlayers` for the guests.
    pub fn wake_guests(&mut self) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let count = host.hub.guests.len();
        for i in 0..count {
            self.wake_guest(i, false, false);
        }
    }
    pub fn wake_woken_guests(&mut self) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let count = host.hub.guests.len();
        for gi in 0..count {
            let Some(host) = self.server.lan.as_mut() else {
                return;
            };
            if host.hub.here(&host.hub.guests[gi])
                && let Some(head) = host.hub.guests.update(gi, |g| g.woke_from.take())
            {
                self.wake_guest_from(gi, head);
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/sleep_skip_night_tests.rs"]
mod sleep_skip_night_tests;

#[cfg(test)]
#[path = "tests/sleep_status_announce_tests.rs"]
mod sleep_status_announce_tests;

#[cfg(test)]
#[path = "tests/sleepers_needed_tests.rs"]
mod sleepers_needed_tests;
