//! Server-side entity-bound player sound broadcast and entity sound state lookup.
use crate::game::ServerGame;
use sunrise_game_data::sound_source::SoundEntityState;
use sunrise_gameplay_common::game::entity_sounds_common::saved_silent;

impl ServerGame<'_> {
    pub(in crate::game) fn server_play_player_bound_sound(
        &mut self,
        event: &str,
        source: usize,
        volume: f32,
        pitch: f32,
        except_actor: bool,
    ) {
        let id = self.native_acting_entity();
        let Some(state) = self.server_sound_entity_state(id) else {
            return;
        };
        let except = except_actor.then_some(id);
        self.server_play_bound_sound(
            event,
            source,
            state.position,
            volume,
            pitch,
            id,
            state.silent,
            except,
        );
    }

    pub(in crate::game::server_side) fn server_sound_entity_state(
        &self,
        id: u32,
    ) -> Option<SoundEntityState> {
        if id == self.native_acting_entity()
            && let Some(player) = self.common.player.as_ref()
        {
            return Some(SoundEntityState {
                position: [player.position.x, player.position.y, player.position.z],
                silent: saved_silent(self.common.level.player_extra.as_ref()),
                removed: false,
            });
        }
        if let Some(guest) = self.server.lan.as_ref().and_then(|host| {
            host.hub.guests.iter().find(|g| {
                g.entity_id as u32 == id && g.placed && g.dimension == self.common.dim.id()
            })
        }) {
            return Some(SoundEntityState {
                position: guest.position,
                silent: saved_silent(guest.saved.as_ref()),
                removed: false,
            });
        }
        if let Some(t) = self.common.tridents.iter().find(|t| t.id == id) {
            return Some(SoundEntityState {
                position: [t.position.x, t.position.y, t.position.z],
                silent: false,
                removed: false,
            });
        }
        self.common
            .mobs
            .iter()
            .find(|m| m.id == id)
            .map(|m| SoundEntityState {
                position: [m.position.x, m.position.y, m.position.z],
                silent: m.silent,
                removed: m.removed,
            })
    }
}
