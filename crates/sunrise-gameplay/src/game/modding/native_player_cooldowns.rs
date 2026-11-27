//! A player's cooldowns for native server mods, through a `PlayerRef`: item
//! cooldowns (`Player.getCooldowns()`, vanilla's `ItemCooldowns`), the
//! experience orb pickup delay (`Player.takeXpDelay`) and the warden warning
//! cooldown (`WardenSpawnTracker.cooldownTicks`). A cooldown group
//! is an item id unless the item's `use_cooldown` component names another
//! (`minecraft:ender_pearl`, a mod's `mymod:wands`...);
//! `item_cooldown_group` gives a stack's. Reads return None for a stale
//! handle or a player who isn't online here; inside another player's turn
//! they still read a resting guest. Item cooldown writes reach the current player
//! record even at zero health, but not one suspended by a nested turn.
use crate::game::ServerGame;
use mod_api::{ItemCooldown, PlayerRef};
use std::sync::Arc;
use sunrise_items::cooldowns::Cooldowns;

impl ServerGame<'_> {
    /// The cooldowns of `player`, where they are kept now: a guest's own
    /// outside their turn, the game's while theirs is swapped in (as
    /// `player` finds the player).
    fn native_cooldowns_of(&self, player: PlayerRef) -> Option<&Cooldowns> {
        match self.player_handle_read(player)? {
            sunrise_gameplay_common::game::native_server::PlayerRead::Acting { .. } => {
                Some(&self.common.cooldowns)
            }
            sunrise_gameplay_common::game::native_server::PlayerRead::Guest(g) => Some(
                self.server
                    .lan
                    .as_ref()?
                    .play
                    .get(&g.conn)
                    .map_or(&EMPTY, |p| &p.cooldowns),
            ),
        }
    }

    /// Every group the player waits on and its ticks left, ordered by
    /// group.
    pub fn player_item_cooldowns(&self, player: PlayerRef) -> Option<Vec<ItemCooldown>> {
        let mut out: Vec<ItemCooldown> = self
            .native_cooldowns_of(player)?
            .remaining()
            .filter(|(_, left)| *left > 0)
            .map(|(group, left)| ItemCooldown {
                group: group.to_string(),
                ticks_left: left,
            })
            .collect();
        out.sort_by(|a, b| a.group.cmp(&b.group));
        Some(out)
    }
    /// `ItemCooldowns.isOnCooldown`: whether `group` still has time to run.
    pub fn player_has_item_cooldown(&self, player: PlayerRef, group: &str) -> Option<bool> {
        Some(self.native_cooldowns_of(player)?.is_on_cooldown(group))
    }
    /// `ItemCooldowns.getCooldownPercent(group, 0)`: what is left of the
    /// group's cooldown, from 1 (just started) to 0 (none).
    pub(in crate::game) fn player_item_cooldown_percent(
        &self,
        player: PlayerRef,
        group: &str,
    ) -> Option<f32> {
        Some(self.native_cooldowns_of(player)?.percent(group, 0.0))
    }
    /// `ItemCooldowns.addCooldown(group, ticks)`: starts (or restarts) the
    /// group's cooldown, and the player's client draws it on every item of the
    /// group (`ServerItemCooldowns.onCooldownStarted`).
    pub fn player_set_item_cooldown(&mut self, player: PlayerRef, group: &str, ticks: i32) -> bool {
        let group: Arc<str> = group.into();
        self.native_as_resident_player(player, |game| game.add_cooldown(group, ticks))
    }
    /// `ItemCooldowns.removeCooldown(group)`: ends the group's cooldown now,
    /// on the client too (`ServerItemCooldowns.onCooldownEnded`).
    pub fn player_remove_item_cooldown(&mut self, player: PlayerRef, group: &str) -> bool {
        let group: Arc<str> = group.into();
        self.native_as_resident_player(player, |game| {
            game.cooldowns.remove(&group);
            if game.server.lan_acting.is_some() {
                game.server.guest_cooldowns.push((group, 0));
            }
        })
    }
    /// `Player.takeXpDelay`: the ticks before the player can take the next
    /// experience orb.
    pub fn player_exp_cooldown(&self, player: PlayerRef) -> Option<u32> {
        match self.player_handle_read(player)? {
            sunrise_gameplay_common::game::native_server::PlayerRead::Acting { .. } => {
                Some(self.server.xp_delay)
            }
            sunrise_gameplay_common::game::native_server::PlayerRead::Guest(g) => Some(
                self.server
                    .lan
                    .as_ref()?
                    .play
                    .get(&g.conn)
                    .map_or(0, |p| p.xp_delay),
            ),
        }
    }
    /// Sets `Player.takeXpDelay`: no orb is taken for `ticks` ticks. The
    /// change is not reported to `player_exp_cooldown_change`.
    pub fn player_set_exp_cooldown(&mut self, player: PlayerRef, ticks: u32) -> bool {
        self.native_as_player_handle(player, |game| game.server.xp_delay = ticks)
    }
    /// Sets the player's `WardenSpawnTracker.cooldownTicks`: sculk shriekers
    /// they set off don't warn for `ticks` ticks. The level's players share
    /// one tracker here, the integrated server owner's (the shriekers read it
    /// for whoever sets them off), so this is every player's.
    /// False, changing nothing, for anyone but the tracker's owner until each player has their own.
    pub fn player_set_warden_warning_cooldown(&mut self, player: PlayerRef, ticks: u32) -> bool {
        // The tracker loads from and saves to the level's player data, the
        // owner's: in another player's turn theirs is in its place.
        let owner = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.owner.as_ref())
            .map(|owner| owner.uuid);
        if owner != Some(player.uuid())
            || self
                .server
                .lan_acting_uuid
                .is_some_and(|acting| Some(acting) != owner)
        {
            return false;
        }
        self.native_as_player_handle(player, |game| {
            // Loaded first, or the next tick's load would overwrite it.
            game.load_warden_tracker();
            game.server.sculk.tracker.cooldown_ticks = i32::try_from(ticks).unwrap_or(i32::MAX);
        })
    }
}

/// What a guest with no play state yet waits on: nothing.
static EMPTY: std::sync::LazyLock<Cooldowns> = std::sync::LazyLock::new(Cooldowns::default);
