use super::{BaseTree, command_arguments, command_tree_for, register_command_nodes};
use mod_api::__private::{RetainedCallback, mod_index, split_command_registration};
use std::{collections::HashMap, sync::Arc};
use sunrise_gameplay_common::game::native_server::registered_commands::NativeCommands;
use sunrise_gameplay_common::game::native_server::registered_commands::RegisteredExecutor;
use sunrise_gameplay_common::game::native_server::registered_commands::without_reserved_roots;

impl crate::game::ServerGame<'_> {
    /// `Commands`' constructor with `RegisterCommandsEvent`: asks the mods for their commands and
    /// builds the session's tree, vanilla's for `features` with the mods' on top. Called before
    /// functions are admitted (world open and each reload), so both function checks see them.
    pub fn build_native_commands(&mut self, features: &[String]) {
        // A server hook: a client's tree is the one its server sends.
        if !self.common.is_logical_server() {
            return;
        }
        let mut registrations = Vec::new();
        self.native_register_commands(&mut registrations);
        let (usages, executors): (Vec<_>, Vec<_>) = registrations
            .into_iter()
            .filter_map(|registration| {
                let owner = registration.mod_id;
                let mod_index = mod_index(owner)?;
                let (command, call) = split_command_registration(registration.answer);
                let callback =
                    RetainedCallback::new(owner, mod_index, "command_executor", &command.usage);
                Some((command, RegisteredExecutor { callback, call }))
            })
            .unzip();
        let vanilla = sunrise_command::vanilla::session_dispatcher(
            &self.common.level.rule_registry,
            features,
        );
        let arguments = command_arguments();
        let mut slots = HashMap::new();
        let (tree, levels, problems) = command_tree_for(
            &usages,
            BaseTree::Merged(&vanilla),
            &arguments,
            false,
            Some(&mut slots),
        );
        for p in problems {
            log::warn!("Sunrise: a native mod's command was left out: {p}");
        }
        let state = &mut self.server.native_commands.0;
        state.registered = Some(NativeCommands {
            levels,
            tree,
            arguments,
            slots,
            executors,
        });
        state.display_revision = state.display_revision.wrapping_add(1);
        self.common.ui.dispatcher.0 = Arc::new(vanilla);
        self.common.ui.registries.selector_tokens =
            super::super::native_entity_selectors::selector_tokens();
        self.rebind_native_commands();
    }
    /// Rebind against the new node IDs. True if the display report changed too.
    pub(in crate::game::server_side) fn rebind_native_commands(&mut self) -> bool {
        let state = &mut self.server.native_commands.0;
        let Some(commands) = &state.registered else {
            return false;
        };
        let dispatcher = Arc::make_mut(&mut self.common.ui.dispatcher.0);
        // A replacement can add a session tool, e.g. pregen. Never overwrite its nodes
        // or advertise a mod usage behind the tool's name. Vanilla's roots take mod usages.
        let report = without_reserved_roots(&commands.tree, |name| {
            sunrise_command::vanilla::is_session_tool(name) && dispatcher.find(name).is_some()
        });
        register_command_nodes(
            dispatcher,
            &report,
            "native_mod_command",
            &commands.arguments,
            Some(&commands.slots),
        );
        let changed = state.report != report;
        state.report = report;
        changed
    }
    /// Gives the hub the session's mod commands when it lacks them; first asks the mods again
    /// when one asked (`native_refresh_commands`), and then sends every player their tree.
    pub fn register_native_commands(&mut self) {
        let refresh = std::mem::take(&mut self.server.native_commands.0.refresh);
        if self.server.lan.is_none() {
            return;
        }
        if refresh || self.server.native_commands.0.registered.is_none() {
            self.build_native_commands(&self.common.level.enabled_features.clone());
        }
        self.send_native_command_report(refresh);
    }
    /// Restore mod nodes (and rebind their executors) after vanilla replaces the tree.
    pub(in crate::game::server_side) fn restore_native_commands(&mut self) {
        if self.rebind_native_commands() {
            let revision = &mut self.server.native_commands.0.display_revision;
            *revision = revision.wrapping_add(1);
        }
    }
    /// Send accepted display data, without rebuilding usages or exposing executor slots.
    /// Forced when vanilla itself changes, even if the accepted mod report did not.
    pub(in crate::game::server_side) fn send_native_command_report(&mut self, force_resend: bool) {
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let state = &self.server.native_commands.0;
        if state.registered.is_none() {
            if force_resend {
                self.send_placed_command_trees(false);
            }
            return;
        }
        let revision = Some(state.display_revision);
        let replacing = host.native_commands_sent.is_some();
        if host.native_commands_sent != revision {
            // The hub merges a root vanilla has into vanilla's node (`CommandTree::packet_with`).
            let tree = state.report.clone();
            host.native_commands_sent = revision;
            host.hub
                .set_selector_tokens(self.common.ui.registries.selector_tokens.clone());
            let changed = host.hub.set_mod_commands(0, tree);
            if !force_resend && !replacing && !changed {
                return;
            }
        } else if !force_resend {
            return;
        }
        self.send_placed_command_trees(false);
    }
}
