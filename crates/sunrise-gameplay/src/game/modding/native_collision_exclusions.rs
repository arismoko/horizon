//! A player's collision exclusions for mods (#3717): boxes of its own level in which that level's
//! blocks don't stop it, as a portal in a wall needs. The server keeps them on the guest, and its
//! body takes them at each turn; a client keeps its own player's on the body it moves. Neither
//! side sends its set to the other: a mod sets both.
use super::commands::Target;
use mod_api::{CollisionExclusionsError, EntityRef};
use sunrise_gameplay_common::game::native_collision_exclusions::refused;
use sunrise_world::collision_exclusions::Refused;

impl crate::game::ServerGame<'_> {
    /// For a server mod: the player's collision exclusions, in place of the last ones (the
    /// guest's, which its moves are checked against, and its body's). Refused for a stale handle,
    /// an entity that isn't a player, and boxes the set refuses; the set there was stays.
    pub(in crate::game) fn entity_set_collision_exclusions(
        &mut self,
        entity: EntityRef,
        boxes: &[[f64; 6]],
    ) -> Result<(), CollisionExclusionsError> {
        let conn = match self
            .native_entity_target(entity)
            .ok_or(CollisionExclusionsError::Stale)?
        {
            Target::Guest(conn) => conn,
            Target::Player => self
                .server
                .lan_acting
                .ok_or(CollisionExclusionsError::Stale)?,
            _ => return Err(CollisionExclusionsError::NotAPlayer),
        };
        // During the guest's own turn its body is the game's player.
        let acting = self.server.lan_acting == Some(conn);
        let host = self
            .server
            .lan
            .as_mut()
            .ok_or(CollisionExclusionsError::Stale)?;
        let turn_body = host
            .hub
            .update_guest(conn, |g| {
                g.collision_exclusions.set(boxes)?;
                if let Some(body) = g.motion.as_deref_mut() {
                    body.collision_exclusions
                        .clone_from(&g.collision_exclusions);
                }
                Ok::<_, Refused>(acting.then(|| g.collision_exclusions.clone()))
            })
            .ok_or(CollisionExclusionsError::Stale)?
            .map_err(refused)?;
        if let (Some(set), Some(body)) = (turn_body, self.common.player.as_mut()) {
            body.collision_exclusions = set;
        }
        Ok(())
    }
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;
