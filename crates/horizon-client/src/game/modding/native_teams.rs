//! Scoreboard teams for server mods (`/team add|join|leave|remove|modify color`).
use super::Game;
use horizon_client::text::COLORS;

impl Game<'_> {
    /// `Scoreboard.addPlayerTeam` and `PlayerTeam.setColor`: `name` made when missing, its
    /// colour one of the sixteen chat colour names or none. False on a client, for an empty
    /// name or one with a space, and for an unknown colour.
    pub(super) fn team_set(&mut self, name: &str, color: Option<&str>) -> bool {
        if self.remote.is_some() || name.is_empty() || name.contains(char::is_whitespace) {
            return false;
        }
        if color.is_some_and(|color| !COLORS.iter().any(|(_, known, _)| *known == color)) {
            return false;
        }
        self.load_scoreboard();
        self.score.board.add_team(name).color = color.map(str::to_owned);
        true
    }
    /// `Scoreboard.addPlayerToTeam`: `holder` leaves its team and joins `team`. False on a
    /// client, for an empty holder and an unknown team.
    pub(super) fn team_join(&mut self, team: &str, holder: &str) -> bool {
        if self.remote.is_some() || holder.is_empty() {
            return false;
        }
        self.load_scoreboard();
        if self.score.board.team(team).is_none() {
            return false;
        }
        self.score.board.add_to_team(holder, team);
        true
    }
    /// `Scoreboard.removePlayerFromTeam`: true when `holder` was on a team.
    pub(super) fn team_leave(&mut self, holder: &str) -> bool {
        if self.remote.is_some() {
            return false;
        }
        self.load_scoreboard();
        self.score.board.remove_from_team(holder)
    }
    /// `Scoreboard.removePlayerTeam`: true when the team existed.
    pub(super) fn team_remove(&mut self, name: &str) -> bool {
        if self.remote.is_some() {
            return false;
        }
        self.load_scoreboard();
        if self.score.board.team(name).is_none() {
            return false;
        }
        self.score.board.remove_team(name);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_guest_turn_tests::with_guests;

    /// Teams are made, coloured, joined, left and removed in the world's scoreboard, which
    /// the next tick's sync sends to the players; refused input changes nothing.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn teams_are_kept_in_the_worlds_scoreboard() {
        with_guests(|game| {
            assert!(game.team_set("red", Some("red")));
            assert_eq!(
                game.score.board.team("red").unwrap().color.as_deref(),
                Some("red")
            );
            assert!(!game.team_set("blue", Some("not_a_colour")));
            assert!(game.score.board.team("blue").is_none());
            assert!(!game.team_set("two words", None));
            assert!(
                game.team_set("red", None),
                "an existing team takes the change"
            );
            assert_eq!(game.score.board.team("red").unwrap().color, None);
            assert!(!game.team_join("missing", "guest12"));
            assert!(game.team_join("red", "guest12"));
            assert!(game.score.board.allied("guest12", "guest12"));
            assert_eq!(
                game.score.board.players_team("guest12").unwrap().name,
                "red"
            );
            assert!(game.team_leave("guest12"));
            assert!(!game.team_leave("guest12"));
            assert!(game.team_join("red", "guest13"));
            assert!(game.team_remove("red"));
            assert!(game.score.board.players_team("guest13").is_none());
            assert!(!game.team_remove("red"));
        });
    }
}
