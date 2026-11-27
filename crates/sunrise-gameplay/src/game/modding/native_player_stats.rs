//! A player's statistics and recipe book for native server mods, through a
//! `PlayerRef` (docs/modding/src/how-to/native-mods.md): `ServerStatsCounter` and
//! `ServerRecipeBook`. Reads return None for a stale handle or a player who
//! isn't online here. Writes return false (or None) for those, for a dead
//! player, and, inside another player's turn, for anyone but that player.
use super::{advancements::progress_snapshot, lan_actions_server::CombatPlayer, stats::Stats};
use core::ops::ControlFlow;
use mod_api::{
    AdvancementError, AdvancementProgress, EntityReadError, Identifier, PlayerRef, StatType,
};
use sunrise_gameplay_common::game::{native_server::PlayerRead, transactions::DeferredEffect};
use sunrise_gameplay_common::stats::StatValues;
use sunrise_sim::advancements::Progress;

use super::native_namespaced_id::namespaced;

impl crate::game::ServerGame<'_> {
    /// The player's statistics, including the counter lent to a nested turn.
    fn native_player_stats(&self, player: PlayerRef) -> Option<Option<&Stats>> {
        match self.player_handle_read(player) {
            Some(PlayerRead::Guest(g)) => Some(
                self.server
                    .lan
                    .as_ref()?
                    .play
                    .get(&g.conn)
                    .map(|p| &p.stats),
            ),
            Some(PlayerRead::Acting { .. }) => Some(Some(&self.common.stats)),
            None => {
                let entity = player.entity();
                if self.server.lan_displaced_players.0.is_empty()
                    || self.common.native_lifecycle.epoch()? != entity.epoch()
                {
                    return None;
                }
                let CombatPlayer::Displaced(depth, _) = self.combat_player(entity.id())? else {
                    return None;
                };
                let frame = &self.server.lan_displaced_players.0[depth];
                (frame.uuid == Some(player.uuid())).then_some(Some(&frame.stats))
            }
        }
    }

    /// The value of statistic `stat_type` for `key` (`minecraft:stone`,
    /// `minecraft:jump`...), as `ServerStatsCounter.getValue`: 0 when never
    /// counted.
    pub fn player_stat(&self, player: PlayerRef, stat_type: StatType, key: &str) -> Option<i32> {
        let ty = stat_type.path();
        let stats = self.native_player_stats(player)?;
        Some(stats.map_or(0, |s| s.get(ty, &namespaced(key))))
    }
    /// `ServerStatsCounter.toJson` for this player's live counter.
    pub(in crate::game) fn player_statistics(&self, player: PlayerRef) -> Option<String> {
        if !self.common.is_logical_server() {
            return None;
        }
        let json = self
            .native_player_stats(player)?
            .map_or_else(|| Stats::default().to_json(), Stats::to_json);
        Some(json.to_string())
    }

    /// The player's live advancement progress, including one lent to a nested turn.
    /// `None` while the engine has not loaded that player's advancements.
    fn native_player_advancements(&self, player: PlayerRef) -> Option<&Progress> {
        match self.player_handle_read(player) {
            Some(PlayerRead::Guest(g)) => g.advancements_loaded.then_some(&g.advancements),
            Some(PlayerRead::Acting { .. }) => {
                self.common.adv.loaded.then_some(&self.common.adv.progress)
            }
            None => {
                let entity = player.entity();
                if self.server.lan_displaced_players.0.is_empty()
                    || self.common.native_lifecycle.epoch()? != entity.epoch()
                {
                    return None;
                }
                let CombatPlayer::Displaced(depth, _) = self.combat_player(entity.id())? else {
                    return None;
                };
                let frame = &self.server.lan_displaced_players.0[depth];
                if frame.uuid != Some(player.uuid()) || !frame.adv_loaded {
                    return None;
                }
                Some(
                    &self
                        .server
                        .lan
                        .as_ref()?
                        .hub
                        .guest(frame.record_conn)?
                        .advancements,
                )
            }
        }
    }

    /// `ServerPlayer.getAdvancements().getOrStartProgress(holder)` for one loaded
    /// advancement.
    pub(in crate::game) fn player_advancement_progress(
        &self,
        player: PlayerRef,
        id: &Identifier,
    ) -> Option<AdvancementProgress> {
        if !self.common.is_logical_server() {
            return None;
        }
        let progress = self.native_player_advancements(player)?;
        let advancement = self.common.adv.tree.get(id.as_str())?;
        Some(progress_snapshot(advancement, progress))
    }

    /// `PlayerAdvancements.award(holder, criterion)` (or all remaining criteria when
    /// `criterion` is `None`, matching `AdvancementCommands.Action.GRANT`).
    pub(in crate::game) fn player_award_advancement(
        &mut self,
        player: PlayerRef,
        id: &Identifier,
        criterion: Option<&str>,
    ) -> Result<bool, AdvancementError> {
        self.player_change_advancement(player, id, criterion, true)
    }

    /// `PlayerAdvancements.revoke(holder, criterion)` (or all obtained criteria when
    /// `criterion` is `None`, matching `AdvancementCommands.Action.REVOKE`).
    pub(in crate::game) fn player_revoke_advancement(
        &mut self,
        player: PlayerRef,
        id: &Identifier,
        criterion: Option<&str>,
    ) -> Result<bool, AdvancementError> {
        self.player_change_advancement(player, id, criterion, false)
    }

    fn player_change_advancement(
        &mut self,
        player: PlayerRef,
        id: &Identifier,
        criterion: Option<&str>,
        grant: bool,
    ) -> Result<bool, AdvancementError> {
        if !self.common.is_logical_server() {
            return Err(AdvancementError::NotServer);
        }
        if !self.native_player_writable(player) {
            return Err(AdvancementError::PlayerUnavailable);
        }
        let target = self
            .native_entity_target(player.entity())
            .ok_or(AdvancementError::PlayerUnavailable)?;
        let tree = self.advancement_tree();
        let Some(index) = tree.find_id(id.as_str()) else {
            return Err(AdvancementError::UnknownAdvancement);
        };
        if let Some(name) = criterion
            && !tree.list[index].criteria.iter().any(|c| c.name == name)
        {
            return Err(AdvancementError::UnknownCriterion);
        }
        if self.common.transactions.last().is_some() {
            let progress = self
                .native_player_advancements(player)
                .ok_or(AdvancementError::PlayerUnavailable)?;
            let snapshot = progress_snapshot(&tree.list[index], progress);
            let would_change = match (grant, criterion) {
                (true, Some(name)) => snapshot
                    .criteria
                    .iter()
                    .any(|c| c.name == name && c.obtained_at.is_none()),
                (true, None) => !snapshot.done,
                (false, Some(name)) => snapshot
                    .criteria
                    .iter()
                    .any(|c| c.name == name && c.obtained_at.is_some()),
                (false, None) => snapshot.criteria.iter().any(|c| c.obtained_at.is_some()),
            };
            self.defer_effect(DeferredEffect::ChangeAdvancement {
                player,
                id: id.as_str().to_owned(),
                criterion: criterion.map(str::to_owned),
                grant,
            });
            return Ok(would_change);
        }
        self.for_player(target, |game| {
            game.load_advancements();
            let Some(index) = game.common.adv.tree.find_id(id.as_str()) else {
                return Err(AdvancementError::UnknownAdvancement);
            };
            if let Some(name) = criterion
                && !game.common.adv.tree.list[index]
                    .criteria
                    .iter()
                    .any(|c| c.name == name)
            {
                return Err(AdvancementError::UnknownCriterion);
            }
            Ok(if grant {
                game.grant_advancement(index, criterion)
            } else {
                game.revoke_advancement(index, criterion)
            })
        })
        .ok_or(AdvancementError::PlayerUnavailable)?
    }

    /// Apply a deferred `PlayerRef::award_advancement` / `revoke_advancement` at root commit.
    pub(in crate::game) fn apply_deferred_advancement_change(
        &mut self,
        player: PlayerRef,
        id: &str,
        criterion: Option<&str>,
        grant: bool,
    ) {
        if !self.common.is_logical_server() || !self.native_player_writable(player) {
            return;
        }
        let Some(target) = self.native_entity_target(player.entity()) else {
            return;
        };
        let _ = self.for_player(target, |game| {
            game.load_advancements();
            let Some(index) = game.common.adv.tree.find_id(id) else {
                return;
            };
            if let Some(name) = criterion
                && !game.common.adv.tree.list[index]
                    .criteria
                    .iter()
                    .any(|c| c.name == name)
            {
                return;
            }
            if grant {
                game.grant_advancement(index, criterion);
            } else {
                game.revoke_advancement(index, criterion);
            }
        });
    }

    /// Decode with the server's registries before entering the player's turn. Inside a
    /// transaction the replacement waits for the root commit, as the game's own deferred
    /// statistics do, so counters and criterion scores never outlive an abort apart.
    pub(in crate::game) fn player_set_statistics(
        &mut self,
        player: PlayerRef,
        snapshot: &str,
    ) -> bool {
        if !self.common.is_logical_server() {
            return false;
        }
        let Some(values) = Stats::decode_snapshot(snapshot, |registry, id| {
            self.stat_registry_contains(registry, id)
        }) else {
            return false;
        };
        if self.common.transactions.last().is_some() {
            if !self.native_player_writable(player) {
                return false;
            }
            self.defer_effect(DeferredEffect::RestoreStatistics(player, Box::new(values)));
            return true;
        }
        self.restore_player_statistics(player, values)
    }
    /// The replacement itself, in the player's turn.
    pub(in crate::game) fn restore_player_statistics(
        &mut self,
        player: PlayerRef,
        values: StatValues,
    ) -> bool {
        self.native_as_player_handle(player, |game| {
            // A local counter loads lazily; guest turns already own their counter.
            if game.server.lan_acting.is_none() {
                game.load_stats();
            }
            game.stats.replace_values(values);
        })
    }
    /// Whether `native_as_player_handle` would run for `player` now.
    fn native_player_writable(&self, player: PlayerRef) -> bool {
        match self.player_handle_read(player) {
            None => false,
            Some(PlayerRead::Acting { .. }) => true,
            Some(PlayerRead::Guest(g)) => !g.dead,
        }
    }

    /// `Player.awardStat(stat, amount)`: adds `amount`, as the game does
    /// for its own counts, so `player_stat_award` hears it and may rewrite or cancel
    /// it, and scoreboard criteria follow. False for a player it can't
    /// reach.
    pub(in crate::game) fn player_award_stat(
        &mut self,
        player: PlayerRef,
        stat_type: StatType,
        key: &str,
        amount: i32,
    ) -> bool {
        let ty = stat_type.path();
        let key = namespaced(key);
        self.native_player_current(player)
            && self.native_as_player_handle(player, |game| game.award_stat(ty, &key, amount))
    }
    /// `Player.resetStat`: sets the statistic back to 0, which scoreboard
    /// criteria follow. False for a player it can't reach.
    pub(in crate::game) fn player_reset_stat(
        &mut self,
        player: PlayerRef,
        stat_type: StatType,
        key: &str,
    ) -> bool {
        let ty = stat_type.path();
        let key = namespaced(key);
        self.native_player_current(player)
            && self.native_as_player_handle(player, |game| game.stats.reset(ty, &key))
    }

    /// Whether the player's recipe book knows `recipe`
    /// (`ServerRecipeBook.contains`).
    pub fn player_has_discovered_recipe(&self, player: PlayerRef, recipe: &str) -> Option<bool> {
        if !self.native_player_current(player) {
            return None;
        }
        let id = namespaced(recipe);
        Some(match self.native_player_resting(player) {
            Some(g) => g.recipe_book.recipes.contains(&id),
            None => self.common.level.recipe_book.recipes.contains(&id),
        })
    }
    /// `ServerPlayer.awardRecipes`, as `/recipe give` does: the recipes
    /// unlock in the player's book, with their toasts and the
    /// `player_recipe_discovered` hook, and the client hears of them.
    /// Unknown ids are skipped. The number of recipe displays added, None
    /// for a player it can't reach.
    pub(in crate::game) fn player_award_recipes(
        &mut self,
        player: PlayerRef,
        recipes: &[&str],
    ) -> Option<i32> {
        self.native_player_change_recipes(player, true, recipes)
    }
    /// `ServerPlayer.resetRecipes`, as `/recipe take` does. The number of
    /// recipe displays taken out, None for a player it can't reach.
    pub(in crate::game) fn player_reset_recipes(
        &mut self,
        player: PlayerRef,
        recipes: &[&str],
    ) -> Option<i32> {
        self.native_player_change_recipes(player, false, recipes)
    }
    fn native_player_change_recipes(
        &mut self,
        player: PlayerRef,
        give: bool,
        recipes: &[&str],
    ) -> Option<i32> {
        if !self.native_player_current(player) {
            return None;
        }
        // `awardRecipes` takes recipes the server has: an id no recipe has
        // would otherwise go into the book.
        let index = &self.stations().recipe_index;
        let ids: Vec<String> = recipes
            .iter()
            .map(|r| namespaced(r))
            .filter(|id| index.contains(id))
            .collect();
        let mut count = 0;
        self.native_as_player_handle(player, |game| {
            count = game.change_recipes(give, &ids);
        })
        .then_some(count)
    }
}

impl crate::game::ServerGame<'_> {
    /// Whether `player` still names the same player on this server.
    pub(in crate::game) fn native_player_current(&self, player: PlayerRef) -> bool {
        self.player_handle_read(player).is_some()
    }

    /// The guest record `player` reads from outside their turn, or None
    /// when their state is in the game (they act, or they are the host).
    pub(in crate::game) fn native_player_resting(
        &self,
        player: PlayerRef,
    ) -> Option<&sunrise_sim::lan::guest::Guest> {
        match self.player_handle_read(player)? {
            PlayerRead::Guest(guest) => Some(guest),
            PlayerRead::Acting { .. } => None,
        }
    }

    /// Every recipe the player's recipe book knows (`ServerRecipeBook.recipes`), qualified ids in
    /// order.
    pub(in crate::game) fn player_visit_discovered_recipes(
        &self,
        player: PlayerRef,
        visit: &mut dyn FnMut(&str) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, EntityReadError> {
        if !self.common.is_logical_server() {
            return Err(EntityReadError::Unavailable);
        }
        let book = match self
            .player_handle_read(player)
            .ok_or(EntityReadError::Stale)?
        {
            PlayerRead::Guest(guest) => &guest.recipe_book.recipes,
            PlayerRead::Acting { .. } => &self.common.level.recipe_book.recipes,
        };
        Ok(book.iter().map(String::as_str).try_for_each(visit))
    }
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;
