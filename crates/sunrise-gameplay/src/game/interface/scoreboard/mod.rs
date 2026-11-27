//! The scoreboard in the running game: saved data, criteria that follow the
//! player (health, food, xp, deaths, kills, statistics), score holders for
//! entities, the selector options `scores=`, `team=` and `tag=`, the
//! commands' [`Env`] and the sidebar.
use super::chat_common::PLAYER_NAME;
use mod_api::EntityTags;
use std::collections::{HashMap, HashSet};
use sunrise_core::nbt::Tag;
use sunrise_gameplay_common::game::commands_common::Target;
use sunrise_gameplay_common::game::scoreboard_common::{
    Recorded, pet_team, root_owner, tame_owner,
};
use sunrise_gameplay_common::stats::{StatScoreChange, StatScoreOp};
#[allow(unused_imports)]
use {
    sunrise_command::Context, sunrise_command::args::IntRange, sunrise_command::args::Selector,
    sunrise_game_data::scoreboard as sb, sunrise_game_data::scoreboard::Comp,
    sunrise_game_data::scoreboard::Scoreboard, sunrise_game_data::scoreboard::literal,
    sunrise_game_data::text::Text, sunrise_level::profile_lookup::TabPlayer,
    sunrise_sim::lan::guest::Guest, sunrise_sim::lan::playerdata::ints_uuid, sunrise_sim::save,
    sunrise_sim::save::uuid_from_text, sunrise_sim::save::uuid_text, sunrise_sim::scoreboard_cmd,
    sunrise_sim::scoreboard_cmd::Env, sunrise_sim::scoreboard_cmd::Holder,
};

pub(in crate::game::server_side) fn in_range(v: i32, (lo, hi): IntRange) -> bool {
    lo.is_none_or(|l| v >= l) && hi.is_none_or(|h| v <= h)
}

/// The id of a target whose scoreboard name is a UUID it is given when first asked for (an
/// orb's registered identity included).
pub(in crate::game::server_side) fn gives_uuid(t: Target) -> Option<u32> {
    match t {
        Target::Mob(id)
        | Target::Item(id)
        | Target::Stand(id)
        | Target::Frame(id)
        | Target::Painting(id)
        | Target::Boat(id)
        | Target::Cart(id)
        | Target::Crystal(id)
        | Target::Dragon(id)
        | Target::Tnt(id)
        | Target::Cushion(id)
        | Target::Falling(id)
        | Target::Rocket(id)
        | Target::Trident(id)
        | Target::Other(id)
        | Target::Fangs(id)
        | Target::Cloud(id)
        | Target::Bolt(id)
        | Target::General(id)
        | Target::Hook(id)
        | Target::Orb(id) => Some(id),
        _ => None,
    }
}

impl crate::game::ServerGame<'_> {
    pub(in crate::game::server_side) fn scoreboard_path(&self) -> std::path::PathBuf {
        self.server.world_root.join("data/minecraft/scoreboard.dat")
    }
    /// `ServerLevel.getWorldBorder`'s file: each dimension's own
    /// `data/minecraft/world_border.dat`.
    pub(in crate::game::server_side) fn border_path(&self, dim: super::Dim) -> std::path::PathBuf {
        save::dimension_dir(&self.server.world_root, dim.id())
            .join("data/minecraft/world_border.dat")
    }
    /// The border `dim`'s level saved. The Overworld also reads the root file older Sunrise
    /// builds wrote for their one shared border; the other levels start from the default.
    pub(in crate::game::server_side) fn load_border(
        &self,
        dim: super::Dim,
    ) -> sunrise_level::worldborder::WorldBorder {
        let mut root = save::read_dat(&self.border_path(dim));
        if root.is_none() && dim == super::Dim::Overworld {
            root = save::read_dat(
                &self
                    .server
                    .world_root
                    .join("data/minecraft/world_border.dat"),
            );
        }
        if let Some(data) = root.as_ref().and_then(|root| root.get("data")) {
            match sunrise_level::worldborder::WorldBorder::unpack(data) {
                Ok(border) => return border,
                Err(error) => log::warn!("invalid saved world border in {}: {error}", dim.id()),
            }
        }
        sunrise_level::worldborder::WorldBorder::default()
    }
    /// Save the border of the level in place (`SavedData`, one per level).
    pub(in crate::game::server_side) fn save_border(&self) {
        if !self.common.score.loaded || !self.common.is_logical_server() {
            return;
        }
        if self.common.border != sunrise_level::worldborder::WorldBorder::default()
            && let Err(e) = save::write_dat(
                &self.border_path(self.common.dim),
                Tag::compound([
                    ("data", self.common.border.pack()),
                    ("DataVersion", Tag::Int(sunrise_world::DATA_VERSION)),
                ]),
            )
        {
            log::warn!("the world border of {} was not written: {e}", self.dim.id());
        }
    }
    /// `ScoreboardSaveData` read from `data/minecraft/scoreboard.dat`.
    pub fn load_scoreboard(&mut self) {
        if self.common.score.loaded || !self.common.is_logical_server() {
            return;
        }
        self.common.score.loaded = true;
        if let Some(root) = save::read_dat(&self.scoreboard_path())
            && let Some(data) = root.get("data")
        {
            self.common.score.board = Scoreboard::unpack(data);
        }
        self.common.score.storage =
            sunrise_sim::command_storage::CommandStorage::load(&self.server.world_root);
        self.common.score.stopwatches =
            sunrise_sim::stopwatches::Stopwatches::load(&self.server.world_root);
        self.sync_score_registries();
    }
    pub fn save_scoreboard(&mut self) {
        if !self.common.score.loaded || !self.common.is_logical_server() {
            return;
        }
        self.common.score.storage.save(&self.server.world_root);
        self.common.score.stopwatches.save(&self.server.world_root);
        // The border is saved with its level (`save_level`).
        let path = self.scoreboard_path();
        let board = &self.common.score.board;
        if board == &Scoreboard::default() && !path.exists() {
            return;
        }
        if let Err(e) = save::write_dat(
            &path,
            Tag::compound([
                ("data", board.pack()),
                ("DataVersion", Tag::Int(sunrise_world::DATA_VERSION)),
            ]),
        ) {
            log::warn!("the scoreboard was not written: {e}");
        }
    }
    /// The names commands complete: objectives, teams and tracked holders.
    pub(in crate::game) fn sync_score_registries(&mut self) {
        let board = &self.common.score.board;
        let reg = &mut self.common.ui.registries;
        reg.objectives = board.objectives.iter().map(|o| o.name.clone()).collect();
        // Trigger objectives: the locks `/trigger` and `scoreboard players enable` complete from.
        reg.trigger_scores.clear();
        for o in board.objectives.iter().filter(|o| o.criteria == "trigger") {
            let locks = board
                .scores
                .iter()
                .filter_map(|(holder, s)| s.get(&o.name).map(|s| (holder.clone(), s.locked)))
                .collect();
            reg.trigger_scores.insert(o.name.clone(), locks);
        }
        reg.trigger_objectives = reg
            .trigger_scores
            .iter()
            .filter(|(_, locks)| locks.get(PLAYER_NAME) == Some(&false))
            .map(|(o, _)| o.clone())
            .collect();
        reg.teams = board.teams.iter().map(|t| t.name.clone()).collect();
        reg.tracked = board.tracked();
        reg.storages = self.common.score.storage.keys();
        reg.stopwatches = self.common.score.stopwatches.ids();
    }

    // ---- score holders ---------------------------------------------------

    /// `ScoreHolder.getScoreboardName`: the player's name, an entity's UUID.
    pub fn holder_name(&mut self, t: Target) -> Option<String> {
        match gives_uuid(t) {
            Some(id) => Some(uuid_text(self.uuid_for(id))),
            None => self.existing_holder_name(t),
        }
    }
    /// [`holder_name`](Self::holder_name) without giving a UUID to an entity that has none: its
    /// name on the board, if it could have a score yet.
    pub(in crate::game::server_side) fn existing_holder_name(&self, t: Target) -> Option<String> {
        match t {
            Target::Player => Some(self.acting_player_name().to_owned()),
            Target::Guest(conn) => self.command_guest(conn).map(|g| g.name.clone()),
            // It keeps its UUID with itself.
            Target::Spawner(id) => self
                .common
                .ominous_spawners
                .iter()
                .find(|s| s.id == id)
                .map(|s| uuid_text(s.uuid)),
            Target::Knot(id) => self.common.persist.uuids.get(&id).map(|&u| uuid_text(u)),
            // Projectiles have no UUID here.
            Target::Projectile(_) | Target::DragonPart(_) => None,
            other => {
                let id = gives_uuid(other)?;
                self.known_resident_entity_uuid_at(self.common.dim.key(), id)
                    .or_else(|| self.common.persist.uuids.get(&id).copied())
                    .map(uuid_text)
            }
        }
    }
    pub(in crate::game::server_side) fn holder_for(&mut self, t: Target) -> Option<Holder> {
        let name = self.holder_name(t)?;
        let shown = feedback_name(self.target_name(t), &name);
        Some(Holder {
            display: Some(literal(&shown.plain())),
            text: shown,
            name,
        })
    }
    /// The entity with the given scoreboard name.
    pub(in crate::game::server_side) fn target_of_holder(&self, name: &str) -> Option<Target> {
        // `PlayerList.getPlayerByName` (any case); a mod's nicknames are not scoreboard names.
        if self.common.player.is_some() && name.eq_ignore_ascii_case(self.acting_player_name()) {
            return Some(Target::Player);
        }
        if let Some(g) = self
            .command_players()
            .find(|g| g.name.eq_ignore_ascii_case(name))
        {
            return Some(Target::Guest(g.conn));
        }
        let uuid = uuid_from_text(name).filter(|_| name.len() == 36)?;
        self.target_with_uuid(uuid)
    }
    /// The saved compound read by commands and native entity handles.
    pub(in crate::game) fn saved_entity_compound(&self, t: Target) -> Option<&Tag> {
        match t {
            Target::Mob(id)
            | Target::Item(id)
            | Target::Stand(id)
            | Target::Frame(id)
            | Target::Painting(id)
            | Target::Boat(id)
            | Target::Cart(id)
            | Target::Tnt(id)
            | Target::Cushion(id)
            | Target::Orb(id)
            | Target::Projectile(id)
            | Target::Crystal(id)
            | Target::Dragon(id)
            | Target::Rocket(id)
            | Target::General(id)
            | Target::Trident(id)
            | Target::Other(id)
            | Target::Fangs(id)
            | Target::Cloud(id)
            | Target::Bolt(id)
            | Target::Hook(id) => self.common.persist.raw.get(&id),
            Target::DragonPart(_) => None,
            Target::Player => self.common.level.player_extra.as_ref(),
            // A server's player: the saved compound the guest's file is written from.
            Target::Guest(conn) => self.command_guest(conn).and_then(|g| g.saved.as_ref()),
            Target::Spawner(id) => self
                .common
                .ominous_spawners
                .iter()
                .find(|s| s.id == id)
                .and_then(|s| s.raw.as_deref()),
            Target::Falling(id) => self
                .common
                .falling
                .iter()
                .find(|f| f.id == id)
                .and_then(|f| f.saved.as_ref().map(|s| &s.1)),
            // A natural knot keeps no saved compound: it has no tags.
            Target::Knot(_) => None,
        }
    }

    /// The saved `Tags` list read by commands and native entity handles.
    pub(in crate::game::server_side) fn saved_entity_tags(&self, t: Target) -> EntityTags<'_> {
        let raw = self.saved_entity_compound(t);
        let tags = match raw.and_then(|r| r.get("Tags")) {
            Some(Tag::List(tags)) => tags.as_slice(),
            _ => &[],
        };
        mod_api::__private::game::borrow_entity_tags(tags)
    }

    /// The lossy UTF-8 presentation used by `/tag` and its existing write paths.
    pub fn tags_of(&self, t: Target) -> Vec<String> {
        self.saved_entity_tags(t)
            .iter_utf16()
            .map(String::from_utf16_lossy)
            .collect()
    }
    /// The selector options that need the scoreboard or entity data: which
    /// of `candidates` pass `scores=`, `team=` and `tag=`.
    pub(in crate::game) fn selector_extra_filter(
        &mut self,
        candidates: &[Target],
        s: &Selector,
    ) -> HashSet<Target> {
        let mut pass = HashSet::new();
        let needs_holder = !s.scores.is_empty() || !s.teams.is_empty();
        for &t in candidates {
            let mut ok = true;
            if needs_holder {
                let holder = self.holder_name(t);
                for (objective, range) in &s.scores {
                    let value = holder
                        .as_deref()
                        .and_then(|h| self.common.score.board.score(h, objective))
                        .map(|score| score.value);
                    if !value.is_some_and(|v| in_range(v, *range)) {
                        ok = false;
                    }
                }
                // TamableAnimal.getTeam: a tamed pet with no team of its own
                // answers with its root owner's.
                let team = match t {
                    Target::Mob(id) => self.mob_team(id).map(|t| t.name.clone()),
                    _ => holder
                        .as_deref()
                        .and_then(|h| self.common.score.board.players_team(h))
                        .map(|t| t.name.clone()),
                };
                for (want, negated) in &s.teams {
                    let matches = match &team {
                        Some(name) => name == want,
                        None => want.is_empty(),
                    };
                    if matches == *negated {
                        ok = false;
                    }
                }
            }
            if !s.tags.is_empty() {
                let tags = self.tags_of(t);
                for (tag, negated) in &s.tags {
                    let has = if tag.is_empty() {
                        // EntitySelectorOptions' empty-name predicate (lambda$bootStrap$43).
                        tags.is_empty()
                    } else {
                        tags.contains(tag)
                    };
                    if has == *negated {
                        ok = false;
                    }
                }
            }
            if ok && !s.nbt.is_empty() {
                // NbtPredicate: the entity's saved data contains the pattern.
                let data = self.command_target_nbt(t);
                for (pattern, negated) in &s.nbt {
                    let matches = match (&data, sunrise_core::nbt::parse_snbt(pattern)) {
                        (Some(d), Ok((p, _))) => {
                            sunrise_game_data::nbt_path::compare_nbt(&p, Some(d))
                        }
                        _ => false,
                    };
                    if matches == *negated {
                        ok = false;
                    }
                }
            }
            if ok && !s.predicates.is_empty() {
                // EntityPredicates: the datapack predicate with the entity as
                // `this` and the entity's position as origin.
                let view = self.entity_view_of(t);
                let origin = self.target_position(t).unwrap_or_default();
                for (id, negated) in &s.predicates {
                    // EntitySelectorOptions rejects a missing registry entry
                    // before applying the predicate's negation.
                    let Some(json) = self.server.fns.0.store.predicates.get(id) else {
                        ok = false;
                        break;
                    };
                    let matches = self.conditions_hold(json, Some(&view), origin);
                    if matches == *negated {
                        ok = false;
                    }
                }
            }
            if ok && !s.advancements.is_empty() {
                ok = self.advancements_match(t, &s.advancements);
            }
            if ok {
                pass.insert(t);
            }
        }
        pass
    }

    /// `advancements=` of a selector, against the selected player's progress.
    pub(in crate::game::server_side) fn advancements_match(
        &self,
        target: Target,
        wanted: &[(String, sunrise_command::args::AdvCondition)],
    ) -> bool {
        use sunrise_command::args::AdvCondition;
        let progress = match target {
            Target::Player => &self.common.adv.progress,
            Target::Guest(conn) => match self.command_guest(conn) {
                Some(guest) => &guest.advancements,
                None => return false,
            },
            _ => return false,
        };
        wanted.iter().all(|(id, cond)| {
            let Some(a) = self.common.adv.tree.get(id) else {
                return false;
            };
            match cond {
                AdvCondition::Done(want) => progress.done(a) == *want,
                AdvCondition::Criteria(list) => list.iter().all(|(c, want)| {
                    a.criteria.iter().any(|criterion| criterion.name == *c)
                        && progress.has(id, c) == *want
                }),
            }
        })
    }

    // ---- criteria ----------------------------------------------------------

    /// `ServerPlayer.awardStat` and `ServerPlayer.resetStat`: the objectives of
    /// each changed statistic's criteria follow it for the player `name`.
    pub(in crate::game::server_side) fn stat_criteria_changed(
        &mut self,
        name: &str,
        events: Vec<StatScoreChange>,
    ) {
        if self.common.score.board.objectives.is_empty() {
            return;
        }
        for (ty, key, op) in events {
            let criteria = sb::stat_criteria(&ty, &key);
            if self.common.score.board.has_objectives_for(&criteria) {
                self.journal_scoreboard();
                self.common
                    .score
                    .board
                    .for_all_objectives(&criteria, name, |v| match op {
                        StatScoreOp::Add(amount) => v.wrapping_add(amount),
                        StatScoreOp::Set(value) => value,
                    });
            }
        }
    }
    /// `ServerScoreboard`'s packets: each placed player's client is sent
    /// what changed on the board since it was last sent it (all of it on
    /// joining), lan/scoreboard.rs.
    pub fn lan_sync_scoreboard(&mut self) {
        if self
            .server
            .lan
            .as_ref()
            .is_none_or(|host| host.hub.guests.iter().all(|g| !g.placed))
        {
            return;
        }
        self.load_scoreboard();
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let board = &self.common.score.board;
        host.hub.teams.clone_from(&board.teams);
        let view = sunrise_sim::lan::scoreboard::View::of(board);
        host.hub.guests.update_all(|g| {
            if g.placed {
                g.sync_scoreboard(&view, board);
            }
        });
    }
    /// `ServerPlayer.doTick` score updates and the statistic criteria.
    pub fn tick_scoreboard(&mut self) {
        self.load_scoreboard();
        let events = std::mem::take(&mut self.common.stats.events);
        self.stat_criteria_changed(PLAYER_NAME, events);
        let mut rec = std::mem::take(&mut self.common.score.recorded);
        self.record_player_scores(&mut rec);
        self.common.score.recorded = rec;
        if self.common.ticks.is_multiple_of(20) {
            self.sync_score_registries();
        }
    }
    /// `ServerPlayer.doTick`'s `updateScoreForCriteria` calls for the acting
    /// player (the host's own, or a guest swapped in): the health, food, air,
    /// armor, experience and level objectives follow what changed since `rec`.
    pub(in crate::game::server_side) fn record_player_scores(&mut self, rec: &mut Recorded) {
        self.load_scoreboard();
        if self.common.score.board.objectives.is_empty() {
            return;
        }
        let display = self.player_display_comp();
        let name = self.acting_player_name().to_owned();
        let Some(p) = self.common.player.as_mut() else {
            return;
        };
        let health_and_absorption = p.health.health() + p.health.absorption();
        let food = p.health.food.food;
        let air = p.air;
        // getArmorValue: the armor attribute rounded down.
        let armor = p
            .attribute("minecraft:armor")
            .or_else(|_| p.attribute("armor"))
            .unwrap_or(0.0)
            .floor() as i32;
        let board = &mut self.common.score.board;
        if rec.health_and_absorption != Some(health_and_absorption) {
            rec.health_and_absorption = Some(health_and_absorption);
            let value = health_and_absorption.ceil() as i32;
            board.for_all_objectives_with(sb::CRITERIA_HEALTH, &name, Some(&display), |_| value);
        }
        let (xp, level) = (self.common.experience.total, self.common.experience.level);
        for (criteria, value) in rec.changed_scores(food, air, armor, xp, level) {
            board.for_all_objectives_with(criteria, &name, Some(&display), |_| value);
        }
    }
    /// `ServerPlayer.die`: the acting player's `deathCount` objectives go up by one.
    pub(in crate::game::server_side) fn score_player_died(&mut self) {
        let display = self.player_display_comp();
        let me = self.acting_player_name().to_owned();
        self.common.score.board.for_all_objectives_with(
            sb::CRITERIA_DEATH_COUNT,
            &me,
            Some(&display),
            |v| v.wrapping_add(1),
        );
    }
    /// `ServerPlayer.awardKillScore`: total kills, the player kills and the
    /// team kill counts, for the victim `victim`.
    pub(in crate::game) fn score_player_killed(&mut self, victim: Target) {
        let name = self.holder_name(victim);
        self.score_player_killed_named(name, matches!(victim, Target::Player | Target::Guest(_)));
    }
    /// [`score_player_killed`](Self::score_player_killed) with the victim's
    /// scoreboard name at hand.
    pub(in crate::game::server_side) fn score_player_killed_named(
        &mut self,
        victim_name: Option<String>,
        player: bool,
    ) {
        let killer = self.acting_player_name().to_owned();
        self.score_player_kill_for(&killer, victim_name.as_deref(), player);
    }

    /// `ServerPlayer.awardKillScore` uses the killer as score holder, even
    /// when the body installed for `ServerPlayer.die` belongs to the victim.
    pub(in crate::game::server_side) fn score_player_kill_for(
        &mut self,
        killer: &str,
        victim: Option<&str>,
        player: bool,
    ) {
        let display = self.player_score_display(killer);
        let board = &mut self.common.score.board;
        let inc = |v: i32| v.wrapping_add(1);
        board.for_all_objectives_with(sb::CRITERIA_TOTAL_KILLS, killer, Some(&display), inc);
        if player {
            board.for_all_objectives_with(sb::CRITERIA_PLAYER_KILLS, killer, Some(&display), inc);
        }
        let color_of =
            |board: &Scoreboard, name: &str| board.players_team(name).and_then(|t| t.color.clone());
        if let Some(victim) = victim {
            if let Some(color) = color_of(board, victim) {
                let criteria = format!("teamkill.{color}");
                board.for_all_objectives_with(&criteria, killer, Some(&display), inc);
            }
            if let Some(color) = color_of(board, killer) {
                let criteria = format!("killedByTeam.{color}");
                board.for_all_objectives(&criteria, victim, inc);
            }
        }
    }

    // ---- teams in gameplay -----------------------------------------------------

    /// `TamableAnimal.getTeam` of a mob or another player: its own team, else,
    /// when it is a tamed pet, its root owner's.
    pub(in crate::game) fn mob_team(&self, id: u32) -> Option<&sb::Team> {
        if self.common.score.board.teams.is_empty() {
            return None;
        }
        // The mob is looked up only when its own name has no team: the
        // owner's team is the fallback.
        self.team_with_owner(id, || {
            self.common
                .mobs
                .iter()
                .find(|m| m.id == id)
                .and_then(tame_owner)
        })
    }
    /// `mob_team_of` while the mob tick has lent its mobs out of `Game`.
    /// The active mob and the disjoint live loan replace `self.mobs` for
    /// resolving a tame owner's UUID; other live entities still use Game.
    pub(in crate::game::server_side) fn mob_team_scoped(
        &self,
        m: &sunrise_sim::mobs::Mob,
        actor: &sunrise_sim::mobs::Mob,
        world: &sunrise_sim::mobs::MobWorld<'_>,
    ) -> Option<&sb::Team> {
        if self.common.score.board.teams.is_empty() {
            return None;
        }
        let own_uuid = *self.common.persist.uuids.get(&m.id)?;
        let own = self.scoreboard_name(m.id, own_uuid);
        if let Some(team) = self.common.score.board.players_team(&own) {
            return Some(team);
        }
        let owner = tame_owner(m)?;
        let loaned = |uuid: [i32; 4]| {
            self.common
                .persist
                .uuids
                .iter()
                .filter(|(_, found)| **found == uuid)
                .find_map(|(&id, _)| {
                    if id == actor.id {
                        (!actor.removed).then_some(actor)
                    } else {
                        world
                            .live_mobs
                            .and_then(|mobs| mobs.find(id))
                            .filter(|mob| !mob.removed)
                    }
                })
        };
        let root = root_owner(owner, |uuid| {
            if let Some(mob) = loaned(uuid) {
                return tame_owner(mob);
            }
            let Target::Mob(id) = self.target_by_uuid(ints_uuid(uuid))? else {
                return None;
            };
            self.common
                .mobs
                .iter()
                .find(|mob| mob.id == id)
                .and_then(tame_owner)
        });
        let owner_name = if let Some(mob) = loaned(root) {
            self.scoreboard_name(mob.id, root)
        } else {
            match self.target_by_uuid(ints_uuid(root))? {
                Target::Player => self.acting_player_name().to_owned(),
                Target::Guest(conn) => self.command_guest(conn)?.name.clone(),
                Target::Mob(id) => self.scoreboard_name(id, root),
                _ => uuid_text(root),
            }
        };
        pet_team(&self.common.score.board, &own, Some(&owner_name))
    }
    pub(in crate::game) fn player_team(&self) -> Option<&sb::Team> {
        self.common
            .score
            .board
            .players_team(self.acting_player_name())
    }
    /// The teams of the player and of the mobs on one (empty without teams).
    pub(in crate::game) fn mob_teams(&self) -> sunrise_sim::mobs::Teams<'_> {
        if self.common.score.board.teams.is_empty() {
            return Default::default();
        }
        // One pass over the mobs, not one per mob.
        let tame_owners: HashMap<u32, [i32; 4]> = self
            .common
            .mobs
            .iter()
            .filter_map(|m| Some((m.id, tame_owner(m)?)))
            .collect();
        let host = self
            .player_identity()
            .and_then(|(_, id)| Some((id, self.player_team()?)));
        let guests = self.server.lan.iter().flat_map(|lan| &lan.hub.guests);
        let players = host
            .into_iter()
            .chain(guests.filter(|g| !g.is_fake()).filter_map(|g| {
                Some((
                    u32::try_from(g.entity_id).ok()?,
                    self.common.score.board.players_team(&g.name)?,
                ))
            }))
            .collect();
        sunrise_sim::mobs::Teams {
            player: self.player_team(),
            players,
            mobs: self
                .common
                .persist
                .uuids
                .keys()
                .filter_map(|id| {
                    let owner = || tame_owners.get(id).copied();
                    Some((*id, self.team_with_owner(*id, owner)?))
                })
                .collect(),
        }
    }
    /// The carts (indices into `carts`, as `near` holds them) that cart `i`
    /// pushes this tick, as `OldMinecartBehavior.pushAndPickupEntities` asks
    /// `EntitySelector.pushableBy(cart)`: a moving rideable cart keeps only
    /// what its own team's and the target's collision rules allow; a still or
    /// unrideable cart pushes every minecart in its box (no team filter).
    pub(in crate::game::server_side) fn cart_push_allowed(
        &self,
        carts: &[super::minecarts::Minecart],
        i: usize,
        near: &[u32],
    ) -> Vec<bool> {
        let v = carts[i].velocity;
        let moving =
            carts[i].kind == super::minecarts::CartKind::Rideable && v.x * v.x + v.z * v.z >= 0.01;
        if !moving {
            return vec![true; near.len()];
        }
        let own = self.team_with_owner(carts[i].id, || None);
        near.iter()
            .map(|&j| {
                let target = &carts[j as usize];
                sb::pushable_by(own, self.team_with_owner(target.id, || None), true)
            })
            .collect()
    }
    /// The mobs on the player's team (`isAlliedTo`).
    pub(in crate::game::server_side) fn player_allies(&self) -> HashSet<u32> {
        let Some(team) = self.player_team() else {
            return HashSet::new();
        };
        self.common
            .persist
            .uuids
            .iter()
            .filter(|(id, uuid)| team.players.contains(&self.scoreboard_name(**id, **uuid)))
            .map(|(id, _)| *id)
            .collect()
    }
    /// `Player.getDisplayName`: the name with the team's prefix, suffix and colour.
    pub(in crate::game::server_side) fn player_display_comp(&self) -> Comp {
        self.player_score_display(self.acting_player_name())
    }
    pub(in crate::game::server_side) fn player_score_display(&self, name: &str) -> Comp {
        let display = literal(name);
        match self.common.score.board.players_team(name) {
            Some(t) => t.formatted_name_comp(&display),
            None => display,
        }
    }
    pub(in crate::game::server_side) fn player_display_text(&self) -> Text {
        let name = Text::literal(self.acting_player_name());
        match self.player_team() {
            Some(t) => t.formatted_name(name),
            None => name,
        }
    }

    // ---- commands ------------------------------------------------------------

    /// The scoreboard names a score holder argument resolves to.
    pub(in crate::game::server_side) fn score_holder_names(
        &mut self,
        ctx: &Context,
        name: &str,
        default_wildcard: bool,
    ) -> super::commands::R<Vec<String>> {
        let mut env = GameEnv { g: self };
        Ok(
            scoreboard_cmd::holders(ctx, name, &mut env, default_wildcard)?
                .into_iter()
                .map(|h| h.name)
                .collect(),
        )
    }
    pub(in crate::game::server_side) fn cmd_scoreboard(
        &mut self,
        id: &str,
        ctx: &Context,
    ) -> super::commands::R<i32> {
        let mut env = GameEnv { g: self };
        let result = scoreboard_cmd::execute(id, ctx, &mut env)
            .unwrap_or_else(|| Err(sunrise_command::CommandError::tr("command.failed", &[])));
        self.sync_score_registries();
        result
    }
    /// ComponentUtils.updateForEntity for the pieces scoreboards store:
    /// `selector` and `score` components become their text.
    pub(in crate::game::server_side) fn resolve_component(&mut self, c: Comp) -> Comp {
        match c {
            Comp::Object(mut o) => {
                if let Some(sel) = o.get("selector").and_then(Comp::as_str).map(str::to_owned) {
                    let text = self.resolve_selector_text(&sel);
                    o.remove("selector");
                    o.insert("text".into(), Comp::String(text));
                } else if let Some(score) = o.get("score").cloned() {
                    let name = score.get("name").and_then(Comp::as_str).unwrap_or("");
                    let objective = score.get("objective").and_then(Comp::as_str).unwrap_or("");
                    let name = if name == "*" { PLAYER_NAME } else { name };
                    let text = self
                        .common
                        .score
                        .board
                        .score(name, objective)
                        .map_or_else(String::new, |s| s.value.to_string());
                    o.remove("score");
                    o.insert("text".into(), Comp::String(text));
                }
                for key in ["extra", "with"] {
                    if let Some(Comp::Array(list)) = o.remove(key) {
                        let list = list
                            .into_iter()
                            .map(|c| self.resolve_component(c))
                            .collect();
                        o.insert(key.into(), Comp::Array(list));
                    }
                }
                Comp::Object(o)
            }
            Comp::Array(list) => Comp::Array(
                list.into_iter()
                    .map(|c| self.resolve_component(c))
                    .collect(),
            ),
            other => other,
        }
    }
    pub(in crate::game::server_side) fn resolve_selector_text(&mut self, raw: &str) -> String {
        let mut reader = sunrise_command::Reader::new(raw);
        let ty = sunrise_command::args::ArgType::Entity {
            single: false,
            players: false,
        };
        let Ok(sunrise_command::Arg::Entity(sel)) =
            ty.parse(&mut reader, &self.common.ui.registries)
        else {
            return raw.to_owned();
        };
        let names: Vec<String> = self
            .select(&sel)
            .into_iter()
            .map(|t| self.target_name(t).plain())
            .collect();
        names.join(", ")
    }

    // ---- the sidebar -----------------------------------------------------------
}

/// The host and guests that have entered the world, rebuilt from live state.
#[cfg(test)]
pub(in crate::game) fn local_tab_players(me: TabPlayer, guests: &[Guest]) -> Vec<TabPlayer> {
    let mut players = vec![me];
    players.extend(guests.iter().filter(|g| g.placed).map(|g| {
        let mut player = TabPlayer::new(g.uuid, &g.name);
        player.game_mode = g.game_mode;
        player.latency = g.latency.load(std::sync::atomic::Ordering::Relaxed);
        player
    }));
    players
}

/// `ScoreHolder.getFeedbackDisplayName`: the holder's display name with its
/// hover replaced by `HoverEvent.ShowText` of its scoreboard name (a player's
/// name, an entity's UUID string); the insertion stays.
pub(in crate::game::server_side) fn feedback_name(display: Text, scoreboard_name: &str) -> Text {
    let mut component = display.component();
    if let serde_json::Value::Object(o) = &mut component {
        o.insert(
            "hover_event".into(),
            serde_json::json!({"action": "show_text", "value": scoreboard_name}),
        );
    }
    display.with_component(component)
}

/// `TriggerCommand.suggestObjectives`: the trigger objectives `holder` has an
/// unlocked score in.
pub(in crate::game::server_side) fn trigger_objectives_for(
    board: &Scoreboard,
    holder: &str,
) -> Vec<String> {
    let mut names: Vec<String> = board
        .objectives
        .iter()
        .filter(|o| o.criteria == "trigger")
        .filter(|o| board.score(holder, &o.name).is_some_and(|s| !s.locked))
        .map(|o| o.name.clone())
        .collect();
    names.sort();
    names
}

/// The commands' view of the game.
pub(in crate::game) struct GameEnv<'a, 'b> {
    pub(in crate::game) g: &'a mut crate::game::ServerGame<'b>,
}
impl Env for GameEnv<'_, '_> {
    fn sb(&self) -> &Scoreboard {
        &self.g.score.board
    }
    fn sb_mut(&mut self) -> &mut Scoreboard {
        &mut self.g.score.board
    }
    fn select(&mut self, selector: &Selector) -> Vec<Holder> {
        let found = self.g.select(selector);
        found
            .into_iter()
            .filter_map(|t| self.g.holder_for(t))
            .collect()
    }
    fn named(&mut self, name: &str) -> Holder {
        if name.eq_ignore_ascii_case(self.g.acting_player_name()) && self.g.player.is_some() {
            return self
                .g
                .holder_for(Target::Player)
                .unwrap_or_else(|| Holder::named(name));
        }
        match self.g.target_of_holder(name) {
            Some(t) => self.g.holder_for(t).unwrap_or_else(|| Holder::named(name)),
            None => Holder::named(name),
        }
    }
    fn source(&mut self) -> Option<(Holder, bool)> {
        let entity = match &self.g.server.command_source {
            Some(s) => s.entity,
            None => Some(Target::Player),
        }?;
        let name = match entity {
            Target::Player => self.g.acting_player_name().to_owned(),
            Target::Mob(id) | Target::Item(id) => uuid_text(self.g.uuid_for(id)),
            _ => return None,
        };
        let text = feedback_name(self.g.target_name(entity), &name);
        Some((
            Holder {
                display: Some(literal(&text.plain())),
                text,
                name,
            },
            entity == Target::Player,
        ))
    }
    fn resolve(&mut self, c: Comp) -> Comp {
        self.g.resolve_component(c)
    }
    fn feedback(&mut self, message: Text) {
        self.g.feedback(message);
    }
}

#[cfg(test)]
pub(in crate::game) mod tab_list_tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicI32, Ordering},
        mpsc,
    };
    use sunrise_protocol::packets::configuration::ClientInformation;
    use sunrise_sim::lan::{Joined, PlayerSender};

    fn guest(uuid: u128, name: &str) -> Guest {
        let (tx, _rx) = mpsc::channel();
        Guest::new(
            Joined {
                conn: uuid as u32,
                uuid,
                name: name.into(),
                properties: Vec::new(),
                info: ClientInformation::new(8),
                brand: "test".into(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(42)),
                address: "127.0.0.1:25565".parse().unwrap(),
                expected: false,
            },
            uuid as i32,
            [0.0, 64.0, 0.0],
            8,
        )
    }

    #[test]
    #[cfg(any())]
    fn lan_tab_list_tracks_join_and_disconnect() {
        let me = TabPlayer::new(1, PLAYER_NAME);
        let mut guests = vec![guest(2, "Alex")];
        let players = local_tab_players(me.clone(), &guests);
        assert_eq!(players.as_slice(), std::slice::from_ref(&me));
        assert!(!tab_list::visible(true, true, true, players.len(), false));

        guests[0].placed = true;
        let players = local_tab_players(me.clone(), &guests);
        assert_eq!(players.len(), 2);
        assert_eq!(players[0], me);
        assert_eq!(players[1].uuid, 2);
        assert_eq!(players[1].name, "Alex");
        assert!(tab_list::visible(true, true, true, players.len(), false));
        assert!(!tab_list::visible(true, false, true, players.len(), false));

        guests.clear();
        let players = local_tab_players(me, &guests);
        assert_eq!(players.len(), 1);
        assert!(!tab_list::visible(true, true, true, players.len(), false));
        assert!(tab_list::visible(true, true, true, players.len(), true));
        // F1 (`Hud.isHidden`) takes the list away: `extractRenderState` draws
        // it inside `if (!isHidden)`, the key and the objective aside.
        assert!(!tab_list::visible(false, true, true, players.len(), true));
        assert!(!tab_list::visible(false, true, false, 2, false));
    }

    #[test]
    fn lan_tab_list_reads_current_guest_state() {
        let me = TabPlayer::new(1, PLAYER_NAME);
        let mut guests = vec![guest(2, "Alex"), guest(3, "Grumm")];
        for guest in &mut guests {
            guest.placed = true;
        }
        guests[0].game_mode = sunrise_level::profile_lookup::SPECTATOR;
        let players = local_tab_players(me.clone(), &guests);
        assert_eq!(players.len(), 3);
        assert_eq!(
            players[1].game_mode,
            sunrise_level::profile_lookup::SPECTATOR
        );
        assert_eq!(players[1].latency, 42);
        assert_eq!(
            players[1].skin,
            sunrise_level::profile_lookup::default_skin(2)
        );
        assert_eq!(players[2].name, "Grumm");

        guests[0].game_mode = 1;
        guests[0].latency.store(123, Ordering::Relaxed);
        guests.remove(1);
        let players = local_tab_players(me, &guests);
        assert_eq!(players.len(), 2);
        assert_eq!(players[1].game_mode, 1);
        assert_eq!(players[1].latency, 123);
    }
}

#[cfg(test)]
pub(in crate::game) mod team_tests {
    use super::*;
    /// #14279: `ScoreHolder.getFeedbackDisplayName` keeps the display name and
    /// its insertion, and shows the scoreboard name (an entity's UUID) on hover.
    #[test]
    fn a_feedback_name_shows_the_scoreboard_name_on_hover() {
        let uuid = "00000001-0000-0002-0000-000300000004";
        let display = Text::translate("entity.minecraft.pig", &[]).with_component_fields(vec![
            (
                "hover_event".into(),
                serde_json::json!({"action": "show_entity", "id": "minecraft:pig", "uuid": uuid}),
            ),
            ("insertion".into(), uuid.into()),
        ]);
        let c = feedback_name(display, uuid).component();
        assert_eq!(c["translate"], "entity.minecraft.pig");
        assert_eq!(c["insertion"], uuid);
        assert_eq!(
            c["hover_event"],
            serde_json::json!({"action": "show_text", "value": uuid})
        );
    }

    const WOLF: [i32; 4] = [1, 2, 3, 4];
    const CAT: [i32; 4] = [5, 6, 7, 8];
    const STEVE: [i32; 4] = [9, 10, 11, 12];

    /// The player in place is on `red`, the cat on `blue`; nobody else is on a
    /// team (`Game::mob_team` names the player by profile, a mob by UUID).
    fn board() -> Scoreboard {
        let mut board = Scoreboard::default();
        board.add_team("red");
        board.add_to_team("Steve", "red");
        board.add_team("blue");
        board.add_to_team(&uuid_text(CAT), "blue");
        board
    }

    fn team_name(team: Option<&sb::Team>) -> Option<&str> {
        team.map(|t| t.name.as_str())
    }

    #[test]
    fn a_tamed_pet_with_no_team_answers_its_owners() {
        let board = board();
        let wolf = uuid_text(WOLF);
        assert_eq!(
            team_name(pet_team(&board, &wolf, Some("Steve"))),
            Some("red")
        );
    }

    #[test]
    fn a_pet_with_its_own_team_answers_that_one() {
        let board = board();
        let cat = uuid_text(CAT);
        assert_eq!(
            team_name(pet_team(&board, &cat, Some("Steve"))),
            Some("blue")
        );
    }

    #[test]
    fn an_untamed_pet_answers_no_team() {
        let board = board();
        let wolf = uuid_text(WOLF);
        // `mob_team` names no owner for a pet that is not tamed.
        assert_eq!(pet_team(&board, &wolf, None), None);
    }

    #[test]
    fn a_pet_owned_by_a_pet_reaches_the_root_owner() {
        let board = board();
        // The wolf's owner is the cat, whose own owner is the player: the
        // wolf answers the player's team, not the cat's.
        let pet_owner = |pet: [i32; 4]| (pet == CAT).then_some(STEVE);
        let root = root_owner(CAT, pet_owner);
        assert_eq!(root, STEVE);
        let root_name = if root == STEVE { "Steve" } else { "nobody" };
        assert_eq!(
            team_name(pet_team(&board, &uuid_text(WOLF), Some(root_name))),
            Some("red")
        );
        // An owner loop (only `/data` can make one) stops instead of hanging.
        let looped = |uuid: [i32; 4]| Some(if uuid == WOLF { CAT } else { WOLF });
        let root = root_owner(WOLF, looped);
        assert!(root == WOLF || root == CAT);
    }

    #[test]
    fn air_score_follows_the_air_supply_and_updates_only_on_change() {
        let mut rec = Recorded::default();
        let first = rec.changed_scores(20, 120, 0, 0, 0);
        assert!(first.contains(&(sb::CRITERIA_AIR, 120)));
        assert!(rec.changed_scores(20, 120, 0, 0, 0).is_empty());
        assert_eq!(
            rec.changed_scores(20, 119, 0, 0, 0),
            vec![(sb::CRITERIA_AIR, 119)]
        );
        assert_eq!(
            rec.changed_scores(20, -5, 0, 0, 0),
            vec![(sb::CRITERIA_AIR, -5)]
        );
    }
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::scoreboard_common::CommonGameExt1 as _;
