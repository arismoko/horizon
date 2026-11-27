//! The two names a command reads from an entity: `Entity.getDisplayName` for
//! command replies (the team-formatted name with `show_entity` hover and UUID
//! insertion, with click events removed from the name and its siblings) and
//! `Nameable.getPlainTextName` for the selector's `name=`.
use super::Target;
use crate::game::ServerGame;
use sunrise_game_data::text::Text;

impl ServerGame<'_> {
    /// `Nameable.getPlainTextName`: `getName().getString()`, the name without
    /// the team's prefix and suffix that `getDisplayName` adds.
    pub(in crate::game) fn target_plain_name(&self, t: Target) -> String {
        match t {
            Target::Player => self.acting_player_name().to_owned(),
            Target::Guest(conn) => self
                .command_guest(conn)
                .map_or_else(String::new, |g| g.name.clone()),
            Target::Mob(id) => self
                .common
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map_or_else(String::new, |m| m.command_name().plain()),
            other => self.target_base_name(other).plain(),
        }
    }
    /// Capture enough of `t` before `/kill` removes its storage to build
    /// `Entity.getDisplayName` afterwards (`KillCommand` calls `getDisplayName`
    /// after `entity.kill`, when `Scoreboard.entityRemoved` has already left
    /// any team for removed non-player entities).
    pub(in crate::game) fn capture_kill_display(&self, t: Target) -> KillDisplay {
        match t {
            Target::Player | Target::Guest(_) | Target::Mob(_) => {
                KillDisplay::Ready(self.target_name(t))
            }
            other => KillDisplay::Entity {
                kind: self.target_type(other),
                uuid: sunrise_gameplay_common::game::commands_common::target_id(other)
                    .and_then(|id| self.common.persist.uuid_of(id)),
                name: self.target_base_name(other),
            },
        }
    }
    /// Finish `Entity.getDisplayName` for a killed target against the post-kill
    /// scoreboard team state.
    pub(in crate::game) fn finish_kill_display(&self, captured: KillDisplay) -> Text {
        match captured {
            KillDisplay::Ready(text) => text,
            KillDisplay::Entity { kind, uuid, name } => {
                let holder = uuid.map(sunrise_sim::save::uuid_text);
                let team = holder
                    .as_deref()
                    .and_then(|holder| self.common.score.board.players_team(holder));
                sunrise_gameplay_common::game::CommonGame::decorate_entity_display_name_with_uuid(
                    &kind, uuid, team, name,
                )
            }
        }
    }
}

/// Pre-kill identity snapshot so `KillCommand` can format `Entity.getDisplayName`
/// after `entity.kill` removes the entity from its storage and team.
pub(in crate::game) enum KillDisplay {
    Ready(Text),
    Entity {
        kind: String,
        uuid: Option<[i32; 4]>,
        name: Text,
    },
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;
