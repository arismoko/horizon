//! Entity command execution shared by the bool and feedback-returning mod API methods.
use super::commands::execute;
use sunrise_game_data::text::Text;
use sunrise_gameplay_common::game::native_server::system_chat;

impl crate::game::ServerGame<'_> {
    /// Runs the command `line` (a leading slash allowed) with the entity as
    /// its source, as vanilla runs a portal's or a sign's command:
    /// `ServerPlayer.createCommandSourceStack` (another entity:
    /// `Entity.createCommandSourceStackForNameResolution`), `.withPermission(level)`,
    /// then `Commands.performPrefixedCommand`. `level` is taken as given, above
    /// or below the entity's own. `@s` is the entity, and the command runs from
    /// its position and rotation in its level. A player is answered as for their
    /// own command unless `suppress_output` silences the source
    /// (`CommandSourceStack.withSuppressedOutput`); another entity's source is
    /// always silent (`CommandSource.NULL`). Whether the command ran: false for a line that
    /// fails to parse or to run, and, with nothing run, for a stale or removed
    /// entity, an entity outside the level in place (a player in another level
    /// included: that level is parked while the hook runs), and on a client.
    /// During a guest's turn, displaced outer players are excluded, as in
    /// `native_run_command`.
    pub(in crate::game) fn entity_run_command(
        &mut self,
        entity: mod_api::EntityRef,
        line: &str,
        level: mod_api::PermissionLevel,
        suppress_output: bool,
    ) -> bool {
        self.run_entity_command(entity, line, level, suppress_output)
            .0
    }

    /// The entity command's outcome and source feedback, also delivered to the player when audible.
    pub(in crate::game) fn entity_run_command_with_output(
        &mut self,
        entity: mod_api::EntityRef,
        line: &str,
        level: mod_api::PermissionLevel,
        suppress_output: bool,
    ) -> mod_api::CommandResult {
        let (success, lines) = self.run_entity_command(entity, line, level, suppress_output);
        mod_api::CommandResult::new(success, lines.iter().map(Text::plain).collect())
    }

    fn run_entity_command(
        &mut self,
        entity: mod_api::EntityRef,
        line: &str,
        level: mod_api::PermissionLevel,
        suppress_output: bool,
    ) -> (bool, Vec<Text>) {
        use super::commands::Target;
        if self.server.lan.is_none() {
            return (false, Vec::new());
        }
        let Some(target) = self.native_entity_target(entity) else {
            return (false, Vec::new());
        };
        let line = line.strip_prefix('/').unwrap_or(line);
        let permission = level.level();
        match target {
            // The acting player, in place for this turn.
            Target::Player => match self.server.lan_acting_uuid {
                Some(uuid) => self.run_command_as_player(uuid, line, permission, suppress_output),
                None => (false, Vec::new()),
            },
            Target::Guest(conn) => {
                let Some(host) = &self.server.lan else {
                    return (false, Vec::new());
                };
                let Some(uuid) = host
                    .hub
                    .guest(conn)
                    .filter(|g| host.hub.here(g))
                    .map(|g| g.uuid)
                else {
                    return (false, Vec::new());
                };
                self.as_guest(conn, |game| {
                    game.run_command_as_player(uuid, line, permission, suppress_output)
                })
                .unwrap_or_default()
            }
            target => {
                let Some(pos) = self.target_position(target) else {
                    return (false, Vec::new());
                };
                // `CommandSource.NULL`: no feedback, no failure, no operator told.
                let source = execute::Source {
                    entity: Some(target),
                    pos,
                    rot: self.target_rotation(target),
                    eyes: false,
                    silent: true,
                    store: None,
                    dimension: self.common.dim,
                    origin_dimension: self.common.dim,
                    origin_player: false,
                    native_output: None,
                    level: None,
                    block: None,
                    cart: None,
                };
                self.run_command_from(source, line, permission)
            }
        }
    }
    /// `entity_run_command` for the player `uuid` in place: their own source,
    /// its lines sent to them and its `broadcastToAdmins` to the other operators.
    fn run_command_as_player(
        &mut self,
        uuid: u128,
        line: &str,
        permission: u8,
        suppress_output: bool,
    ) -> (bool, Vec<Text>) {
        use super::commands::Target;
        // `ServerPlayer.createCommandSourceStack`: the player's own position and
        // rotation, never an outer command's (`player_source` reads those).
        let Some(pos) = self.target_position(Target::Player) else {
            return (false, Vec::new());
        };
        let source = execute::Source {
            pos,
            rot: self.target_rotation(Target::Player),
            silent: suppress_output,
            ..self.player_source()
        };
        let outer_admins = std::mem::take(&mut self.server.admin_feedback);
        let (ok, lines) = self.run_command_from(source, line, permission);
        for line in &lines {
            self.native_send(uuid, system_chat(line));
        }
        self.send_admin_feedback();
        self.server.admin_feedback = outer_admins;
        (ok, lines)
    }
    /// `line` run from `source` at `permission`, with what it tells its source:
    /// the outer command's source, state and output are kept apart and put back.
    fn run_command_from(
        &mut self,
        source: execute::Source,
        line: &str,
        permission: u8,
    ) -> (bool, Vec<Text>) {
        let silent = source.silent;
        let outer = self.server.command_source.replace(source);
        let filter = std::mem::replace(&mut self.server.native_command_filter, true);
        let console = self.server.console_broadcast.take();
        // A silent source has no recipient for command feedback. Do not capture
        // explicit broadcasts such as /say; they go straight to their recipients.
        let capture = std::mem::replace(
            &mut self.server.chat_capture,
            if silent { None } else { Some(Vec::new()) },
        );
        let outer_exec = std::mem::take(&mut self.server.exec.0);
        let outer_results = self.server.cmds.0.results.take();
        let level = std::mem::replace(&mut self.common.ui.registries.permission, permission);
        let ok = self.run_command(line);
        self.common.ui.registries.permission = level;
        self.server.exec.0 = outer_exec;
        self.server.cmds.0.results = outer_results;
        let lines = std::mem::replace(&mut self.server.chat_capture, capture).unwrap_or_default();
        self.server.console_broadcast = console;
        self.server.native_command_filter = filter;
        self.server.command_source = outer;
        (ok, lines)
    }
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;
