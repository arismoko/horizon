//! Server-side `PlayerRef` post-effect access (`ServerPlayer.getPostEffects`,
//! `addPostEffect`, `removePostEffect`, `clearPostEffects`), sharing state and
//! `ClientboundPostEffectsPacket` dirty tracking with `/posteffect`.

use crate::game::ServerGame;
use mod_api::{Identifier, PlayerRef, PlayerWorldRefusal};

enum PostEffectsTarget {
    Guest(u32),
    Local,
}

fn matches_effect(stored: &str, effect: &Identifier) -> bool {
    match stored.split_once(':') {
        Some((ns, path)) => {
            (if ns.is_empty() { "minecraft" } else { ns }) == effect.namespace()
                && path == effect.path()
        }
        None => effect.namespace() == "minecraft" && stored == effect.path(),
    }
}

impl ServerGame<'_> {
    fn post_effects_target(
        &self,
        player: PlayerRef,
    ) -> Result<PostEffectsTarget, PlayerWorldRefusal> {
        if !self.is_logical_server() {
            return Err(PlayerWorldRefusal::Client);
        }
        if self.common.native_lifecycle.epoch() != Some(player.entity().epoch()) {
            return Err(PlayerWorldRefusal::Stale);
        }
        if let Some(host) = self.server.lan.as_ref() {
            if let Some(index) = host
                .hub
                .guests
                .identity_index(player.uuid(), player.entity().id())
            {
                let guest = &host.hub.guests[index];
                if (guest.joined() && guest.participates_online())
                    || (guest.placed && guest.is_fake())
                {
                    return Ok(PostEffectsTarget::Guest(guest.conn));
                }
            }
            return Err(PlayerWorldRefusal::Stale);
        }
        if self.server.serving.is_some()
            && self
                .common
                .level
                .player_uuid
                .map(sunrise_sim::lan::playerdata::ints_uuid)
                == Some(player.uuid())
            && self
                .common
                .player
                .as_ref()
                .is_some_and(|body| body.entity_id as u32 == player.entity().id())
        {
            return Ok(PostEffectsTarget::Local);
        }
        Err(PlayerWorldRefusal::Stale)
    }

    fn edit_player_post_effects(
        &mut self,
        player: PlayerRef,
        edit: impl FnOnce(&mut Vec<String>) -> bool,
    ) -> Result<bool, PlayerWorldRefusal> {
        match self.post_effects_target(player)? {
            PostEffectsTarget::Guest(conn) => self
                .server
                .lan
                .as_mut()
                .and_then(|host| {
                    host.hub.update_guest(conn, |guest| {
                        let changed = edit(&mut guest.post_effects);
                        guest.post_effects_dirty |= changed;
                        changed
                    })
                })
                .ok_or(PlayerWorldRefusal::Stale),
            PostEffectsTarget::Local => Ok(edit(&mut self.server.cmds.0.post_effects)),
        }
    }

    /// `ServerPlayer.getPostEffects`: ordered active post effects on this server player.
    pub(in crate::game) fn player_post_effects(
        &self,
        player: PlayerRef,
    ) -> Result<Vec<Identifier>, PlayerWorldRefusal> {
        let list = match self.post_effects_target(player)? {
            PostEffectsTarget::Guest(conn) => {
                &self
                    .server
                    .lan
                    .as_ref()
                    .and_then(|host| host.hub.guest(conn))
                    .ok_or(PlayerWorldRefusal::Stale)?
                    .post_effects
            }
            PostEffectsTarget::Local => &self.server.cmds.0.post_effects,
        };
        Ok(list
            .iter()
            .filter_map(|entry| Identifier::parse(entry).ok())
            .collect())
    }

    /// `ServerPlayer.addPostEffect`: appends `effect` if absent and marks dirty for sync.
    pub(in crate::game) fn player_add_post_effect(
        &mut self,
        player: PlayerRef,
        effect: &Identifier,
    ) -> Result<bool, PlayerWorldRefusal> {
        self.edit_player_post_effects(player, |list| {
            if list.iter().any(|entry| matches_effect(entry, effect)) {
                false
            } else {
                list.push(effect.to_string());
                true
            }
        })
    }

    /// `ServerPlayer.removePostEffect`: removes `effect` if present and marks dirty for sync.
    pub(in crate::game) fn player_remove_post_effect(
        &mut self,
        player: PlayerRef,
        effect: &Identifier,
    ) -> Result<bool, PlayerWorldRefusal> {
        self.edit_player_post_effects(player, |list| {
            let Some(pos) = list.iter().position(|entry| matches_effect(entry, effect)) else {
                return false;
            };
            list.remove(pos);
            true
        })
    }

    /// `ServerPlayer.clearPostEffects`: clears all active post effects and marks dirty for sync.
    pub(in crate::game) fn player_clear_post_effects(
        &mut self,
        player: PlayerRef,
    ) -> Result<bool, PlayerWorldRefusal> {
        self.edit_player_post_effects(player, |list| {
            if list.is_empty() {
                false
            } else {
                list.clear();
                true
            }
        })
    }
}
