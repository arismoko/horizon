use super::Target;
use sunrise_command::args::Selector;
use sunrise_game_data::text::Text;

impl crate::game::ServerGame<'_> {
    /// Run `f` for the player `t`: in place, or with the guest's state swapped
    /// in, in their level, as their own turn. Return `None` if the target is
    /// not a reachable player or their level cannot be entered.
    /// What `f` tells the player (`system_message`) goes to their chat; the
    /// command's own feedback is the caller's to send.
    pub fn for_player<T>(&mut self, t: Target, f: impl FnOnce(&mut Self) -> T) -> Option<T> {
        let conn = match t {
            Target::Player => return Some(f(self)),
            // Their body is already in place, but a nested call can reach here in another
            // level than theirs: run in theirs, with no second body swap.
            Target::Guest(conn) if self.server.lan_acting == Some(conn) => {
                let back = self.common.dim;
                let theirs = super::super::Dim::from_id(
                    &self.server.lan.as_ref()?.hub.guest(conn)?.dimension,
                );
                if theirs == back {
                    return Some(f(self));
                }
                if let Err(e) = self.enter_level(theirs) {
                    log::error!("could not open {}: {e}", theirs.id());
                    return None;
                }
                let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(self)));
                if self.common.dim != back
                    && let Err(e) = self.enter_level(back)
                {
                    log::error!("could not reopen {}: {e}", back.id());
                }
                return match out {
                    Ok(out) => Some(out),
                    Err(payload) => std::panic::resume_unwind(payload),
                };
            }
            Target::Guest(conn) => conn,
            _ => return None,
        };
        let back = self.common.dim;
        let theirs =
            super::super::Dim::from_id(&self.server.lan.as_ref()?.hub.guest(conn)?.dimension);
        // Opening a level or leaving the guest can itself fire hooks. Put the outer level
        // back even when one of those hooks, rather than `f`, unwinds.
        let mut messages = Vec::new();
        let turn = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if theirs != back
                && let Err(e) = self.enter_level(theirs)
            {
                log::error!("could not open {}: {e}", theirs.id());
                return None;
            }
            self.with_guest_swap(conn, back, |game| {
                let (result, told) = game.capture_guest_messages(f);
                messages = told;
                if result.is_err() {
                    game.record_interrupted_turn(conn);
                }
                result
            })
        }));
        self.send_guest_messages(conn, messages);
        if self.common.dim != back
            && let Err(e) = self.enter_level(back)
        {
            log::error!("could not reopen {}: {e}", back.id());
        }
        match turn {
            Ok(Some(Ok(out))) => Some(out),
            Ok(None) => None,
            Ok(Some(Err(payload))) | Err(payload) => std::panic::resume_unwind(payload),
        }
    }
    /// The guests a selector looks at, in join order (`PlayerList.getPlayers`),
    /// the one in place as `Target::Player`: all of them, or those in the
    /// source's level when it limits the search to a place
    /// (`isWorldLimited`); with only the `Entity::isAlive` ones when
    /// `alive_only` (the built-in `@e` and `@n`).
    pub(in crate::game::server_side) fn player_candidates(
        &self,
        s: &Selector,
        alive_only: bool,
    ) -> Vec<Target> {
        let limited = s.distance.is_some()
            || s.pos.iter().any(Option::is_some)
            || s.delta.iter().any(Option::is_some);
        let here = self.common.dim.id();
        let in_place = self
            .server
            .lan_acting
            .filter(|_| self.common.player.is_some());
        self.server
            .lan
            .iter()
            .flat_map(|h| h.hub.online_players())
            .filter(|g| self.native_command_player_available(g))
            .filter(|g| !limited || g.dimension == here || Some(g.conn) == in_place)
            .filter(|g| !alive_only || (!g.dead && g.health > 0.0))
            .map(|g| {
                if Some(g.conn) == in_place {
                    Target::Player
                } else {
                    Target::Guest(g.conn)
                }
            })
            .collect()
    }
    /// `PlayerList.getPlayerByName`: names are not case sensitive. Sunrise:
    /// on a server where a mod gave players nicknames
    /// (`native_set_nickname`), a name no player has finds the player going
    /// by it, without its `~` or with it (quoted), so `/msg` and `/tell`
    /// reach a player by the name chat shows. No nickname, no change.
    pub(in crate::game::server_side) fn player_named(&self, name: &str) -> Option<Target> {
        if !self.server.lan_acting_fake
            && self.common.player.is_some()
            && name.eq_ignore_ascii_case(self.acting_player_name())
        {
            return Some(Target::Player);
        }
        self.command_players()
            .find(|g| g.name.eq_ignore_ascii_case(name))
            .map(|g| Target::Guest(g.conn))
            .or_else(|| self.player_nicknamed(name))
    }
    /// The name of the server's own source: "Rcon" for a command an RCON client sent
    /// (`RconConsoleSource`), "Server" otherwise (`MinecraftServer.createCommandSourceStack`).
    pub(in crate::game::server_side) fn server_name(&self) -> Text {
        Text::literal(if self.server.rcon_source {
            "Rcon"
        } else {
            "Server"
        })
    }
    /// `CommandSourceStack.getDisplayName`: the executing entity's name, the
    /// command block's or minecart's own name when the source has no entity,
    /// or the server's.
    pub(in crate::game::server_side) fn source_display_name(&self) -> Text {
        match &self.server.command_source {
            Some(source) => match source.entity {
                Some(target) => self.target_name(target),
                None => source
                    .block
                    .and_then(|pos| match self.common.world.block_entities.get(&pos) {
                        Some(sunrise_level::containers::BlockEntity::CommandBlock(block)) => {
                            Some(block.name())
                        }
                        _ => None,
                    })
                    .or_else(|| {
                        source.cart.and_then(|id| {
                            self.common
                                .carts
                                .iter()
                                .find(|cart| cart.id == id)
                                .map(|cart| cart.command.name())
                        })
                    })
                    .unwrap_or_else(|| self.server_name()),
            },
            None => self.player_display_name(),
        }
    }
    /// The entity the command runs as (`CommandSourceStack.getEntity`): a
    /// command typed by the player in place runs as them, the server
    /// console's as nobody.
    pub fn source_entity(&self) -> Option<Target> {
        match &self.server.command_source {
            Some(s) => s.entity,
            None => self.common.player.is_some().then_some(Target::Player),
        }
    }
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;
