use super::{Dim, respawn_anchor::RespawnConfig};
use mod_api::__private::game::*;
use mod_api::LevelRef;
use mod_api::{
    ActionOutcome, ActionRefused, EntityRef, ExperienceOrbRef, ExperienceOrbWriteError, FakeEffect,
    GameMode, GlobalPos, Identifier, LevelReadError, PlayerAction, PlayerActionSetup,
    PlayerDataRefusal, PlayerRef, PlayerRespawnConfig, ScopedActionResult, Sidebar, TextFormError,
    VirtualHost,
};
use serde_json::json;
#[allow(unused_imports)]
use std::sync::{LazyLock, Mutex, atomic::Ordering};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use sunrise_command::{Arg, ArgType, Context};
use sunrise_core::position::BlockPos;
use sunrise_game_data::text::Text;
use sunrise_gameplay_common::game::api_values::TryIntoApi;
use sunrise_gameplay_common::game::native_server::Acting;
use sunrise_gameplay_common::game::native_server::BaseTree;
use sunrise_gameplay_common::game::native_server::LevelTickListeners;
use sunrise_gameplay_common::game::native_server::PlayerRead;
use sunrise_gameplay_common::game::native_server::ScopedPlayerState;
use sunrise_gameplay_common::game::native_server::argument_kinds::command_arguments;
use sunrise_gameplay_common::game::native_server::argument_kinds::{
    custom_value, dimension_value, nbt_value, parsed_type, positional_value,
};
use sunrise_gameplay_common::game::native_server::command_tree_for;
use sunrise_gameplay_common::game::native_server::online_mode_of;
use sunrise_gameplay_common::game::native_server::orb_meets;
use sunrise_gameplay_common::game::native_server::ordered_box;
use sunrise_gameplay_common::game::native_server::register_command_nodes;
use sunrise_gameplay_common::game::native_server::system_chat;
use sunrise_gameplay_common::game::{GameGameExt1 as _, GameGameExt3 as _};
use sunrise_items::items::ItemStack;
use sunrise_level::world::World;
use sunrise_protocol::packets::play::{self, Clientbound as C};
use sunrise_sim::{
    lan::{
        abilities::PlayerAbilities, data::component_nbt, guest::Guest, scoreboard::SidebarContents,
    },
    motion::PlayerMotion,
};

#[path = "native_server/registered_commands_server.rs"]
pub(in crate::game) mod registered_commands;

/// Helpers for server mods, kept as stable as the hook points.
// Native mods use these; nothing in Sunrise does.
#[allow(dead_code)]
/// The resident record a server write will borrow, without borrowing `Game` itself.
pub(in crate::game) enum PlayerTurn {
    Acting,
    Parked(u32),
}

impl PlayerTurn {
    pub(in crate::game) fn run<'a, R>(
        self,
        game: &mut crate::game::ServerGame<'a>,
        f: impl FnOnce(&mut crate::game::ServerGame<'a>) -> R,
    ) -> Option<R> {
        match self {
            Self::Acting => Some(f(game)),
            Self::Parked(conn) => game.as_guest(conn, f),
        }
    }
}

impl crate::game::ServerGame<'_> {
    /// Whether the acting player is in the level in place: a guest's own level is their hub
    /// record's, also when a hook of another level borrows them into its turn. A raw throw,
    /// equip sound or block break requires that level in place; `/give`'s effects replay in
    /// that level separately.
    pub(in crate::game) fn acting_in_level_in_place(&self) -> bool {
        let Some(conn) = self.server.lan_acting else {
            return true;
        };
        self.server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .is_none_or(|g| g.dimension == self.common.dim.id())
    }
    /// Discard state belonging to an already removed connectionless guest.
    pub(in crate::game) fn cleanup_fake_player(
        &mut self,
        host: &mut super::lan::LanHost,
        guest: &Guest,
    ) {
        host.play.remove(&guest.conn);
        host.forget_menus(guest.conn);
        host.poses.remove(&guest.conn);
        host.mob_players.retain(|_, conn| *conn != guest.conn);
        self.server.guest_combat.0.remove(&guest.conn);
    }
    pub(in crate::game) fn fake_action_actor(
        &self,
        player: PlayerRef,
        action: PlayerAction,
    ) -> Result<(usize, u32), ActionRefused> {
        if self.common.native_lifecycle.epoch() != Some(player.fields().entity.epoch()) {
            return Err(ActionRefused::Stale);
        }
        let found = self.server.lan.as_ref().and_then(|host| {
            let index = host
                .hub
                .guests
                .identity_index(player.fields().uuid, player.fields().entity.id())?;
            let guest = host.hub.guests.get(index)?;
            guest.placed.then_some((index, guest))
        });
        let (gi, conn) = match found {
            Some((gi, g)) if g.is_fake() => (gi, g.conn),
            Some(_) => return Err(ActionRefused::NotFake),
            // Not in the hub's list as itself: the acting player, or gone.
            None => {
                return Err(match self.player_is_fake(player) {
                    Some(false) => ActionRefused::NotFake,
                    Some(true) => ActionRefused::Busy,
                    None => ActionRefused::Stale,
                });
            }
        };
        if !self.server.lan_guest_swaps.is_empty()
            || !(self.common.transactions.is_empty() || self.in_level_tick_boundary_alone())
            || self.server.fake_action.0.acting()
        {
            return Err(ActionRefused::Busy);
        }
        if let PlayerAction::InteractOn { location, .. } = action
            && !location.iter().all(|v| v.is_finite())
        {
            return Err(ActionRefused::InvalidSetup);
        }
        if let PlayerAction::Attack { target } | PlayerAction::InteractOn { target, .. } = action
            && (self.common.native_lifecycle.epoch() != Some(target.epoch())
                || target.id() == player.fields().entity.id())
        {
            return Err(ActionRefused::InvalidTarget);
        }
        Ok((gi, conn))
    }

    /// Whether the only open transaction is the one a server level tick's dispatch opens around
    /// each listener (#5750): a machine's fake player acts from its level tick.
    pub(in crate::game) fn in_level_tick_boundary_alone(&self) -> bool {
        LevelTickListeners::marked() && self.transactions.len() == 1
    }
    /// Whether `effect` must not start because the player acting is a fake one: true stops it,
    /// and, inside `perform_action`, records it for the action's answer. The one check every
    /// effect a fake can't take part in makes where it starts.
    pub fn fake_blocks(&self, effect: FakeEffect) -> bool {
        if !self.server.lan_acting_fake {
            return false;
        }
        self.server.fake_action.0.block(effect);
        true
    }
    /// Create or retrieve a connectionless player for the level currently in place.
    pub fn fake_player(&mut self, level: LevelRef, uuid: u128, name: &str) -> Option<PlayerRef> {
        if !self.level_in_place(level)
            || !self.server.lan_guest_swaps.is_empty()
            || !sunrise_sim::offline_name::valid_name(name)
        {
            return None;
        }
        let epoch = self.common.native_lifecycle.epoch()?;
        if self
            .common
            .level
            .player_uuid
            .map(sunrise_sim::lan::playerdata::ints_uuid)
            == Some(uuid)
        {
            return None;
        }
        let host = self.server.lan.as_ref()?;
        if host
            .hub
            .guests
            .iter()
            .any(|g| g.uuid == uuid && g.participates_online())
        {
            return None;
        }
        if let Some(g) = host
            .hub
            .guests
            .iter()
            .find(|g| g.uuid == uuid && g.dimension == level.dimension_id())
        {
            return (g.name == name).then_some(mod_api::__private::game::player_ref(
                mod_api::EntityRef::new(epoch, g.entity_id as u32),
                uuid,
            ));
        }
        let id = self.server.next_entity.checked_add(1)?;
        let entity_id = i32::try_from(id).ok()?;
        let motion = PlayerMotion::new(
            self.common.motion,
            sunrise_core::geometry::Vec3::new(0.0, 0.0, 0.0),
        )
        .ok()?;
        let host = self.server.lan.as_mut()?;
        let conn = host.hub.lan.shared.next_id()?;
        let mut guest = Guest::connectionless(conn, uuid, name.into(), entity_id, [0.0; 3], motion);
        guest.dimension = level.dimension_id().to_owned();
        host.hub.guests.push(guest);
        host.play.insert(
            conn,
            crate::game::server_side::lan_actions_server::GuestPlay::default(),
        );
        self.server.next_entity = id;
        Some(mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(epoch, id),
            uuid,
        ))
    }
    /// The player whose action or command a hook hears, as their state is
    /// swapped in (command executors, `native_right_click_block`,
    /// `native_use_on`, `native_break_block`...): None outside such a
    /// hook.
    pub fn native_acting_player(&self) -> Option<PlayerRef> {
        self.common.player.as_ref()?;
        self.server.lan_acting_name.as_ref()?;
        self.server.lan_acting_permission?;
        Some(mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(
                self.common.native_lifecycle.epoch()?,
                self.native_acting_entity(),
            ),
            self.server.lan_acting_uuid?,
        ))
    }
    /// Runs `f` with the player `uuid`'s state in place, against the level in place: theirs
    /// already when they are acting (a guest in their turn, or the `Game`'s own player where no
    /// guest holds them), else a guest's through `as_guest`, including within another guest's
    /// turn. What `f` writes to the player's own state holds wherever they are; what would land
    /// in the world checks [`Self::acting_in_level_in_place`]. False when it didn't run: no such
    /// player here, or dead.
    pub(in crate::game) fn native_as_player(
        &mut self,
        uuid: u128,
        f: impl FnOnce(&mut Self),
    ) -> bool {
        if self.server.lan_acting_uuid == Some(uuid) {
            return self.native_as_acting(f);
        }
        // With no guest's turn running, `native_acting_id` is the world's owner: on a server
        // thread that owner is a guest too, whose state is theirs, not the `Game`'s own.
        let conn = self
            .native_player_holder(uuid)
            .filter(|g| !g.dead)
            .map(|g| g.conn);
        match conn {
            Some(conn) => self.as_guest(conn, f).is_some(),
            None if uuid == self.native_acting_id() => self.native_as_acting(f),
            None => false,
        }
    }
    /// Runs `f` in the turn of the player a handle names, against the level in place. False
    /// when it didn't run: stale or dead.
    pub fn native_as_player_handle(
        &mut self,
        player: PlayerRef,
        f: impl FnOnce(&mut Self),
    ) -> bool {
        self.player_handle_turn(player, true)
            .and_then(|turn| turn.run(self, f))
            .is_some()
    }

    /// Resolve a server setter's receiver. Dead players still have a writable record.
    pub(in crate::game) fn native_resident_player_turn(
        &self,
        player: PlayerRef,
    ) -> Result<PlayerTurn, PlayerDataRefusal> {
        if !self.common.is_logical_server() {
            return Err(PlayerDataRefusal::Client);
        }
        self.player_handle_turn(player, false)
            .ok_or(PlayerDataRefusal::Stale)
    }

    pub(in crate::game) fn native_as_resident_player(
        &mut self,
        player: PlayerRef,
        f: impl FnOnce(&mut Self),
    ) -> bool {
        self.native_resident_player_turn(player)
            .ok()
            .and_then(|turn| turn.run(self, f))
            .is_some()
    }

    fn player_handle_turn(&self, player: PlayerRef, require_living: bool) -> Option<PlayerTurn> {
        match self.player_handle_read(player)? {
            PlayerRead::Acting { body, .. } => {
                if require_living {
                    self.native_live_acting_body()?;
                } else {
                    body?;
                }
                Some(PlayerTurn::Acting)
            }
            PlayerRead::Guest(guest) if !require_living || !guest.dead => {
                Some(PlayerTurn::Parked(guest.conn))
            }
            PlayerRead::Guest(_) => None,
        }
    }
    /// Resolve the source once at the command boundary, retaining its output origin.
    pub fn native_command_origin(
        &mut self,
        permission: u8,
    ) -> super::commands::R<mod_api::CommandSource> {
        use super::commands::err;
        use mod_api::CommandSource;
        let permission = mod_api::PermissionLevel::from_level(i32::from(permission));
        let source = self
            .server
            .command_source
            .clone()
            .unwrap_or_else(|| self.player_source());
        let origin = if let Some(id) = self.server.exec.0.functions.last() {
            CommandSource::Function {
                id: mod_api::Identifier::parse(id).map_err(|_| err("command.failed", &[]))?,
                permission,
            }
        } else if let Some((x, y, z)) = source.block {
            CommandSource::CommandBlock {
                position: [x, y, z],
                level: source.origin_dimension.key(),
                permission,
            }
        } else if let Some(id) = source.cart {
            let entity = self
                .with_source(
                    super::commands::execute::Source {
                        dimension: source.origin_dimension,
                        ..source
                    },
                    |game| game.entity(id),
                )
                .ok_or_else(|| err("argument.entity.notfound.entity", &[]))?;
            CommandSource::CommandBlockMinecart { entity, permission }
        } else if source.origin_player {
            let player = self
                .native_acting_player()
                .ok_or_else(|| err("argument.entity.notfound.player", &[]))?;
            CommandSource::Player { player, permission }
        } else {
            CommandSource::Console { permission }
        };
        Ok(origin)
    }
    /// `CommandSourceStack.sendFailure` for a mod's executor: the red line to the originating
    /// source, using vanilla's silence and block/cart track-output rules. The command's result
    /// is whatever the executor returns.
    pub(in crate::game) fn native_failure(&mut self, line: Text) {
        let source = self
            .server
            .command_source
            .clone()
            .unwrap_or_else(|| self.player_source());
        self.with_source(
            super::commands::execute::Source {
                dimension: source.origin_dimension,
                ..source
            },
            |game| game.command_failure(line),
        );
    }
    /// A command's success feedback: `CommandSourceStack.sendSuccess(line,
    /// broadcast_to_admins)` from the executor's source, the line to its
    /// originating source and, with `broadcast_to_admins`, the other operators
    /// and the log (`send_success`). The executing level is in place: the
    /// entity's name is read there, the origin block or minecart in its own.
    pub fn native_feedback(&mut self, line: Text, broadcast_to_admins: bool) {
        let source = self
            .server
            .command_source
            .clone()
            .unwrap_or_else(|| self.player_source());
        self.with_source(source, |game| game.send_success(line, broadcast_to_admins));
    }
    /// Asks the mods what root commands each player's tree keeps
    /// (`native_player_commands_sent`) and stores the answer in the hub,
    /// Commands.sendCommands: build this recipient's tree once, let the installed
    /// listeners select exclusions, then encode and send that same tree.
    pub fn send_player_command_tree(&mut self, conn: u32) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let Some(i) = host.hub.guests.connection_index(conn) else {
            return;
        };
        let guest = &host.hub.guests[i];
        if !guest.participates_online() {
            return;
        }
        let graph = host.hub.command_graph(guest.permission);
        let mut hidden = Vec::new();
        if mod_api::__private::listened(mod_api::HookId::PlayerCommandsSent)
            && let Some(epoch) = self.common.native_lifecycle.epoch()
        {
            let player = mod_api::__private::game::player_ref(
                mod_api::EntityRef::new(epoch, guest.entity_id as u32),
                guest.uuid,
            );
            let mut dispatch = |game: &Self| {
                let roots = graph.roots();
                let send = Arc::new(());
                let name_at = |index: usize| roots.clone().nth(index).expect("indexed usable root");
                let mut answers = |answer: mod_api::Answered<mod_api::CommandTreeSelection>| {
                    if let Some(indexes) =
                        mod_api::__private::selected_command_roots(&send, answer.answer)
                    {
                        hidden.extend(indexes);
                    } else {
                        log::warn!(
                            "mod {}: player_commands_sent returned an exclusion from a different command tree",
                            answer.mod_id
                        );
                    }
                };
                game.native_player_commands_sent(mod_api::__private::PlayerCommandsSentCall {
                    player,
                    tree: mod_api::__private::command_tree_filter(roots.len(), &name_at, &send),
                    answers: &mut answers,
                });
            };
            if !guest.placed && self.server.lan_acting != Some(conn) {
                // The joining body exists but is not in the online player list yet.
                if let Err(error) = self.as_guest_at(i, conn, |game| dispatch(game)) {
                    log::error!(
                        "player_commands_sent: cannot lend the joining player for connection {conn}: {error:?}"
                    );
                }
            } else {
                dispatch(self);
            }
        }
        // A joining turn may have moved hub rows while settling callbacks.
        if let Some(host) = self.server.lan.as_ref()
            && let Some(i) = host.hub.guests.connection_index(conn)
        {
            let packet = host.hub.encode_command_graph(graph, &hidden);
            host.hub.send_commands(i, packet);
        }
    }
    /// Resolve a UUID-based item place in the acting state or a parked guest's own record.
    /// Unlike a write, a read never initializes a body or the level owner's UUID.
    pub(in crate::game::server_side) fn native_item_place_player(
        &self,
        uuid: u128,
    ) -> Option<PlayerRead<'_>> {
        if self.server.lan_acting_uuid != Some(uuid) {
            if let Some(guest) = self.native_player_holder(uuid) {
                return (!guest.dead && !self.server.lan_guest_swaps.contains(&guest.conn))
                    .then_some(PlayerRead::Guest(guest));
            }
            if self.server.lan_acting_uuid.is_some()
                || self
                    .common
                    .level
                    .player_uuid
                    .map(sunrise_sim::lan::playerdata::ints_uuid)
                    != Some(uuid)
            {
                return None;
            }
        }
        Some(PlayerRead::Acting {
            mode: Some(self.common.mode_id()),
            body: Some(self.native_live_acting_body()?),
            abilities: self.server.acting_abilities,
            last_death: self.common.level.last_death.as_ref(),
        })
    }
    /// A mod changed the commands it registers (`native_register_commands`):
    /// at the start of the next server tick every mod is asked again, the
    /// commands players may run are rebuilt, and every player on the server is
    /// sent their tree again, after `native_player_commands_sent`. Nothing
    /// happens where no world is open to players, and nothing for a server
    /// whose mods never call it. API @API:refresh_commands@.
    pub(in crate::game) fn native_refresh_commands(&mut self) {
        self.server.native_commands.0.refresh = true;
    }
    /// The real players on this server, in join order, including an integrated
    /// server's own player even when the world is not open to LAN. Empty on a client.
    pub fn native_server_players(&self) -> Vec<PlayerRef> {
        let Some(epoch) = self.common.native_lifecycle.epoch() else {
            return Vec::new();
        };
        let Some(host) = &self.server.lan else {
            return Vec::new();
        };
        // Construct from the online records, not the live-body resolver: outer
        // players remain connected while their bodies are displaced by a turn.
        host.hub
            .online_players()
            .map(|g| {
                mod_api::__private::game::player_ref(
                    mod_api::EntityRef::new(epoch, g.entity_id as u32),
                    g.uuid,
                )
            })
            .collect()
    }
    pub(in crate::game) fn perform_fake_action(
        &mut self,
        gi: usize,
        conn: u32,
        action: PlayerAction,
    ) -> Result<ActionOutcome, ActionRefused> {
        let acting = Acting::start(&self.server.fake_action.0);
        let failed = std::mem::replace(&mut self.server.use_failed, false);
        let consumed = (|| match action {
            PlayerAction::UseOn { hand, pos, hit } => {
                let hit = super::Hit {
                    pos,
                    face: hit.face,
                    point: hit.point,
                    inside: hit.inside,
                };
                self.guest_use_item_on(conn, gi, hit, hand, None)
            }
            PlayerAction::UseItem { hand } => self.guest_use_item(conn, gi, hand, None),
            PlayerAction::InteractOn {
                hand,
                target,
                location,
            } => {
                let id = i32::try_from(target.id()).map_err(|_| ActionRefused::InvalidTarget)?;
                self.guest_interact(conn, gi, id, hand, location)
            }
            PlayerAction::Attack { target } => {
                let id = i32::try_from(target.id()).map_err(|_| ActionRefused::InvalidTarget)?;
                self.guest_attack(conn, gi, id)
            }
            PlayerAction::BreakBlock { pos } => self.guest_break_block(conn, gi, pos),
        })();
        // FAIL ends the click, but is not a consuming InteractionResult.
        let consumed = consumed.map(|used| used && !self.server.use_failed);
        self.server.use_failed = failed;
        let consumed = consumed?;
        Ok(match acting.blocked() {
            Some(effect) => ActionOutcome::EffectBlocked(effect),
            None if consumed => ActionOutcome::Consumed,
            None => ActionOutcome::Passed,
        })
    }
    /// A guest's command for the mods, inside their swap (`run_guest_command`).
    /// `None` leaves dispatch to the caller; `Some(false)` is a handled failure.
    pub fn perform_native_command(
        &mut self,
        conn: u32,
        permission: u8,
        line: &str,
        quiet: bool,
    ) -> Option<bool> {
        let host = self.server.lan.as_ref()?;
        if self.server.native_commands.0.level(line)? > permission {
            return None;
        }
        host.hub.guest(conn)?;
        let previous = std::mem::replace(&mut self.common.ui.registries.permission, permission);
        let ok = if quiet {
            self.run_command_quietly(line)
        } else {
            self.run_command(line)
        };
        self.common.ui.registries.permission = previous;
        Some(ok)
    }
    /// A console line for the mods: true when a mod registered its command
    /// (`native_register_commands`). The console is level 4, so a registered
    /// usage is always allowed; a command no mod takes answers as vanilla's
    /// unknown command.
    pub fn perform_native_console_command(&mut self, line: &str) -> bool {
        if self.server.lan.is_none() || self.server.native_commands.0.level(line).is_none() {
            return false;
        }
        self.as_console_in_spawn_level(|game| {
            let previous = game
                .server
                .command_source
                .replace(super::commands::execute::Source {
                    silent: false,
                    ..game.server_source()
                });
            game.run_command(line);
            game.server.command_source = previous;
        });
        true
    }
    pub(in crate::game) fn server_player(&self, uuid: u128) -> Option<PlayerRef> {
        let epoch = self.common.native_lifecycle.epoch()?;
        let (_, id) = self.player_lookup(uuid)?;
        Some(mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(epoch, id),
            uuid,
        ))
    }
    pub(in crate::game) fn server_player_read_both(
        &self,
        player: PlayerRef,
    ) -> Option<PlayerRead<'_>> {
        self.player_handle_read(player)
    }
    pub(in crate::game) fn server_player_client_brand(&self, player: PlayerRef) -> Option<&str> {
        if let Some(guest) = self.player_connection_record(player) {
            return Some(&guest.brand);
        }
        (self.native_acting_player() == Some(player)).then_some("")
    }
    pub(in crate::game) fn server_player_collision_flags(
        &self,
        player: PlayerRef,
    ) -> Result<Option<mod_api::CollisionFlags>, mod_api::LevelReadError> {
        Ok(self.player_handle_read(player).and_then(|read| {
            // A connected player's position reports do not retain collision
            // flags, even when its body is swapped in for a server guest turn.
            let PlayerRead::Acting {
                body: Some(body), ..
            } = read
            else {
                return None;
            };
            if self.server.lan_acting.is_some() {
                return None;
            }
            let flags = body.collision;
            Some(mod_api::CollisionFlags::new(
                flags.horizontal,
                flags.minor,
                flags.vertical,
                flags.below,
            ))
        }))
    }
    /// Whether `SleepStatus.update` counts the player: not a spectator, not a
    /// fake, and not an ignored player who is out of bed (`sleep_counts`'s
    /// own rule). None for a stale or unavailable player.
    pub(in crate::game) fn player_counts_for_sleep(&self, player: PlayerRef) -> Option<bool> {
        let ignored = |game: &Self| {
            game.server
                .lan
                .as_ref()
                .is_some_and(|host| host.sleep_ignored.contains(&player.uuid()))
        };
        Some(match self.player_handle_read(player)? {
            // The player in place: the host's own, or a guest in their turn.
            PlayerRead::Acting { .. } => {
                !self.server.lan_acting_fake
                    && !self.common.spectator()
                    && (self.common.sleeping.is_some() || !ignored(self))
            }
            PlayerRead::Guest(guest) => self.guest_counts_for_sleep(guest),
        })
    }
    /// Server arm for the dimension key of an online player (`ServerPlayer.level().dimension()`).
    pub(in crate::game) fn server_player_dimension_key(
        &self,
        player: PlayerRef,
    ) -> Option<mod_api::DimensionKey> {
        let epoch = self.common.native_lifecycle.epoch()?;
        if epoch != player.fields().entity.epoch() {
            return None;
        }
        if self.native_acting_player() == Some(player) {
            let dim = match self.server.lan_acting {
                Some(conn) => {
                    let g = self.command_guest(conn)?;
                    Dim::from_id(&g.dimension).key()
                }
                None => self.common.dim.key(),
            };
            return Some(dim);
        }
        if let Some(guest) = self.online_player_record(player) {
            return Some(Dim::from_id(&guest.dimension).key());
        }
        if self.server.serving.is_some()
            && self.server_player(player.uuid()) == Some(player)
            && self
                .common
                .level
                .player_uuid
                .map(sunrise_sim::lan::playerdata::ints_uuid)
                == Some(player.uuid())
        {
            return Some(self.common.dim.key());
        }
        None
    }
    /// Whether a live player handle names a connectionless actor.
    pub fn player_is_fake(&self, player: PlayerRef) -> Option<bool> {
        match self.player_handle_read(player)? {
            PlayerRead::Acting { .. } => Some(self.server.lan_acting_fake),
            PlayerRead::Guest(guest) => Some(guest.is_fake()),
        }
    }
    /// The stable name of a connected or acting player. None for a stale or
    /// unavailable player; displaced online players retain their identity.
    pub(in crate::game) fn player_name(&self, player: PlayerRef) -> Option<&str> {
        if self.native_acting_player() == Some(player) {
            return self.server.lan_acting_name.as_deref();
        }
        Some(&self.online_player_record(player)?.name)
    }
    /// A machine's one use on the persistent actor, with its private state put back even
    /// when the action refuses. Only the existing guest turn runs gameplay and its hooks.
    pub fn player_perform_action_scoped(
        &mut self,
        player: PlayerRef,
        action: PlayerAction,
        setup: PlayerActionSetup,
    ) -> Result<ScopedActionResult, ActionRefused> {
        let (gi, conn) = self.fake_action_actor(player, action)?;
        self.beside_listener_journal(|game| game.perform_scoped(gi, conn, action, setup))
    }

    /// Runs a fake player's action outside the level tick listener's own transaction.
    pub(in crate::game) fn beside_listener_journal<T>(
        &mut self,
        act: impl FnOnce(&mut Self) -> T,
    ) -> T {
        if self.common.transactions.is_empty() {
            return act(self);
        }
        let frames = std::mem::take(&mut *self.common.transactions);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _outside = LevelTickListeners::mark(false);
            act(self)
        }));
        // Anything the action left open nests in the listener's transaction.
        let inner = std::mem::replace(&mut *self.common.transactions, frames);
        self.common.transactions.extend(inner);
        result.unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    }

    fn perform_scoped(
        &mut self,
        gi: usize,
        conn: u32,
        action: PlayerAction,
        setup: PlayerActionSetup,
    ) -> Result<ScopedActionResult, ActionRefused> {
        if !setup.eye.iter().all(|v| v.is_finite())
            || !setup.yaw.is_finite()
            || !setup.pitch.is_finite()
        {
            return Err(ActionRefused::InvalidSetup);
        }
        let full_attack_charge = setup.full_attack_charge;
        let mut state = ScopedPlayerState::new(self.common.motion, self.items(), setup)?;
        let host = self.server.lan.as_mut().expect("validated fake's host");
        host.hub.guests.update(gi, |guest| state.swap(guest));
        let play = host.play.insert(conn, Default::default());
        let menus = host.menus.insert(conn, Default::default());
        // This body is created with the setup's equipment and never reaches a body tick.
        // Install its modifiers before the first action or calculation of attack charge.
        let initialized = self.as_guest_at(gi, conn, |game| {
            game.put_equipment_modifiers();
            if full_attack_charge {
                game.charge_attack_fully();
            }
        });
        let outcome = if initialized.is_ok() {
            self.perform_fake_action(gi, conn, action)
        } else {
            Err(ActionRefused::Busy)
        };
        let host = self
            .server
            .lan
            .as_mut()
            .expect("guest turns retain the host");
        let (held, other, position) = host.hub.guests.update(gi, |guest| {
            let held = guest
                .inventory
                .get_mut(guest.selected)
                .and_then(Option::take)
                .filter(|s| s.count > 0);
            let other: Vec<sunrise_items::items::ItemStack> = guest
                .inventory
                .iter_mut()
                .chain([&mut guest.offhand, &mut guest.carried])
                .chain(guest.armor.iter_mut())
                .filter_map(Option::take)
                .filter(|s| s.count > 0)
                .collect();
            let position = guest.position;
            state.swap(guest);
            (held, other, position)
        });
        host.menus.remove(&conn);
        if let Some(menus) = menus {
            host.menus.insert(conn, menus);
        }
        host.play.remove(&conn);
        if let Some(play) = play {
            host.play.insert(conn, play);
        }
        // A stack the API can't represent (a saved item this build doesn't have, #1364) is never
        // destroyed or replaced: it goes back into the world where the actor stood, with
        // vanilla's default pickup delay so a repeating action doesn't take it again next tick.
        // It joins as any drop does, so a join listener may keep it out; only the stacks that
        // joined are counted.
        let mut unrepresentable = 0;
        let mut represent = |stack: sunrise_items::items::ItemStack| match stack.try_into_api() {
            Ok(stack) => Some(stack),
            Err(_) => {
                // ItemEntity.setDefaultPickUpDelay.
                self.push_item_entity(stack, position, None, 10, None);
                if self.join_item_entity(self.server.next_entity, position) {
                    unrepresentable += 1;
                }
                None
            }
        };
        let held = held.and_then(&mut represent);
        let other = other.into_iter().filter_map(&mut represent).collect();
        outcome.map(|outcome| ScopedActionResult::new(outcome, held, other, unrepresentable))
    }
    /// Remove a fake without treating it as a disconnect or saving it.
    pub fn player_remove_fake(&mut self, player: PlayerRef) -> bool {
        if self.common.native_lifecycle.epoch() != Some(player.fields().entity.epoch())
            || !self.server.lan_guest_swaps.is_empty()
        {
            return false;
        }
        let Some(mut host) = self.server.lan.take() else {
            return false;
        };
        let index = host.hub.guests.iter().position(|g| {
            g.is_fake()
                && g.uuid == player.fields().uuid
                && g.entity_id as u32 == player.fields().entity.id()
                && host
                    .hub
                    .holder_guest(g.uuid, &g.dimension)
                    .is_some_and(|owner| owner.conn == g.conn)
        });
        let removed = if let Some(index) = index {
            let guest = host.hub.guests.remove(index);
            self.cleanup_fake_player(&mut host, &guest);
            true
        } else {
            false
        };
        self.server.lan = Some(host);
        removed
    }
    /// `ServerPlayer.getRespawnConfig`: the player's respawn point as the
    /// game keeps it (the guest's record, or the acting player's in place),
    /// `Some(None)` when they have none. None for a stale or unavailable
    /// player, or a dimension key the API can't name.
    pub(in crate::game) fn player_respawn_config(
        &self,
        player: PlayerRef,
    ) -> Option<Option<PlayerRespawnConfig>> {
        let saved = match self.player_handle_read(player)? {
            PlayerRead::Acting { .. } => self.server.respawn_config.map(|c| c.saved()),
            PlayerRead::Guest(guest) => guest.respawn_elsewhere.clone(),
        };
        let Some((dimension, [x, y, z], yaw, _pitch, forced)) = saved else {
            return Some(None);
        };
        Some(Some(PlayerRespawnConfig {
            position: GlobalPos::new(Identifier::parse(&dimension).ok()?, BlockPos::new(x, y, z)),
            yaw,
            forced,
        }))
    }
    /// Stores a server player's `Abilities` and tells their client
    /// (`ServerPlayer.onUpdateAbilities`). They stay as written until the
    /// player's game mode changes (`GameType.updatePlayerAbilities`), and are
    /// saved with the player. Both speeds are stored as supplied. False for stale, unavailable or displaced outer players.
    pub fn player_set_abilities(
        &mut self,
        player: PlayerRef,
        abilities: mod_api::PlayerAbilities,
    ) -> bool {
        let stored = PlayerAbilities {
            invulnerable: abilities.invulnerable,
            flying: abilities.flying,
            may_fly: abilities.may_fly,
            instabuild: abilities.instabuild,
            may_build: abilities.may_build,
            flying_speed: abilities.fly_speed,
            walking_speed: abilities.walk_speed,
        };
        if self.player_handle_read(player).is_none() {
            return false;
        }
        let mut stored_here = false;
        self.native_as_player_handle(player, |game| {
            let Some(slot) = game.server.acting_abilities.as_mut() else {
                return;
            };
            *slot = stored;
            if let Some(body) = game.player.as_mut() {
                body.invulnerable = stored.invulnerable;
                body.may_fly = stored.may_fly;
                body.flying = stored.flying;
                body.flying_speed = stored.flying_speed;
            }
            stored_here = true;
        }) && stored_here
    }
    /// `Inventory.setSelectedSlot` for a server write: `slot` (0 to 8) becomes the hotbar slot the
    /// player holds and a guest's client is told (`SetHeldSlot`). A new slot ends a main-hand use,
    /// as `handleSetCarriedItem` does, without `player_switch_hotbar_slot_pre`, which is for the
    /// player's own input. False for a slot past 8 or an unavailable player.
    pub fn player_set_selected_slot(&mut self, player: PlayerRef, slot: usize) -> bool {
        if slot >= 9 {
            return false;
        }
        // The guest turn stores the selected slot back but sends nothing. Tell the same
        // connection after the turn, whether the player was parked or already acting.
        let mut conn = None;
        if !self.native_as_resident_player(player, |game| {
            conn = game.server.lan_acting;
            if game.common.selected != slot
                && game
                    .common
                    .using
                    .is_some_and(|(held, _)| held != crate::game::shield::OFFHAND_SLOT)
            {
                game.stop_active_item_use();
            }
            game.common.selected = slot;
        }) {
            return false;
        }
        if let (Some(conn), Some(host)) = (conn, self.server.lan.as_ref()) {
            host.hub.send_to(
                conn,
                &C::SetHeldSlot(play::SetHeldSlot { slot: slot as i32 }),
            );
        }
        true
    }
    /// Drop the fake players belonging to a level as it closes.
    pub(in crate::game::server_side) fn remove_level_fake_players(&mut self, dimension: &str) {
        let Some(mut host) = self.server.lan.take() else {
            return;
        };
        let mut i = 0;
        while i < host.hub.guests.len() {
            let g = &host.hub.guests[i];
            if g.is_fake() && g.dimension == dimension {
                let guest = host.hub.guests.remove(i);
                self.cleanup_fake_player(&mut host, &guest);
            } else {
                i += 1;
            }
        }
        self.server.lan = Some(host);
    }
    /// The level changes the level in place's tickets made since the last
    /// call (`ChunkTickets::take_changes`), told to the mods in order as
    /// `chunk_ticket_level_updated`, with no player's state swapped in. The
    /// tickets keep them only while a mod listens (`update_chunk_tickets`).
    pub(in crate::game::server_side) fn run_chunk_ticket_level_updates(&mut self) {
        use sunrise_sim::ticket_levels::UNLOADED;
        let changes = self.server.tickets.0.take_changes();
        if changes.is_empty() {
            return;
        }
        let Some(level) = self.level(self.common.dim.id()) else {
            return;
        };
        let told = |level| match level {
            UNLOADED => mod_api::ChunkTicketLevel::Unloaded,
            level => mod_api::ChunkTicketLevel::Loaded(level),
        };
        for change in changes {
            let chunk = mod_api::ChunkRef::new(level, [change.chunk.0, change.chunk.1]);
            self.native_chunk_ticket_level_updated(chunk, told(change.old), told(change.new));
        }
    }
    /// The chunks players started and stopped watching that the hub
    /// recorded (`lan::chunk_watch`), told to the mods in the hub's order,
    /// with no player's state swapped in, and a sent chunk's passages sent
    /// after it. The player handle is built from the record, so an unwatch
    /// after a disconnect still names them. Nothing while the log is empty.
    pub fn run_chunk_watches(&mut self) {
        use sunrise_sim::lan::chunk_watch::ChunkWatchKind;
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        if host.hub.chunk_watches.is_empty() {
            return;
        }
        let runs = host.hub.chunk_watches.take();
        let Some(epoch) = self.common.native_lifecycle.epoch() else {
            return;
        };
        for run in runs {
            // A level the server no longer holds: nothing to name.
            let Some(level) = self.level(&run.dimension) else {
                continue;
            };
            for watch in run.watches {
                let Ok(id) = u32::try_from(watch.entity_id) else {
                    continue;
                };
                let player = mod_api::__private::game::player_ref(
                    mod_api::EntityRef::new(epoch, id),
                    watch.uuid,
                );
                let chunk = mod_api::ChunkRef::new(level, watch.chunk);
                match watch.kind {
                    ChunkWatchKind::Watch => self.native_chunk_watch(player, chunk),
                    ChunkWatchKind::Sent => {
                        self.native_chunk_sent(player, chunk);
                        self.send_chunk_passages(level, watch.uuid, watch.chunk);
                    }
                    ChunkWatchKind::Unwatch => self.native_chunk_unwatch(player, chunk),
                }
            }
        }
    }
    /// The shared command-node executor, also reached by functions and `execute`.
    pub fn run_native_command(&mut self, context: Context) -> super::commands::R<i32> {
        use super::commands::err;
        use mod_api::__private::RetainedFailure;
        // Argument resolution uses node IDs from the parse. Never use them against a
        // replacement tree, even if the new tree has the same command at the same ID.
        let executor = context
            .executor
            .filter(|&key| self.common.ui.dispatcher.0.owns_executor(key))
            .and_then(|key| {
                self.server
                    .native_commands
                    .0
                    .registered
                    .as_ref()?
                    .executors
                    .get(key.slot())
            })
            .filter(|executor| executor.callback.enabled())
            .ok_or_else(|| err("command.unknown.command", &[]))?;
        let callback = executor.callback.clone();
        let call = Arc::clone(&executor.call);
        let command = self.resolve_native_command(context)?;
        let origin = self.native_command_origin(self.common.ui.registries.permission)?;
        // Isolate journaled writes when the caller already has a transaction. An ordinary
        // command is not implicitly transactional, and writes outside the journal stay put.
        let result = if self.has_open_transaction() {
            self.storage_transaction(|game, _| callback.call(|| call(game, &origin, &command)))
        } else {
            callback.call(|| call(self, &origin, &command))
        };
        match result {
            Ok(Ok(value)) => Ok(value),
            // Failure text can reach a packet or a saved command block. An
            // invalid component still reports an ordinary failed command.
            Ok(Err(message)) => Err(sunrise_sim::api_text::to_serialized_engine(&message)
                .map_or_else(
                    |_| err("command.failed", &[]),
                    sunrise_command::CommandError::new,
                )),
            Err(RetainedFailure::Disabled) => Err(err("command.unknown.command", &[])),
            Err(RetainedFailure::Panicked) => Err(sunrise_command::CommandError::unexpected()),
        }
    }
    /// The open world's folder (`world/`, beside its `level.dat`): a server
    /// mod keeps its data in a folder of its own there, so the world's
    /// backups carry it. None when no world is open.
    pub(in crate::game) fn world_dir(&self) -> Option<PathBuf> {
        (!self.server.world_root.as_os_str().is_empty()).then(|| self.server.world_root.clone())
    }

    pub fn player_perform_action(
        &mut self,
        player: PlayerRef,
        action: PlayerAction,
    ) -> Result<ActionOutcome, ActionRefused> {
        let (gi, conn) = self.fake_action_actor(player, action)?;
        self.beside_listener_journal(|game| game.perform_fake_action(gi, conn, action))
    }

    pub fn player_set_game_mode(&mut self, player: PlayerRef, mode: GameMode) -> bool {
        if self.player_handle_read(player).is_none() {
            return false;
        }
        let mut changed = false;
        self.native_as_player_handle(player, |game| {
            changed = game.set_game_mode(mode);
        }) && changed
    }

    pub fn player_drop_stack(
        &mut self,
        player: PlayerRef,
        stack: ItemStack,
        thrown_by_player: bool,
    ) -> Option<ItemStack> {
        if self.entity_ids_exhausted() {
            return Some(stack);
        }
        if stack.count == 0 {
            return Some(stack);
        }
        let copy = stack.clone();
        let mut dropped = false;
        let ran = self.native_as_player_handle(player, |game| {
            if game.acting_in_level_in_place() {
                dropped = game.drop_stack_from_eyes(stack, thrown_by_player).is_some();
            }
        });
        (!(ran && dropped)).then_some(copy)
    }

    pub(in crate::game) fn native_player_by_name(&self, name: &str) -> Option<PlayerRef> {
        let epoch = self.common.native_lifecycle.epoch()?;
        let player = self
            .server
            .lan
            .as_ref()?
            .hub
            .online_players()
            .find(|player| player.name.eq_ignore_ascii_case(name))?;
        Some(mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(epoch, player.entity_id as u32),
            player.uuid,
        ))
    }

    fn player_connection_record(&self, player: PlayerRef) -> Option<&Guest> {
        (self.common.native_lifecycle.epoch()? == player.entity().epoch()).then_some(())?;
        let guests = &self.server.lan.as_ref()?.hub.guests;
        let guest = guests.get(guests.identity_index(player.uuid(), player.entity().id())?)?;
        (guest.participates_online()
            && (guest.joined() || self.server.lan_acting == Some(guest.conn)))
        .then_some(guest)
    }

    pub(in crate::game) fn online_player_record(&self, player: PlayerRef) -> Option<&Guest> {
        self.player_connection_record(player)
            .filter(|guest| guest.joined())
    }

    pub(in crate::game) fn player_address(&self, player: PlayerRef) -> Option<&str> {
        if let Some(guest) = self.player_connection_record(player) {
            return Some(guest.ip_address());
        }
        (self.native_acting_player() == Some(player)).then_some("")
    }

    pub(in crate::game) fn player_virtual_host(&self, player: PlayerRef) -> Option<&VirtualHost> {
        if !self.common.is_logical_server() {
            return None;
        }
        self.online_player_record(player)?.sender.virtual_host()
    }

    pub(in crate::game) fn player_announced_channels(
        &self,
        player: PlayerRef,
    ) -> Option<Arc<[String]>> {
        static EMPTY: LazyLock<Arc<[String]>> = LazyLock::new(Arc::default);
        if let Some(guest) = self.player_connection_record(player) {
            return Some(guest.sender.announced.list());
        }
        (self.native_acting_player() == Some(player)).then(|| Arc::clone(&EMPTY))
    }

    pub(in crate::game) fn message_player_uuid(&self, player: PlayerRef) -> Option<u128> {
        (self.native_acting_player() == Some(player) || self.online_player_record(player).is_some())
            .then_some(player.uuid())
    }

    pub(in crate::game) fn player_tell(&mut self, player: PlayerRef, line: Text) -> bool {
        let Some(uuid) = self.message_player_uuid(player) else {
            return false;
        };
        self.native_send(uuid, system_chat(&line))
    }

    pub(in crate::game) fn player_action_bar(&mut self, player: PlayerRef, line: Text) -> bool {
        let Some(uuid) = self.message_player_uuid(player) else {
            return false;
        };
        let packet = C::SystemChat(play::SystemChat {
            content: component_nbt(&line.component()),
            overlay: true,
        });
        self.native_send(uuid, packet)
    }

    pub(in crate::game) fn native_send(&mut self, uuid: u128, packet: C) -> bool {
        if self.server.lan_acting_uuid == Some(uuid) && !self.server.lan_acting_fake {
            self.server.guest_packets.push(packet);
            return true;
        }
        let Some(host) = &self.server.lan else {
            return false;
        };
        let Some(g) = host.hub.online_players().find(|g| g.uuid == uuid) else {
            return false;
        };
        host.hub.send_to(g.conn, &packet);
        true
    }

    pub(in crate::game) fn player_set_sidebar(
        &mut self,
        player: PlayerRef,
        sidebar: Option<Sidebar>,
    ) -> Result<bool, TextFormError> {
        let Some(conn) = self.online_player_record(player).map(|guest| guest.conn) else {
            return Ok(false);
        };
        let contents = sidebar
            .map(|sidebar| {
                let items = self.common.res.item_defaults().entries();
                SidebarContents::prepare(sidebar, &|item| items.contains_key(item)).map(Box::new)
            })
            .transpose()?;
        self.after_commit(
            sunrise_gameplay_common::game::transactions::After::PersonalSidebar(conn, contents),
        );
        Ok(true)
    }

    pub(in crate::game) fn player_tab_list(
        &mut self,
        player: PlayerRef,
        header: &Text,
        footer: &Text,
    ) -> bool {
        let Some(uuid) = self.message_player_uuid(player) else {
            return false;
        };
        let packet = C::TabList(play::TabList {
            header: component_nbt(&header.component()),
            footer: component_nbt(&footer.component()),
        });
        self.native_send(uuid, packet)
    }

    pub(in crate::game) fn player_set_sleep_ignored(
        &mut self,
        player: PlayerRef,
        ignored: bool,
    ) -> bool {
        let Some(uuid) = self.sleep_list_uuid(player) else {
            return false;
        };
        let Some(host) = &mut self.server.lan else {
            return false;
        };
        if ignored {
            host.sleep_ignored.insert(uuid);
        } else {
            host.sleep_ignored.remove(&uuid);
        }
        true
    }

    pub(in crate::game) fn sleep_list_uuid(&self, player: PlayerRef) -> Option<u128> {
        let uuid = self.message_player_uuid(player)?;
        self.server
            .lan
            .as_ref()?
            .hub
            .online_players()
            .any(|g| g.uuid == uuid)
            .then_some(uuid)
    }

    pub(in crate::game) fn player_sleep_ignored(&self, player: PlayerRef) -> Option<bool> {
        let uuid = self.sleep_list_uuid(player)?;
        Some(self.server.lan.as_ref()?.sleep_ignored.contains(&uuid))
    }

    pub(in crate::game) fn player_sleep_timer(&self, player: PlayerRef) -> Option<i32> {
        Some(match self.player_handle_read(player)? {
            PlayerRead::Acting { .. } => self.common.sleep_timer,
            PlayerRead::Guest(guest) => guest.sleep_timer,
        })
    }

    pub(in crate::game) fn player_set_sleep_timer(
        &mut self,
        player: PlayerRef,
        ticks: i32,
    ) -> bool {
        if !self.common.is_logical_server() {
            return false;
        }
        match self.player_handle_read(player) {
            Some(PlayerRead::Acting { .. }) => {
                self.common.sleep_timer = ticks;
                true
            }
            Some(PlayerRead::Guest(guest)) => {
                let conn = guest.conn;
                self.server
                    .lan
                    .as_mut()
                    .and_then(|host| {
                        host.hub
                            .update_guest(conn, |guest| guest.sleep_timer = ticks)
                    })
                    .is_some()
            }
            None => false,
        }
    }

    pub(in crate::game) fn player_text_filtering(&self, player: PlayerRef) -> Option<bool> {
        if let Some(guest) = self.online_player_record(player) {
            return Some(guest.info.text_filtering);
        }
        (self.native_acting_player() == Some(player)).then_some(false)
    }

    pub fn player_set_respawn_config(
        &mut self,
        player: PlayerRef,
        config: Option<PlayerRespawnConfig>,
    ) -> bool {
        let stored = match config {
            Some(config) => {
                let dimension = config.position.dimension().to_string();
                if !config.yaw.is_finite() || !Dim::exists(&dimension) {
                    return false;
                }
                let at = config.position.pos();
                Some(RespawnConfig::wrapped(
                    Dim::from_id(&dimension),
                    [at.x, at.y, at.z],
                    config.yaw,
                    config.forced,
                ))
            }
            None => None,
        };
        match self.player_handle_read(player) {
            Some(PlayerRead::Acting { .. }) => {
                let mut set = false;
                self.native_as_acting(|game| set = game.set_respawn_point(stored)) && set
            }
            Some(PlayerRead::Guest(guest))
                if !guest.dead
                    || self
                        .cloning_guest(guest.uuid)
                        .is_some_and(|original| original.conn == guest.conn) =>
            {
                let conn = guest.conn;
                self.as_guest(conn, |game| game.set_respawn_point(stored)) == Some(true)
            }
            _ => false,
        }
    }

    pub(in crate::game) fn player_latency(
        &self,
        player: PlayerRef,
    ) -> Result<i32, mod_api::PlayerWorldRefusal> {
        use mod_api::PlayerWorldRefusal as Refusal;
        if !self.common.is_logical_server() {
            return Err(Refusal::Client);
        }
        if let Some(guest) = self.online_player_record(player) {
            return Ok(guest.latency.load(Ordering::Relaxed));
        }
        (self.native_acting_player() == Some(player) || self.player_handle_read(player).is_some())
            .then_some(0)
            .ok_or(Refusal::Stale)
    }

    pub(in crate::game) fn player_send_ping(
        &mut self,
        player: PlayerRef,
        id: i32,
    ) -> Result<(), mod_api::PlayerPingRefusal> {
        use mod_api::PlayerPingRefusal as Refusal;
        let Some(guest) = self.online_player_record(player) else {
            return Err(
                if self.native_acting_player() == Some(player)
                    || self.player_handle_read(player).is_some()
                {
                    Refusal::NoConnection
                } else {
                    Refusal::Stale
                },
            );
        };
        if !guest.play_connection_open() {
            return Err(Refusal::NoConnection);
        }
        guest
            .sender
            .try_send_play_ping(id)
            .then_some(())
            .ok_or(Refusal::SendFailed)
    }

    pub(in crate::game) fn player_hidden(
        &self,
        player: PlayerRef,
    ) -> Result<Option<u8>, mod_api::PlayerWorldRefusal> {
        use mod_api::PlayerWorldRefusal as Refusal;
        if !self.common.is_logical_server() {
            return Err(Refusal::Client);
        }
        let hub = &self.server.lan.as_ref().ok_or(Refusal::Stale)?.hub;
        if self.common.native_lifecycle.epoch() != Some(player.entity().epoch()) {
            return Err(Refusal::Stale);
        }
        Ok(hub.hidden.get(&player.uuid()).copied())
    }

    pub(in crate::game) fn player_nickname(
        &self,
        player: PlayerRef,
    ) -> Result<Option<&str>, mod_api::PlayerWorldRefusal> {
        use mod_api::PlayerWorldRefusal as Refusal;
        if !self.common.is_logical_server() {
            return Err(Refusal::Client);
        }
        let uuid = self.message_player_uuid(player).ok_or(Refusal::Stale)?;
        let hub = &self.server.lan.as_ref().ok_or(Refusal::Stale)?.hub;
        if !hub.online_players().any(|g| g.uuid == uuid) {
            return Err(Refusal::Stale);
        }
        Ok(hub.nicknames.get(&uuid).map(String::as_str))
    }

    pub fn player_hide(&mut self, player: PlayerRef, level: Option<u8>) -> bool {
        if self.common.native_lifecycle.epoch() != Some(player.entity().epoch()) {
            return false;
        }
        let uuid = player.uuid();
        let Some(host) = &mut self.server.lan else {
            return false;
        };
        match level {
            Some(level) => host.hub.hidden.insert(uuid, level),
            None => host.hub.hidden.remove(&uuid),
        };
        true
    }

    pub(in crate::game) fn player_set_nickname(
        &mut self,
        player: PlayerRef,
        nickname: Option<&str>,
    ) -> bool {
        let Some(uuid) = self.message_player_uuid(player) else {
            return false;
        };
        let Some(host) = &mut self.server.lan else {
            return false;
        };
        if !host.hub.online_players().any(|g| g.uuid == uuid) {
            return false;
        }
        host.hub.set_nickname(uuid, nickname.map(str::to_owned));
        true
    }

    pub fn native_broadcast(&self, mut line: Text) {
        let mut cancel = false;
        self.native_broadcast_message(&mut line, &mut cancel);
        if cancel {
            return;
        }
        let plain = line.plain();
        if self.common.is_dedicated_server() {
            super::dedicated::log("INFO", &plain);
        } else {
            log::info!("{plain}");
        }
        let Some(host) = &self.server.lan else {
            return;
        };
        let packet = system_chat(&line);
        for g in host.hub.guests.iter().filter(|g| g.placed) {
            host.hub.send_to(g.conn, &packet);
        }
    }

    pub fn native_server_event(&self, name: &str, uuid: u128, message: &str) -> bool {
        if !self.common.is_dedicated_server() || message.trim().is_empty() {
            return false;
        }
        super::dedicated::events::record("announce", name, uuid, json!({ "message": message }));
        true
    }

    /// Resolves `entity` to a live experience orb in the active or any parked server level.
    pub(in crate::game) fn native_orb_ref(
        &self,
        entity: EntityRef,
    ) -> Result<&sunrise_sim::entities::ExperienceOrb, LevelReadError> {
        if !self.common.is_logical_server() {
            return Err(LevelReadError::Unavailable);
        }
        if self.common.native_lifecycle.epoch() != Some(entity.epoch())
            || self.server.held_out.contains(&entity.id())
        {
            return Err(LevelReadError::Stale);
        }
        let id = entity.id();
        let target = super::commands::Target::Orb(id);
        if let Some(orb) = self.common.orbs.iter().find(|o| o.id == id) {
            return if self.resident_source_target_visible(self.common.dim.key(), target) {
                Ok(orb)
            } else {
                Err(LevelReadError::Stale)
            };
        }
        for (dim, parked) in &self.server.levels.0.parked {
            if let Some(orb) = parked.orbs.iter().find(|o| o.id == id) {
                return if self.resident_source_target_visible(dim.key(), target) {
                    Ok(orb)
                } else {
                    Err(LevelReadError::Stale)
                };
            }
        }
        Err(LevelReadError::Stale)
    }

    /// Resolves `entity` to a mutable live experience orb in the active or any parked server level.
    fn native_orb_mut(
        &mut self,
        entity: EntityRef,
    ) -> Result<&mut sunrise_sim::entities::ExperienceOrb, ExperienceOrbWriteError> {
        if !self.common.is_logical_server() {
            return Err(ExperienceOrbWriteError::Unavailable);
        }
        if self.common.native_lifecycle.epoch() != Some(entity.epoch())
            || self.server.held_out.contains(&entity.id())
        {
            return Err(ExperienceOrbWriteError::Stale);
        }
        let id = entity.id();
        let target = super::commands::Target::Orb(id);
        if self.common.orbs.iter().any(|o| o.id == id) {
            if !self.resident_source_target_visible(self.common.dim.key(), target) {
                return Err(ExperienceOrbWriteError::Stale);
            }
            return self
                .common
                .orbs
                .iter_mut()
                .find(|o| o.id == id)
                .ok_or(ExperienceOrbWriteError::Stale);
        }
        let Some(dim) = self
            .server
            .levels
            .0
            .parked
            .iter()
            .find_map(|(&dim, parked)| parked.orbs.iter().any(|o| o.id == id).then_some(dim))
        else {
            return Err(ExperienceOrbWriteError::Stale);
        };
        if !self.resident_source_target_visible(dim.key(), target) {
            return Err(ExperienceOrbWriteError::Stale);
        }
        self.server
            .levels
            .0
            .parked
            .get_mut(&dim)
            .and_then(|parked| parked.orbs.iter_mut().find(|o| o.id == id))
            .ok_or(ExperienceOrbWriteError::Stale)
    }

    /// Narrows `entity` to an [`ExperienceOrbRef`] across active and parked server levels.
    pub fn experience_orb(
        &self,
        entity: EntityRef,
    ) -> Result<Option<ExperienceOrbRef>, LevelReadError> {
        match self.native_orb_ref(entity) {
            Ok(_) => Ok(Some(ExperienceOrbRef::new(entity))),
            Err(LevelReadError::Stale) => Ok(None),
            Err(err) => Err(err),
        }
    }

    /// Visits experience orbs in `level` (active or parked) whose bounding box meets `min`..`max`
    /// in level order without allocating a list.
    pub(in crate::game) fn level_visit_experience_orbs(
        &self,
        level: LevelRef,
        min: [f64; 3],
        max: [f64; 3],
        visit: &mut dyn FnMut(ExperienceOrbRef) -> core::ops::ControlFlow<()>,
    ) -> Result<core::ops::ControlFlow<()>, LevelReadError> {
        self.native_level_world_result(level)?;
        let (min, max) = ordered_box(min, max).ok_or(LevelReadError::Unavailable)?;
        let epoch = level.epoch();
        let dimension = level.dimension();
        let orbs = if dimension == self.common.dim.key() {
            &self.common.orbs
        } else {
            &self
                .server
                .levels
                .0
                .parked_of(level.dimension_id())
                .ok_or(LevelReadError::Stale)?
                .orbs
        };
        for o in orbs.iter() {
            if o.count == 0
                || self.server.held_out.contains(&o.id)
                || !self
                    .resident_source_target_visible(dimension, super::commands::Target::Orb(o.id))
                || !orb_meets([o.position.x, o.position.y, o.position.z], min, max)
            {
                continue;
            }
            if visit(ExperienceOrbRef::new(EntityRef::new(epoch, o.id))).is_break() {
                return Ok(core::ops::ControlFlow::Break(()));
            }
        }
        Ok(core::ops::ControlFlow::Continue(()))
    }

    /// Reads `ExperienceOrb.getValue()` (`DATA_VALUE`, full `i32`).
    pub(in crate::game) fn experience_orb_value(
        &self,
        orb: ExperienceOrbRef,
    ) -> Result<i32, LevelReadError> {
        self.native_orb_ref(orb.entity()).map(|o| o.value)
    }

    /// Reads `ExperienceOrb.count`.
    pub(in crate::game) fn experience_orb_count(
        &self,
        orb: ExperienceOrbRef,
    ) -> Result<u32, LevelReadError> {
        self.native_orb_ref(orb.entity()).map(|o| o.count)
    }

    /// Reads `ExperienceOrb.age` (full signed `i32`).
    pub(in crate::game) fn experience_orb_age(
        &self,
        orb: ExperienceOrbRef,
    ) -> Result<i32, LevelReadError> {
        self.native_orb_ref(orb.entity()).map(|o| o.age)
    }

    /// Writes `ExperienceOrb.DATA_VALUE` (full `i32`; save truncates to `(short)`).
    pub(in crate::game) fn experience_orb_set_value(
        &mut self,
        orb: ExperienceOrbRef,
        value: i32,
    ) -> Result<(), ExperienceOrbWriteError> {
        self.native_orb_mut(orb.entity())?.value = value;
        Ok(())
    }

    /// Writes `ExperienceOrb.count`, rejecting `0` and values above `i32::MAX` (`ExtraCodecs.POSITIVE_INT`).
    pub(in crate::game) fn experience_orb_set_count(
        &mut self,
        orb: ExperienceOrbRef,
        count: u32,
    ) -> Result<(), ExperienceOrbWriteError> {
        let orb = self.native_orb_mut(orb.entity())?;
        if count == 0 || count > sunrise_sim::orb_limits::MAX_COUNT {
            return Err(ExperienceOrbWriteError::InvalidCount);
        }
        orb.count = count;
        Ok(())
    }

    /// Writes `ExperienceOrb.age` (full signed `i32`; save truncates to `(short)`).
    pub(in crate::game) fn experience_orb_set_age(
        &mut self,
        orb: ExperienceOrbRef,
        age: i32,
    ) -> Result<(), ExperienceOrbWriteError> {
        self.native_orb_mut(orb.entity())?.age = age;
        Ok(())
    }

    pub(in crate::game) fn native_player_holder(&self, uuid: u128) -> Option<&Guest> {
        self.cloning_guest(uuid).or_else(|| {
            self.server
                .lan
                .as_ref()?
                .hub
                .holder_guest(uuid, self.common.dim.id())
        })
    }

    pub fn native_live_acting_body(&self) -> Option<&PlayerMotion> {
        let body = self.common.player.as_ref()?;
        if body.health.health() <= 0.0 {
            return None;
        }
        Some(body)
    }

    pub(in crate::game) fn native_as_acting(&mut self, f: impl FnOnce(&mut Self)) -> bool {
        if self.native_live_acting_body().is_none() {
            return false;
        }
        f(self);
        true
    }

    pub fn resend_commands_filtered(&mut self) {
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        host.hub.refresh_permissions();
        self.send_placed_command_trees(true);
    }

    /// `Commands.sendCommands` sends only display data; `PlayerList.sendPlayerPermissionLevel`
    /// also sends the permission event before that data.
    pub(in crate::game) fn send_placed_command_trees(&mut self, send_permission: bool) {
        let Some(count) = self.server.lan.as_ref().map(|host| host.hub.guests.len()) else {
            return;
        };
        for i in 0..count {
            let Some(host) = self.server.lan.as_ref() else {
                return;
            };
            let guest = &host.hub.guests[i];
            if guest.placed && guest.participates_online() {
                let conn = guest.conn;
                if send_permission {
                    host.hub.send_permission_level(i);
                }
                self.send_player_command_tree(conn);
            }
        }
    }

    pub(in crate::game) fn native_command_player_target(
        &self,
        target: super::commands::Target,
    ) -> Option<PlayerRef> {
        use super::commands::Target;
        match target {
            Target::Player => self.native_acting_player(),
            Target::Guest(conn) if self.server.lan_acting == Some(conn) => {
                self.native_acting_player()
            }
            // The selected guest's own identity, a fake player's too (`execute on origin` of
            // its projectile). `player_handle_read` keeps the checks: a real guest must be
            // online, not displaced (or the one being cloned) and still that entity; a fake
            // must be placed and not displaced.
            Target::Guest(conn) => {
                let guest = self.command_guest(conn)?;
                let player = mod_api::__private::game::player_ref(
                    mod_api::EntityRef::new(
                        self.common.native_lifecycle.epoch()?,
                        u32::try_from(guest.entity_id).ok()?,
                    ),
                    guest.uuid,
                );
                self.player_handle_read(player).map(|_| player)
            }
            _ => None,
        }
    }

    pub(in crate::game) fn native_command_entity_target(
        &mut self,
        target: super::commands::Target,
    ) -> Option<mod_api::EntityRef> {
        if let Some(player) = self.native_command_player_target(target) {
            return Some(player.entity());
        }
        let id = sunrise_gameplay_common::game::commands_common::target_id(target)?;
        self.in_target_level(target, |game| game.entity(id))
            .flatten()
    }

    /// Resolve argument kinds shared by server and client native commands.
    fn resolve_client_safe_argument(
        &self,
        ty: Option<&ArgType>,
        value: Arg,
        coords_position: impl FnOnce(&sunrise_command::args::Coordinates) -> [f64; 3],
        rotation: [f32; 2],
    ) -> Result<mod_api::CommandArgumentValue, sunrise_command::CommandError> {
        use super::commands::err;
        use mod_api::CommandArgumentValue as Value;
        Ok(match value {
            Arg::Str(text) if matches!(ty, Some(ArgType::Nbt)) => nbt_value(&text)?,
            Arg::Str(value) => Value::String(value),
            Arg::Int(value) => Value::Integer(value),
            Arg::Long(value) => Value::Long(value),
            Arg::Double(value) => Value::Number(value),
            Arg::Bool(value) => Value::Boolean(value),
            Arg::Coords(coordinates) => {
                positional_value(ty, &coordinates, coords_position(&coordinates), rotation)
            }
            Arg::Item(input) => Value::Item(
                self.command_item_stack(&input, 1)?
                    .try_into_api()
                    .expect("command item validation resolves registered items"),
            ),
            // `ResourceOrTagKeyArgument.Result`: `#` names a tag, else a resource.
            Arg::Resource(id) if matches!(ty, Some(ArgType::ResOrTagKey(_))) => {
                let (tag, name) = id
                    .strip_prefix('#')
                    .map_or((false, &*id), |name| (true, name));
                Value::ResourceOrTag {
                    tag,
                    id: mod_api::Identifier::parse(name).map_err(|_| err("command.failed", &[]))?,
                }
            }
            Arg::Resource(id) => {
                Value::Id(mod_api::Identifier::parse(&id).map_err(|_| err("command.failed", &[]))?)
            }
            Arg::Custom { kind, value } => custom_value(&kind, value),
            _ => unreachable!("the native tree contains only registered native argument kinds"),
        })
    }

    pub(in crate::game) fn resolve_native_command(
        &mut self,
        context: Context,
    ) -> Result<mod_api::CommandInvocation, sunrise_command::CommandError> {
        use super::commands::err;
        use mod_api::{CommandArgument, CommandArgumentValue as Value};
        let mut arguments = Vec::with_capacity(context.args.len());
        for argument in context.args {
            let ty = parsed_type(&self.common.ui.dispatcher.0, &context.nodes, argument.range);
            let value = match argument.value {
                value if matches!(ty, Some(ArgType::GameProfile)) => {
                    Value::GameProfiles(self.mod_game_profiles(&value)?)
                }
                Arg::Entity(selector) => {
                    let (single, players) = context
                        .nodes
                        .iter()
                        .find_map(|(node, range)| {
                            if *range != argument.range {
                                return None;
                            }
                            match &self.common.ui.dispatcher.0.nodes[*node].kind {
                                sunrise_command::NodeKind::Argument {
                                    ty: ArgType::Entity { single, players },
                                    ..
                                } => Some((*single, *players)),
                                _ => None,
                            }
                        })
                        .expect("parsed entity argument has its dispatcher node");
                    let targets = self.resolve_entity_argument(&selector, players)?;
                    if single && targets.len() > 1 {
                        return Err(err(
                            if players {
                                "argument.player.toomany"
                            } else {
                                "argument.entity.toomany"
                            },
                            &[],
                        ));
                    }
                    if players {
                        if single {
                            Value::Player(
                                self.native_command_player_target(targets[0])
                                    .ok_or_else(|| err("argument.entity.notfound.player", &[]))?,
                            )
                        } else {
                            let selected = targets
                                .into_iter()
                                .map(|target| {
                                    self.native_command_player_target(target)
                                        .ok_or_else(|| err("argument.entity.notfound.player", &[]))
                                })
                                .collect::<Result<Vec<_>, _>>()?;
                            Value::Players(selected)
                        }
                    } else if single {
                        Value::Entity(
                            self.native_command_entity_target(targets[0])
                                .ok_or_else(|| err("argument.entity.notfound.entity", &[]))?,
                        )
                    } else {
                        let selected: Vec<_> = targets
                            .into_iter()
                            .filter_map(|target| self.native_command_entity_target(target))
                            .collect();
                        if selected.is_empty() {
                            return Err(err("argument.entity.notfound.entity", &[]));
                        }
                        Value::Entities(selected)
                    }
                }
                Arg::Resource(id) if matches!(ty, Some(ArgType::Dimension)) => {
                    dimension_value(&id, |dimension| self.level(dimension))?
                }
                other => self.resolve_client_safe_argument(
                    ty,
                    other,
                    |coordinates| self.resolve_coords(coordinates),
                    self.source_rotation(),
                )?,
            };
            arguments.push(CommandArgument::new(argument.name, value));
        }
        Ok(self.native_invocation(context.literals, arguments))
    }

    pub(in crate::game) fn server_crafting_remainder_hook(
        &self,
        stack: &ItemStack,
        remainder: &mut Option<ItemStack>,
    ) {
        sunrise_gameplay_common::game::api_values::with_stack_view(Some(stack), |view| {
            if let Some(view) = view {
                self.native_crafting_remainder(view, remainder);
            }
        });
    }

    pub(in crate::game) fn server_world_table_items(
        &self,
    ) -> Option<&Arc<sunrise_items::items::Items>> {
        self.server
            .world_tables
            .0
            .as_ref()
            .map(|tables| &tables.items)
    }

    pub(in crate::game) fn server_lid_openers(&self) -> Option<&HashMap<(i32, i32, i32), i32>> {
        Some(&self.server.lid_openers)
    }

    pub(in crate::game) fn server_signal_capability_site(
        &self,
        at: mod_api::BlockSide,
    ) -> Option<mod_api::BlockCapabilitySite> {
        let [x, y, z] = at.pos;
        self.containers()
            .capability_site(at, self.common.world.block(x, y, z), false)
    }

    pub(in crate::game) fn signal_level_mobs(
        &self,
        level: LevelRef,
    ) -> Option<&[sunrise_sim::mobs::Mob]> {
        let dimension = level.dimension();
        if dimension == self.common.dim.key() {
            Some(&self.common.mobs)
        } else {
            self.server
                .levels
                .0
                .parked_mobs(Dim::from_id(level.dimension_id()))
        }
    }

    pub(in crate::game) fn server_signal_container_size(
        &self,
        world: &World,
        at: mod_api::BlockSide,
    ) -> Option<usize> {
        self.level_container_size(world, self.signal_level_mobs(at.level)?, at)
    }

    pub(in crate::game) fn server_signal_lend_container_stack(
        &self,
        world: &World,
        at: mod_api::BlockSide,
        slot: usize,
        read: &mut dyn for<'a> FnMut(mod_api::ItemStackView<'a>),
    ) {
        if let Some(mobs) = self.signal_level_mobs(at.level) {
            self.level_lend_container_stack(world, mobs, at, slot, read);
        }
    }

    pub(in crate::game) fn server_level_block_face_sturdy(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        face: mod_api::BlockFace,
        support: mod_api::SupportType,
    ) -> Option<bool> {
        self.level_block_face_sturdy(level, pos, face, support)
    }

    pub(in crate::game) fn server_signal_block_entity(
        &self,
        level: LevelRef,
        pos: [i32; 3],
    ) -> Option<mod_api::BlockEntityRef> {
        self.block_entity(level, pos)
    }

    pub(in crate::game) fn server_signal_block_entity_data(
        &self,
        entity: mod_api::BlockEntityRef,
    ) -> Option<mod_api::Compound> {
        self.block_entity_data(entity)
    }

    pub(in crate::game) fn server_level_is_raining_at(
        &self,
        level: LevelRef,
        pos: [i32; 3],
    ) -> Option<bool> {
        self.level_is_raining_at(level, pos)
    }
}

#[expect(dead_code, reason = "dedicated settings stay engine-private")]
impl crate::game::ServerGame<'_> {
    pub(in crate::game) fn native_online_mode(&self) -> Option<bool> {
        Some(online_mode_of(&self.dedicated_lan()?.shared.config))
    }

    pub(in crate::game) fn native_server_port(&self) -> Option<u16> {
        Some(self.dedicated_lan()?.port)
    }

    fn dedicated_lan(&self) -> Option<&sunrise_sim::lan::Lan> {
        self.common.options.dedicated.as_ref()?;
        Some(&self.server.lan.as_ref()?.hub.lan)
    }
}

impl crate::game::ServerGame<'_> {
    pub(in crate::game) fn cloning_guest(&self, uuid: u128) -> Option<&Guest> {
        let pending = self
            .common
            .player_state
            .cloning_players
            .iter()
            .rev()
            .find(|player| player.uuid == uuid)
            .filter(|player| player.entity.is_some())?;
        self.command_guest(pending.conn).filter(|_| {
            !sunrise_gameplay_common::game::Side::lan_guest_swaps(&self.server)
                .contains(&pending.conn)
        })
    }

    pub(in crate::game) fn player_handle_read(&self, player: PlayerRef) -> Option<PlayerRead<'_>> {
        (self.common.native_lifecycle.epoch()? == player.fields().entity.epoch()).then_some(())?;
        if sunrise_gameplay_common::game::Side::lan_acting_fake(&self.server)
            && sunrise_gameplay_common::game::Side::lan_acting_uuid(&self.server)
                == Some(player.fields().uuid)
            && sunrise_gameplay_common::game::Side::lan_acting_entity(&self.server)
                == Some(player.fields().entity.id())
        {
            return Some(PlayerRead::Acting {
                mode: Some(self.common.mode_id()),
                body: self.common.player.as_ref(),
                abilities: sunrise_gameplay_common::game::Side::acting_abilities(&self.server),
                last_death: self.common.level.last_death.as_ref(),
            });
        }
        if let Some((read, id)) = self.player_lookup(player.fields().uuid)
            && id == player.fields().entity.id()
        {
            return Some(read);
        }
        let guests = &self.server.lan.as_ref()?.hub.guests;
        let guest = guests
            .get(guests.identity_index(player.fields().uuid, player.fields().entity.id())?)?;
        if !guest.placed
            || !guest.is_fake()
            || sunrise_gameplay_common::game::Side::lan_guest_swaps(&self.server)
                .contains(&guest.conn)
        {
            return None;
        }
        Some(PlayerRead::Guest(guest))
    }

    pub(in crate::game) fn player_lookup(&self, uuid: u128) -> Option<(PlayerRead<'_>, u32)> {
        if sunrise_gameplay_common::game::Side::lan_acting_uuid(&self.server) == Some(uuid)
            && !sunrise_gameplay_common::game::Side::lan_acting_fake(&self.server)
        {
            return Some((
                PlayerRead::Acting {
                    mode: Some(self.common.mode_id()),
                    body: self.common.player.as_ref(),
                    abilities: sunrise_gameplay_common::game::Side::acting_abilities(&self.server),
                    last_death: self.common.level.last_death.as_ref(),
                },
                sunrise_gameplay_common::game::Side::lan_acting_entity(&self.server)?,
            ));
        }
        if let Some(guest) = self.cloning_guest(uuid).or_else(|| {
            self.server.lan.as_ref().and_then(|host| {
                host.hub.guests.simulation_iter().find(|guest| {
                    host.hub.online(guest)
                        && guest.uuid == uuid
                        && !sunrise_gameplay_common::game::Side::lan_guest_swaps(&self.server)
                            .contains(&guest.conn)
                })
            })
        }) {
            return Some((PlayerRead::Guest(guest), guest.entity_id as u32));
        }
        if sunrise_gameplay_common::game::Side::lan_acting_uuid(&self.server).is_none()
            && self.server.serving.is_some()
            && self
                .common
                .level
                .player_uuid
                .map(sunrise_sim::lan::playerdata::ints_uuid)
                == Some(uuid)
        {
            let body = self.common.player.as_ref()?;
            return Some((
                PlayerRead::Acting {
                    mode: Some(self.common.mode_id()),
                    body: Some(body),
                    abilities: sunrise_gameplay_common::game::Side::acting_abilities(&self.server),
                    last_death: self.common.level.last_death.as_ref(),
                },
                body.entity_id as u32,
            ));
        }
        None
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::native_block_entity_read::NativeBlockEntityReadGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;

use sunrise_gameplay_common::game::native_hooks::ModGameExt1 as _;

use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;

#[cfg(test)]
pub(in crate::game) mod fake_action_tests {
    use super::{ActionOutcome, PlayerAction};
    use crate::game::native_server::fake_action_tests::fake;
    use crate::game::native_server::fake_action_tests::health;
    use crate::game::native_server::fake_action_tests::zombie;
    use sunrise_gameplay_common::game::native_server::fake_action_tests::AttackWatcher;
    use sunrise_gameplay_common::game::native_server::fake_action_tests::ChargeWatcher;
    use sunrise_items::items::ItemStack;

    /// `attack_entity` runs at `Player.attack`'s start, before the blow and the
    /// swing's attack-strength reset (vanilla's `getAttackStrengthScale(0.5)`
    /// is read there too, and `onAttack` resets the ticker after it). The hook
    /// sees the target untouched, the blow uses the charge that was there when
    /// the hook ran, and a veto leaves both the target and the charge alone.
    /// The charge itself is read through the damage it deals: the read a mod
    /// gets in the hook is `PlayerRef::attack_strength_scale` (part of the same
    /// issue, landed separately).
    #[test]
    #[ignore = "daily"]
    fn attack_entity_runs_before_the_blow_resets_the_attack_charge() {
        use crate::game::native_entity_refs::tests::with_game;
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        };

        let seen = Arc::new(Mutex::new(Vec::new()));
        let cancel = Arc::new(AtomicBool::new(false));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "attack-watcher",
            Box::new(AttackWatcher {
                seen: seen.clone(),
                cancel: cancel.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            // Half a fist's cooldown (delay 5 ticks): the charge is (2 + 0.5) / 5 = 0.5.
            with_game(|game| {
                let (player, fake_conn, _) = fake(game, None, 0);
                // A placed fake participates in actions, not online-player tracking.
                // The packet attack gate must still reach the hook and damage below.
                let host = game.server.lan.as_ref().unwrap();
                let guest = host.hub.guest(fake_conn).unwrap();
                assert!(guest.is_fake());
                assert!(guest.placed);
                assert_eq!(guest.dimension, host.hub.config.dimension);
                assert!(!guest.participates_online());
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update_all(|g| g.yaw = -90.0);
                let near = zombie(game, 5000, [2.0, 64.0, 0.5]);
                let full = health(game, 5000);
                game.as_guest(fake_conn, |g| g.server.attack_ticks = 2)
                    .unwrap();
                assert_eq!(
                    game.player_perform_action(player, PlayerAction::Attack { target: near }),
                    Ok(ActionOutcome::Consumed)
                );
                assert_eq!(
                    *seen.lock().unwrap(),
                    vec![full],
                    "the hook runs before the blow lands"
                );
                // The hit then uses the charge that was there for the hook: 0.5 scales a fist's
                // 1 to 0.4, and a zombie's 2 armour takes `getDamageAfterAbsorb`'s share.
                let dealt = 0.4f32 * (1.0 - (2.0 - 0.4 / 2.0) / 25.0);
                assert!(
                    (full - health(game, 5000) - dealt).abs() < 1e-4,
                    "the blow uses the pre-reset charge"
                );
                assert_eq!(
                    game.as_guest(fake_conn, |g| g.server.attack_ticks).unwrap(),
                    0,
                    "the swing resets the charge after the hit"
                );
            });
            // A veto at the hook leaves the target unhurt and the charge where it was: neither
            // the hit nor the reset has run yet.
            with_game(|game| {
                let (player, fake_conn, _) = fake(game, None, 0);
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update_all(|g| g.yaw = -90.0);
                let near = zombie(game, 5001, [2.0, 64.0, 0.5]);
                let full = health(game, 5001);
                seen.lock().unwrap().clear();
                cancel.store(true, Ordering::Relaxed);
                game.as_guest(fake_conn, |g| g.server.attack_ticks = 2)
                    .unwrap();
                assert_eq!(
                    game.player_perform_action(player, PlayerAction::Attack { target: near }),
                    Ok(ActionOutcome::Passed)
                );
                assert_eq!(*seen.lock().unwrap(), vec![full]);
                assert_eq!(health(game, 5001), full, "a veto stops the blow");
                assert_eq!(
                    game.as_guest(fake_conn, |g| g.server.attack_ticks).unwrap(),
                    2,
                    "and the swing that would reset the charge never starts"
                );
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn attack_strength_rejects_non_finite_partial_without_restricting_finite_ticks() {
        use crate::game::native_entity_refs::tests::with_game;
        with_game(|game| {
            let (player, conn, _) = fake(game, None, 0);
            game.as_guest(conn, |g| g.server.attack_ticks = 2).unwrap();
            assert_eq!(
                game.player_current_item_attack_strength_delay(player),
                Some(5.0)
            );
            for partial in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                assert_eq!(game.player_attack_strength_scale(player, partial), None);
                game.as_guest(conn, |g| {
                    assert_eq!(g.attack_strength_scale_at(partial), None);
                    assert_eq!(g.player_attack_strength_scale(player, partial), None);
                })
                .unwrap();
            }
            for (partial, expected) in [(-1.0, 0.2), (0.5, 0.5), (2.0, 0.8)] {
                assert_eq!(
                    game.player_attack_strength_scale(player, partial),
                    Some(expected)
                );
                game.as_guest(conn, |g| {
                    assert_eq!(g.attack_strength_scale_at(partial), Some(expected));
                    assert_eq!(
                        g.player_attack_strength_scale(player, partial),
                        Some(expected)
                    );
                })
                .unwrap();
            }
        });
    }

    /// `PlayerRef::attack_strength_scale` inside `attack_entity` reads the
    /// charge the blow uses: `Player.attack` reads `getAttackStrengthScale`
    /// itself at its start, and the hook runs before that read and before the
    /// swing resets the ticker. After the blow the same read sees 0. A
    /// resting player reads its saved ticker and body too.
    #[test]
    #[ignore = "daily"]
    fn attack_strength_scale_reads_the_charge_at_the_hook_and_not_after_the_swing() {
        use crate::game::native_entity_refs::tests::with_game;
        use std::sync::{Arc, Mutex};

        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "charge-watcher",
            Box::new(ChargeWatcher(seen.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let (player, fake_conn, _) = fake(game, None, 0);
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update_all(|g| g.yaw = -90.0);
                let near = zombie(game, 5000, [2.0, 64.0, 0.5]);
                let full = health(game, 5000);
                // A fist's cooldown is 5 ticks: a full ticker reads 1.0, and half of one
                // ((2 + 0.5) / 5) reads 0.5.
                game.as_guest(fake_conn, |g| g.server.attack_ticks = 5)
                    .unwrap();
                assert_eq!(
                    player.attack_strength_scale(game, 0.5),
                    Some(1.0),
                    "a full ticker is a fully charged blow"
                );
                game.as_guest(fake_conn, |g| g.server.attack_ticks = 2)
                    .unwrap();
                assert_eq!(
                    player.attack_strength_scale(game, 0.5),
                    Some(0.5),
                    "a resting fake reads its saved ticker and body"
                );
                assert_eq!(
                    game.player_perform_action(player, PlayerAction::Attack { target: near }),
                    Ok(ActionOutcome::Consumed)
                );
                assert_eq!(*seen.lock().unwrap(), vec![Some(0.5)]);
                // The blow uses the charge the hook read: 0.5 scales a fist's 1 to 0.4, and a
                // zombie's 2 armour takes `getDamageAfterAbsorb`'s share.
                let dealt = 0.4f32 * (1.0 - (2.0 - 0.4 / 2.0) / 25.0);
                assert!((full - health(game, 5000) - dealt).abs() < 1e-4);
                assert_eq!(
                    player.attack_strength_scale(game, 0.5),
                    Some(0.1),
                    "the swing reset the ticker to 0 after the blow: (0 + 0.5) / 5"
                );
            });
        });
    }

    /// A scoped attack with `full_attack_charge` deals the sword's full damage, one without it
    /// only the empty charge's share, and the fake's own ticker comes back after each.
    #[test]
    #[ignore = "daily"]
    fn scoped_fake_attack_with_full_charge_deals_the_weapons_full_damage() {
        use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};
        use mod_api::PlayerActionSetup;
        if run_fresh_cases(
            "game::server_side::native_server_server::fake_action_tests::scoped_fake_attack_with_full_charge_deals_the_weapons_full_damage",
            "SUNRISE_SCOPED_ATTACK_CASE",
            &["attack"],
        ) {
            return;
        }
        with_game(|game| {
            let (player, conn, _) = fake(game, None, 0);
            game.as_guest(conn, |g| g.server.attack_ticks = 3).unwrap();
            let charged = zombie(game, 5000, [2.0, 64.0, 0.5]);
            // Out of a sweep's reach of the first.
            let uncharged = zombie(game, 5001, [-1.5, 64.0, 0.5]);
            let mut attack = |target, full_attack_charge| {
                let mut setup = PlayerActionSetup::new(
                    Some(sunrise_gameplay_common::game::api_values::test_snapshot(
                        &ItemStack::new("minecraft:diamond_sword", 1),
                    )),
                    [0.5, 64.0 + f64::from(1.62_f32), 0.5],
                    0.0,
                    0.0,
                    false,
                );
                setup.full_attack_charge = full_attack_charge;
                let result = player
                    .perform_action_scoped(game, PlayerAction::Attack { target }, setup)
                    .unwrap();
                assert_eq!(result.outcome, ActionOutcome::Consumed);
                assert_eq!(
                    game.as_guest(conn, |g| g.server.attack_ticks).unwrap(),
                    3,
                    "the fake's own ticker comes back"
                );
            };
            attack(charged, true);
            attack(uncharged, false);
            // Diamond sword: 7 damage and 4 + (-2.4f32) attack speed. Player.attack samples
            // the empty ticker at 0.5, then CombatRules applies the zombie's 2 armour.
            let delay = (1.0 / (4.0 + f64::from(-2.4f32)) * 20.0) as f32;
            let charge = 0.5f32 / delay;
            let after_hit = |damage: f32| {
                let armor = (2.0 - damage / 2.0).max(2.0 * 0.2);
                20.0 - damage * (1.0 - armor / 25.0)
            };
            assert_eq!(health(game, 5000), after_hit(7.0));
            assert_eq!(
                health(game, 5001),
                after_hit(7.0 * (0.2 + charge * charge * 0.8))
            );
        });
    }
}

#[cfg(test)]
#[path = "native_server/profile_tests.rs"]
mod profile_tests;
