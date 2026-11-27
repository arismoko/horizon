use super::commands::Target;
use sunrise_gameplay_common::game::api_values::TryIntoEngine;

impl crate::game::ServerGame<'_> {
    pub(in crate::game::server_side) fn post_contextual_game_event(
        &mut self,
        event: &'static str,
        at: [f64; 3],
        context: mod_api::GameEventContext,
    ) -> bool {
        let state = match context
            .affected_state
            .map(|state| state.try_into_engine(&()))
            .transpose()
        {
            Ok(state) => state,
            Err(_) => return false,
        };
        let Some(entity) = context.source_entity else {
            let context = self.who_context(None, state);
            self.post_game_event(event, at, context);
            return true;
        };
        let Some(target) = self.native_entity_target(entity) else {
            return false;
        };
        if !self.native_entity_here(entity) {
            return false;
        }
        if let Target::Guest(conn) = target {
            let Some(uuid) = self.command_guest(conn).map(|guest| guest.uuid) else {
                return false;
            };
            // Capture and post inside the same guest scope: Who::Player and the
            // cause id must still identify the acting guest when listeners run.
            return self.native_as_player(uuid, |game| {
                let context = game.player_context(state);
                game.post_game_event(event, at, context);
            });
        }
        let context = match target {
            Target::Player => self.player_context(state),
            Target::Mob(id) => self.mob_context(id, state),
            _ => self.entity_event_context(entity.id(), state),
        };
        self.post_game_event(event, at, context);
        true
    }
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;
