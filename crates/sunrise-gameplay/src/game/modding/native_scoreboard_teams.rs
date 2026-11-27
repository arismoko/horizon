//! Scoreboard team handles for native server mods.
//!
//! As the objectives of `native_scoreboard.rs`: every write is made on the same `Scoreboard`
//! `/team` writes, with its methods, so the sync, the save and the selectors need to know
//! nothing of a mod; a handle holds the world's epoch and the identity the board gave its team.
use super::native_scoreboard::api_text;
use core::ops::ControlFlow;
use mod_api::__private::game::*;
use mod_api::{
    CollisionRule, ScoreHolder, ServerReadError, TeamColor, TeamError, TeamMembersReadError,
    TeamRef, TeamVisibility, TextFormError,
};
use sunrise_game_data::{
    scoreboard::{self as sb, Comp, comp_text},
    text::Text as EngineText,
};

pub(super) fn api_visibility(visibility: sb::Visibility) -> TeamVisibility {
    match visibility {
        sb::Visibility::Always => TeamVisibility::Always,
        sb::Visibility::Never => TeamVisibility::Never,
        sb::Visibility::HideForOtherTeams => TeamVisibility::HideForOtherTeams,
        sb::Visibility::HideForOwnTeam => TeamVisibility::HideForOwnTeam,
    }
}

fn engine_visibility(visibility: TeamVisibility) -> sb::Visibility {
    match visibility {
        TeamVisibility::Never => sb::Visibility::Never,
        TeamVisibility::HideForOtherTeams => sb::Visibility::HideForOtherTeams,
        TeamVisibility::HideForOwnTeam => sb::Visibility::HideForOwnTeam,
        _ => sb::Visibility::Always,
    }
}

pub(super) fn api_collision_rule(rule: sb::CollisionRule) -> CollisionRule {
    match rule {
        sb::CollisionRule::Always => CollisionRule::Always,
        sb::CollisionRule::Never => CollisionRule::Never,
        sb::CollisionRule::PushOtherTeams => CollisionRule::PushOtherTeams,
        sb::CollisionRule::PushOwnTeam => CollisionRule::PushOwnTeam,
    }
}

fn engine_collision_rule(rule: CollisionRule) -> sb::CollisionRule {
    match rule {
        CollisionRule::Never => sb::CollisionRule::Never,
        CollisionRule::PushOtherTeams => sb::CollisionRule::PushOtherTeams,
        CollisionRule::PushOwnTeam => sb::CollisionRule::PushOwnTeam,
        _ => sb::CollisionRule::Always,
    }
}

/// The API's color of a team, which the board keeps as the color's name.
pub(super) fn api_color(team: &sb::Team) -> Option<TeamColor> {
    team.color.as_deref().and_then(TeamColor::from_name)
}

#[allow(dead_code)]
pub(super) fn api_comp(comp: &Comp) -> Result<mod_api::Text, TextFormError> {
    api_text(&comp_text(comp))
}

impl crate::game::ServerGame<'_> {
    /// The team a handle names: None for a stale handle, one of an earlier open, or on a client.
    fn team_state(&self, team: TeamRef) -> Option<&sb::Team> {
        if !self.common.is_logical_server()
            || self.common.native_lifecycle.epoch() != Some(team.fields().epoch)
        {
            return None;
        }
        self.common.score.board.team_by_identity(team.fields().id)
    }

    fn team_handle(&self, team: &sb::Team) -> Option<TeamRef> {
        Some(team_ref(
            self.common.native_lifecycle.epoch()?,
            team.identity.0,
        ))
    }

    /// The name of the team `team` names, for a write: the board is loaded, and the savepoint of
    /// an open transaction is kept before anything changes.
    fn team_for_write(&mut self, team: TeamRef) -> Result<String, TeamError> {
        self.load_scoreboard();
        let name = self.team_state(team).ok_or(TeamError::Stale)?.name.clone();
        self.journal_scoreboard();
        Ok(name)
    }

    /// Edits the team `team` names, for a write that cannot fail past the handle.
    fn team_edit(&mut self, team: TeamRef, edit: impl FnOnce(&mut sb::Team)) -> bool {
        let Ok(name) = self.team_for_write(team) else {
            return false;
        };
        if let Some(team) = self.common.score.board.team_mut(&name) {
            edit(team);
        }
        true
    }

    /// `Scoreboard.getPlayerTeam`.
    pub(in crate::game) fn team(&self, name: &str) -> Option<TeamRef> {
        if !self.common.is_logical_server() {
            return None;
        }
        self.team_handle(self.common.score.board.team(name)?)
    }

    /// `Scoreboard.getPlayerTeams`.
    pub(in crate::game) fn visit_teams(
        &self,
        visit: &mut dyn FnMut(TeamRef) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, ServerReadError> {
        let epoch = self.scoreboard_visit_epoch()?;
        Ok(self
            .common
            .score
            .board
            .teams
            .iter()
            .map(|t| team_ref(epoch, t.identity.0))
            .try_for_each(visit))
    }

    /// `/team add`.
    pub(in crate::game) fn add_team(&mut self, name: &str) -> Result<TeamRef, TeamError> {
        if !self.common.is_logical_server() || self.common.native_lifecycle.epoch().is_none() {
            return Err(TeamError::Stale);
        }
        self.load_scoreboard();
        self.journal_scoreboard();
        if self.common.score.board.team(name).is_some() {
            return Err(TeamError::NameTaken);
        }
        self.common.score.board.add_team(name);
        self.sync_score_registries();
        self.common
            .score
            .board
            .team(name)
            .and_then(|t| self.team_handle(t))
            .ok_or(TeamError::Stale)
    }

    /// `Scoreboard.getPlayersTeam`.
    pub(in crate::game) fn holder_team(&self, holder: ScoreHolder) -> Option<TeamRef> {
        let key = self.score_holder_key(&holder)?;
        self.team_handle(self.common.score.board.players_team(&key)?)
    }

    /// `PlayerTeam.getName`.
    pub(in crate::game) fn team_name(&self, team: TeamRef) -> Option<&str> {
        self.team_state(team).map(|t| t.name.as_str())
    }

    /// `PlayerTeam.getDisplayName`.
    pub(in crate::game) fn team_display_name(
        &self,
        team: TeamRef,
    ) -> Result<EngineText, TeamError> {
        let team = self.team_state(team).ok_or(TeamError::Stale)?;
        Ok(comp_text(&team.display_name))
    }

    /// `PlayerTeam.getColor`.
    pub(in crate::game) fn team_color(
        &self,
        team: TeamRef,
    ) -> Result<Option<TeamColor>, TeamError> {
        Ok(api_color(self.team_state(team).ok_or(TeamError::Stale)?))
    }

    /// `PlayerTeam.getPlayerPrefix`.
    pub(in crate::game) fn team_prefix(&self, team: TeamRef) -> Result<EngineText, TeamError> {
        let team = self.team_state(team).ok_or(TeamError::Stale)?;
        Ok(comp_text(&team.prefix))
    }

    /// `PlayerTeam.getPlayerSuffix`.
    pub(in crate::game) fn team_suffix(&self, team: TeamRef) -> Result<EngineText, TeamError> {
        let team = self.team_state(team).ok_or(TeamError::Stale)?;
        Ok(comp_text(&team.suffix))
    }

    /// `PlayerTeam.isAllowFriendlyFire`.
    pub(in crate::game) fn team_friendly_fire(&self, team: TeamRef) -> Option<bool> {
        self.team_state(team).map(|t| t.allow_friendly_fire)
    }

    /// `PlayerTeam.canSeeFriendlyInvisibles`.
    pub(in crate::game) fn team_see_friendly_invisibles(&self, team: TeamRef) -> Option<bool> {
        self.team_state(team).map(|t| t.see_friendly_invisibles)
    }

    /// `PlayerTeam.getNameTagVisibility`.
    pub(in crate::game) fn team_name_tag_visibility(
        &self,
        team: TeamRef,
    ) -> Option<TeamVisibility> {
        self.team_state(team)
            .map(|t| api_visibility(t.nametag_visibility))
    }

    /// `PlayerTeam.getDeathMessageVisibility`.
    pub(in crate::game) fn team_death_message_visibility(
        &self,
        team: TeamRef,
    ) -> Option<TeamVisibility> {
        self.team_state(team)
            .map(|t| api_visibility(t.death_message_visibility))
    }

    /// `PlayerTeam.getCollisionRule`.
    pub(in crate::game) fn team_collision_rule(&self, team: TeamRef) -> Option<CollisionRule> {
        self.team_state(team)
            .map(|t| api_collision_rule(t.collision_rule))
    }

    /// `PlayerTeam.getPlayers`.
    pub(in crate::game) fn team_visit_members(
        &self,
        team: TeamRef,
        visit: &mut dyn FnMut(&str) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, TeamMembersReadError> {
        if !self.common.is_logical_server() {
            return Err(TeamMembersReadError::Unavailable);
        }
        let team = self.team_state(team).ok_or(TeamMembersReadError::Stale)?;
        Ok(team.players.iter().map(String::as_str).try_for_each(visit))
    }

    /// `PlayerTeam.setDisplayName`.
    pub(in crate::game) fn team_set_display_name(
        &mut self,
        team: TeamRef,
        name: EngineText,
    ) -> Result<(), TeamError> {
        let team = self.team_for_write(team)?;
        if let Some(t) = self.common.score.board.team_mut(&team) {
            t.display_name = name.component();
        }
        Ok(())
    }

    /// `PlayerTeam.setColor`.
    pub(in crate::game) fn team_set_color(
        &mut self,
        team: TeamRef,
        color: Option<TeamColor>,
    ) -> bool {
        self.team_edit(team, |t| t.color = color.map(|c| c.name().to_owned()))
    }

    /// `PlayerTeam.setPlayerPrefix`.
    pub(in crate::game) fn team_set_prefix(
        &mut self,
        team: TeamRef,
        prefix: EngineText,
    ) -> Result<(), TeamError> {
        let team = self.team_for_write(team)?;
        if let Some(t) = self.common.score.board.team_mut(&team) {
            t.prefix = prefix.component();
        }
        Ok(())
    }

    /// `PlayerTeam.setPlayerSuffix`.
    pub(in crate::game) fn team_set_suffix(
        &mut self,
        team: TeamRef,
        suffix: EngineText,
    ) -> Result<(), TeamError> {
        let team = self.team_for_write(team)?;
        if let Some(t) = self.common.score.board.team_mut(&team) {
            t.suffix = suffix.component();
        }
        Ok(())
    }

    /// `PlayerTeam.setAllowFriendlyFire`.
    pub(in crate::game) fn team_set_friendly_fire(&mut self, team: TeamRef, allowed: bool) -> bool {
        self.team_edit(team, |t| t.allow_friendly_fire = allowed)
    }

    /// `PlayerTeam.setSeeFriendlyInvisibles`.
    pub(in crate::game) fn team_set_see_friendly_invisibles(
        &mut self,
        team: TeamRef,
        allowed: bool,
    ) -> bool {
        self.team_edit(team, |t| t.see_friendly_invisibles = allowed)
    }

    /// `PlayerTeam.setNameTagVisibility`.
    pub(in crate::game) fn team_set_name_tag_visibility(
        &mut self,
        team: TeamRef,
        visibility: TeamVisibility,
    ) -> bool {
        self.team_edit(team, |t| {
            t.nametag_visibility = engine_visibility(visibility)
        })
    }

    /// `PlayerTeam.setDeathMessageVisibility`.
    pub(in crate::game) fn team_set_death_message_visibility(
        &mut self,
        team: TeamRef,
        visibility: TeamVisibility,
    ) -> bool {
        self.team_edit(team, |t| {
            t.death_message_visibility = engine_visibility(visibility)
        })
    }

    /// `PlayerTeam.setCollisionRule`.
    pub(in crate::game) fn team_set_collision_rule(
        &mut self,
        team: TeamRef,
        rule: CollisionRule,
    ) -> bool {
        self.team_edit(team, |t| t.collision_rule = engine_collision_rule(rule))
    }

    /// `Scoreboard.addPlayerToTeam`.
    pub(in crate::game) fn team_join(
        &mut self,
        team: TeamRef,
        holder: ScoreHolder,
    ) -> Result<bool, TeamError> {
        let name = self.team_for_write(team)?;
        let (key, _) = self
            .score_holder_for_write(&holder)
            .ok_or(TeamError::UnknownHolder)?;
        let changed = self.common.score.board.add_to_team(&key, &name);
        self.sync_score_registries();
        Ok(changed)
    }

    /// `Scoreboard.removePlayerFromTeam(holder, team)`.
    pub(in crate::game) fn team_leave(&mut self, team: TeamRef, holder: ScoreHolder) -> bool {
        let Ok(name) = self.team_for_write(team) else {
            return false;
        };
        let Some(key): Option<String> = self.score_holder_key(&holder) else {
            return false;
        };
        if self.common.score.board.players_team(&key).map(|t| &t.name) != Some(&name) {
            return false;
        }
        self.common.score.board.remove_from_team(&key);
        self.sync_score_registries();
        true
    }

    /// `Scoreboard.removePlayerTeam`.
    pub(in crate::game) fn team_remove(&mut self, team: TeamRef) -> bool {
        let Ok(name) = self.team_for_write(team) else {
            return false;
        };
        self.common.score.board.remove_team(&name);
        self.sync_score_registries();
        true
    }
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

#[cfg(test)]
mod tests {
    use super::*;

    /// The API's visibility and rule names are the board's.
    #[test]
    fn option_names_are_the_boards() {
        let visibilities: Vec<_> = TeamVisibility::ALL.iter().map(|v| v.name()).collect();
        assert_eq!(
            visibilities,
            sb::Visibility::ALL
                .iter()
                .map(|v| v.name())
                .collect::<Vec<_>>()
        );
        let rules: Vec<_> = CollisionRule::ALL.iter().map(|r| r.name()).collect();
        assert_eq!(
            rules,
            sb::CollisionRule::ALL
                .iter()
                .map(|r| r.name())
                .collect::<Vec<_>>()
        );
        for v in TeamVisibility::ALL {
            assert_eq!(api_visibility(engine_visibility(v)), v);
        }
        for r in CollisionRule::ALL {
            assert_eq!(api_collision_rule(engine_collision_rule(r)), r);
        }
    }

    #[test]
    #[ignore = "daily"]
    #[cfg(any())]
    fn teams_are_written_joined_synced_saved_and_stale_once_removed() {
        use super::super::native_entity_refs::tests::with_game;
        use crate::game::server_side::native_scoreboard::tests::open_lan;
        use mod_api::prelude::ScoreboardServerHost;
        use sunrise_gameplay_common::game::server_side::native_scoreboard::tests::guest;

        with_game(|game| {
            game.server.world_root = game.server.save.0.root.clone();
            let saved_board = open_lan(game);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .push(guest(123, 1));
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .push(guest(456, 2));
            let (a, b) = (game.player(123).unwrap(), game.player(456).unwrap());
            let text = mod_api::Text::literal;
            assert_eq!(
                ScoreboardServerHost::visit_teams(game, &mut |_| panic!("empty board")),
                Ok(ControlFlow::Continue(()))
            );

            // A name is any string; only a taken one is refused.
            let red = ScoreboardServerHost::add_team(game, "red").unwrap();
            assert_eq!(
                ScoreboardServerHost::add_team(game, "red"),
                Err(TeamError::NameTaken)
            );
            for odd in ["", "*", &"x".repeat(100)] {
                assert!(ScoreboardServerHost::add_team(game, odd).is_ok());
            }
            assert_eq!(game.team("red"), Some(red));
            let mut first_team = Vec::new();
            assert_eq!(
                ScoreboardServerHost::visit_teams(game, &mut |team| {
                    first_team.push(team);
                    ControlFlow::Break(())
                }),
                Ok(ControlFlow::Break(()))
            );
            assert_eq!(first_team, [red]);
            assert_eq!(red.name(game), Some("red"));
            let mut unexpected_visits = 0;
            let mut count = |_: &str| {
                unexpected_visits += 1;
                ControlFlow::Continue(())
            };
            assert_eq!(
                red.visit_members(game, &mut count),
                Ok(ControlFlow::Continue(()))
            );

            // Vanilla's defaults, then every option, through to the board.
            assert_eq!(red.display_name(game), Ok(text("red")));
            assert_eq!(red.color(game), Ok(None));
            assert_eq!(red.friendly_fire(game), Some(true));
            assert_eq!(red.see_friendly_invisibles(game), Some(true));
            assert_eq!(red.name_tag_visibility(game), Some(TeamVisibility::Always));
            assert_eq!(red.collision_rule(game), Some(CollisionRule::Always));
            assert!(red.set_color(game, Some(TeamColor::Red)));
            red.set_prefix(game, text("[R] ")).unwrap();
            red.set_suffix(game, text(" *")).unwrap();
            red.set_display_name(game, text("Reds")).unwrap();
            assert!(red.set_friendly_fire(game, false));
            assert!(red.set_see_friendly_invisibles(game, false));
            assert!(red.set_name_tag_visibility(game, TeamVisibility::HideForOtherTeams));
            assert!(red.set_death_message_visibility(game, TeamVisibility::Never));
            assert!(red.set_collision_rule(game, CollisionRule::PushOwnTeam));
            assert_eq!(red.color(game), Ok(Some(TeamColor::Red)));
            assert_eq!(red.prefix(game), Ok(text("[R] ")));
            assert_eq!(red.suffix(game), Ok(text(" *")));
            assert_eq!(red.display_name(game), Ok(text("Reds")));
            assert_eq!(red.friendly_fire(game), Some(false));
            assert_eq!(red.see_friendly_invisibles(game), Some(false));
            assert_eq!(
                red.name_tag_visibility(game),
                Some(TeamVisibility::HideForOtherTeams)
            );
            assert_eq!(
                red.death_message_visibility(game),
                Some(TeamVisibility::Never)
            );
            assert_eq!(red.collision_rule(game), Some(CollisionRule::PushOwnTeam));
            let on_board = game.score.board.team("red").unwrap();
            assert_eq!(on_board.color.as_deref(), Some("red"));
            assert!(!on_board.allow_friendly_fire);
            assert_eq!(on_board.collision_rule, sb::CollisionRule::PushOwnTeam);

            // A player and that player's entity are one holder, keyed by the name.
            let as_player = ScoreHolder::Player(a);
            let as_entity = ScoreHolder::Entity(a.entity());
            assert_eq!(red.join(game, as_player.clone()), Ok(true));
            assert_eq!(red.join(game, as_entity.clone()), Ok(true));
            let members = |game: &crate::game::ServerGame<'_>| {
                let mut names = Vec::new();
                assert_eq!(
                    red.visit_members(game, &mut |name| {
                        names.push(name.to_owned());
                        ControlFlow::Continue(())
                    }),
                    Ok(ControlFlow::Continue(()))
                );
                names
            };
            assert_eq!(members(game), ["guest1"]);
            for who in [
                as_player.clone(),
                as_entity.clone(),
                ScoreHolder::Name("guest1".into()),
            ] {
                assert_eq!(game.holder_team(who), Some(red));
            }
            assert!(game.score.board.players_team("guest1").is_some());
            // An offline name joins like any string; an entity that is not here does not resolve.
            assert_eq!(
                red.join(game, ScoreHolder::Name("offline".into())),
                Ok(true)
            );
            // The Java hash set walks bucket 1 (offline) before bucket 10 (guest1).
            assert_eq!(members(game), ["offline", "guest1"]);
            let mut first = Vec::new();
            assert_eq!(
                red.visit_members(game, &mut |name| {
                    first.push(name.to_owned());
                    ControlFlow::Break(())
                }),
                Ok(ControlFlow::Break(()))
            );
            assert_eq!(first, ["offline"]);
            let nobody = ScoreHolder::Entity(mod_api::EntityRef::new(
                game.native_lifecycle.epoch().unwrap(),
                9_999_999,
            ));
            assert_eq!(
                red.join(game, nobody.clone()),
                Err(TeamError::UnknownHolder)
            );
            assert_eq!(game.holder_team(nobody), None);

            // Joining another team moves the holder off the old one.
            let blue = ScoreboardServerHost::add_team(game, "blue").unwrap();
            assert_eq!(blue.join(game, as_player.clone()), Ok(true));
            assert_eq!(game.holder_team(as_player.clone()), Some(blue));
            assert_eq!(members(game), ["offline"]);
            // Leaving a team the holder is not on changes nothing.
            assert!(!red.leave(game, as_player.clone()));
            assert!(blue.leave(game, as_player.clone()));
            assert_eq!(game.holder_team(as_player.clone()), None);
            assert!(!blue.leave(game, as_player.clone()));
            assert_eq!(blue.join(game, ScoreHolder::Player(b)), Ok(true));

            // The command sees the mod's team and the mod sees the command's changes.
            game.ui.registries.permission = 4;
            assert!(game.run_command("team modify red color blue"));
            assert_eq!(red.color(game), Ok(Some(TeamColor::Blue)));
            assert!(game.run_command("team join red guest1"));
            assert_eq!(game.holder_team(as_player.clone()), Some(red));
            assert!(game.run_command("team leave guest1"));

            // A guest's tracked board follows the board, teams included.
            game.lan_sync_scoreboard();
            let view = sunrise_sim::lan::scoreboard::View::of(&game.score.board);
            assert_eq!(
                game.server.lan.as_ref().unwrap().hub.guests[0].scoreboard,
                view
            );

            // Saved, the file holds the team and its members.
            game.save_scoreboard();
            let saved = sunrise_sim::save::read_dat(&saved_board).unwrap();
            let loaded = sb::Scoreboard::unpack(saved.get("data").unwrap());
            assert_eq!(loaded.team("red").unwrap().color.as_deref(), Some("blue"));
            assert!(loaded.players_team("offline").is_some());

            // Reopened, the saved team is there before any write or tick, and a handle from
            // before the open is stale.
            game.native_lifecycle_open_started();
            game.score = Default::default();
            game.native_lifecycle_world_ready();
            let red_again = game.team("red").unwrap();
            assert_ne!(red_again, red);
            assert_eq!(red.name(game), None);
            assert_eq!(red_again.color(game), Ok(Some(TeamColor::Blue)));

            // Removed, and added again under its name: the old handle stays stale.
            assert!(red_again.remove(game));
            assert_eq!(game.holder_team(ScoreHolder::Name("offline".into())), None);
            let renamed = ScoreboardServerHost::add_team(game, "red").unwrap();
            assert_ne!(renamed, red_again);
            assert_eq!(red_again.name(game), None);
            assert_eq!(red_again.display_name(game), Err(TeamError::Stale));
            assert_eq!(
                red_again.visit_members(game, &mut count),
                Err(TeamMembersReadError::Stale)
            );
            assert_eq!(unexpected_visits, 0);
            assert_eq!(
                red_again.join(game, ScoreHolder::Name("x".into())),
                Err(TeamError::Stale)
            );
            assert!(!red_again.remove(game));
            assert!(!red_again.set_color(game, None));
            // The command's removal makes a handle stale the same way.
            assert!(game.run_command("team remove red"));
            assert_eq!(renamed.name(game), None);

            // An aborted transaction puts the teams back, as it does the objectives.
            let before = game.score.board.clone();
            let aborted: Result<(), ()> = game.storage_transaction(|host, _| {
                let temp = ScoreboardServerHost::add_team(host, "temp").map_err(|_| ())?;
                temp.join(host, ScoreHolder::Name("p".into()))
                    .map_err(|_| ())?;
                assert!(temp.set_color(host, Some(TeamColor::Gold)));
                Err(())
            });
            assert!(aborted.is_err());
            assert_eq!(game.score.board, before);
            assert!(game.team("temp").is_none());
            let _ = std::fs::remove_file(&saved_board);
        });
    }
}
