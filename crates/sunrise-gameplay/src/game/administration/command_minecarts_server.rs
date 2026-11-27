//! Server-side `MinecartCommandBlock` execution, synced output, and edit packet handling.
use super::{commands::execute::Source, commands_common::Target};
use sunrise_game_data::text::Text;
use sunrise_protocol::packets::play;

impl crate::game::ServerGame<'_> {
    /// `MinecartCommandBase.onUpdated`: copy command and last output into synched data.
    pub(in crate::game::server_side) fn cart_command_updated(&mut self, id: u32) {
        self.common.edit_command_cart(id, |c| {
            c.synced_command.clone_from(&c.command.command);
            c.sync_command_output();
        });
    }

    /// `BaseCommandBlock.performCommand` for the cart: false when it already ran this game tick.
    pub fn perform_cart_command(&mut self, id: u32) -> bool {
        let game_time = self.common.world.game_time;
        let rules_work = self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::COMMAND_BLOCKS_WORK);
        let dimension = self.common.dim;
        let Some(prepared) = self.common.edit_command_cart(id, |c| {
            use std::ops::ControlFlow::{Break, Continue};
            if c.command.last_execution == game_time {
                return Break(false);
            }
            if c.command.command.eq_ignore_ascii_case("Searge") {
                c.command.last_output = Some(Text::literal("#itzlipofutzli"));
                c.command.success_count = 1;
                return Break(true);
            }
            c.command.success_count = 0;
            let command = c.command.command.clone();
            if !rules_work || command.is_empty() {
                return Continue(None);
            }
            c.command.last_output = None;
            // MinecartCommandBase.createCommandSourceStack: cart position/rotation at game master level.
            let source = Source {
                entity: Some(Target::Cart(id)),
                pos: [c.position.x, c.position.y, c.position.z],
                rot: [c.yaw, 0.0],
                eyes: false,
                silent: false,
                store: None,
                dimension,
                origin_dimension: dimension,
                origin_player: false,
                native_output: None,
                level: None,
                block: None,
                cart: Some(id),
            };
            Continue(Some((command, source)))
        }) else {
            return false;
        };
        let command = match prepared {
            std::ops::ControlFlow::Break(done) => return done,
            std::ops::ControlFlow::Continue(command) => command,
        };
        if let Some((command, source)) = command {
            let successes = self.run_as_game_master(&command, source);
            self.common
                .edit_command_cart(id, |c| c.command.success_count = successes);
        }
        self.common.edit_command_cart(id, |c| {
            c.command.last_execution = if c.command.update_last_execution {
                game_time
            } else {
                -1
            };
        });
        true
    }

    /// `CloseableCommandBlockSource.sendSystemMessage` for the cart.
    pub(in crate::game::server_side) fn cart_command_output(
        &mut self,
        id: u32,
        format: impl FnOnce(Text) -> Text,
    ) {
        let updated = self.common.edit_command_cart(id, |c| {
            if !c.command.track_output {
                return false;
            }
            let time = chrono::Local::now().format("[%H:%M:%S] ").to_string();
            c.command.last_output = Some(format(Text::literal(time)));
            true
        });
        if updated == Some(true) {
            self.cart_command_updated(id);
        }
    }

    /// `CommandSourceStack.sendSuccess` for a command minecart.
    pub(in crate::game::server_side) fn cart_command_success(&mut self, id: u32, message: Text) {
        if self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::SEND_COMMAND_FEEDBACK)
        {
            self.cart_command_output(id, |timestamp| timestamp.append(message));
        }
    }

    /// `ServerGamePacketListenerImpl.handleSetCommandMinecart` for a guest.
    pub(in crate::game::server_side) fn lan_set_command_minecart(
        &mut self,
        conn: u32,
        p: play::SetCommandMinecart,
    ) {
        let Some(g) = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
        else {
            return;
        };
        let allowed = g.creative() && g.permission >= 2;
        self.as_guest_messaged(conn, |game| {
            if allowed {
                game.apply_cart_command(p.entity as u32, p.command, p.track_output);
            } else {
                game.server_system_message(Text::translate("advMode.notAllowed", &[]));
            }
        });
    }

    /// What `handleSetCommandMinecart` does once the player may edit the cart.
    pub(in crate::game::server_side) fn apply_cart_command(
        &mut self,
        id: u32,
        command: String,
        track_output: bool,
    ) {
        let enabled = self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::COMMAND_BLOCKS_WORK);
        if self
            .common
            .edit_command_cart(id, |c| {
                c.command.set_command(&command);
                c.command.track_output = track_output;
                if !track_output {
                    c.command.last_output = None;
                }
            })
            .is_none()
        {
            return;
        }
        if enabled {
            self.cart_command_updated(id);
        }
        if !command.is_empty() {
            let key = if enabled {
                "advMode.setCommand.success"
            } else {
                "advMode.setCommand.disabled"
            };
            self.server_system_message(Text::translate(key, &[Text::literal(command)]));
        }
    }
}
