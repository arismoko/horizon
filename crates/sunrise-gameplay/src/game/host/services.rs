use crate::game::{ServerGame, server_side::native_console::Output};
use core::ops::ControlFlow;
use mod_api::{__private::game::CallbackOwner, *};
use std::sync::mpsc::Sender;
use sunrise_server_files::{user_cache::UserCache, user_lists::NameAndId};
use sunrise_sim::lan::guest::Guest;

impl Commands for ServerGame<'_> {
    fn refresh_commands(&mut self) {
        CallbackOwner::call_host(|| self.native_refresh_commands())
    }
    fn feedback(
        &mut self,
        line: Text,
        broadcast_to_admins: bool,
    ) -> Result<(), mod_api::text::TextFormError> {
        let line = sunrise_sim::api_text::to_serialized_engine(&line)?;
        CallbackOwner::call_host(|| self.native_feedback(line, broadcast_to_admins));
        Ok(())
    }
    fn failure(&mut self, line: Text) -> Result<(), mod_api::text::TextFormError> {
        self.native_failure(sunrise_sim::api_text::to_serialized_engine(&line)?);
        Ok(())
    }
    fn run_command(&mut self, line: &str) -> bool {
        CallbackOwner::call_host(|| self.native_run_command(line))
    }
    fn run_command_with_output(&mut self, line: &str) -> mod_api::CommandResult {
        CallbackOwner::call_host(|| self.native_run_command_with_output(line))
    }
    fn run_command_with_feedback(
        &mut self,
        line: &str,
        output: Sender<Result<CommandFeedback, TextFormError>>,
    ) -> bool {
        CallbackOwner::call_host(|| {
            self.run_console_command(line, Some(Output::Stream(output)))
                .success
        })
    }
}
impl Levels for ServerGame<'_> {
    fn visit_levels(
        &self,
        visit: &mut dyn FnMut(LevelRef) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, ServerReadError> {
        // The caller's visitor runs inside its own callback scope.
        Ok(self.native_visit_levels(visit))
    }
}
impl Messages for ServerGame<'_> {
    fn broadcast(&self, line: Text) -> Result<(), mod_api::text::TextFormError> {
        let line = sunrise_sim::api_text::to_serialized_engine(&line)?;
        CallbackOwner::call_host(|| self.native_broadcast(line));
        Ok(())
    }
}
impl Players for ServerGame<'_> {
    fn server_players(&self) -> Vec<PlayerRef> {
        CallbackOwner::call_host(|| self.native_server_players())
    }
    fn player_by_name(&self, name: &str) -> Option<PlayerRef> {
        CallbackOwner::call_host(|| self.native_player_by_name(name))
    }

    fn profile_by_uuid(&self, uuid: u128) -> Option<GameProfile<'_>> {
        self.lookup_profile(|player| player.uuid == uuid, |cache| cache.find_by_id(uuid))
    }

    fn profile_by_name(&self, name: &str) -> Option<GameProfile<'_>> {
        self.lookup_profile(
            |player| player.name.eq_ignore_ascii_case(name),
            |cache| cache.find_by_name(name),
        )
    }
    fn profile_permission_level(&self, uuid: u128, name: &str) -> PermissionLevel {
        if let Some(lists) = &self.server.server_lists.0 {
            let lists = lists.lock().unwrap_or_else(|e| e.into_inner());
            return lists
                .ops
                .permission_level(uuid)
                .map_or(PermissionLevel::All, PermissionLevel::from_level);
        }
        let Some(host) = &self.server.lan else {
            return PermissionLevel::All;
        };
        PermissionLevel::from_level(i32::from(host.hub.default_permission(uuid, name)))
    }
    fn acting_player(&self) -> Option<PlayerRef> {
        CallbackOwner::call_host(|| self.native_acting_player())
    }
    fn singleplayer_profile(&self) -> Option<GameProfile<'_>> {
        self.native_singleplayer_profile()
    }
}
impl ServerGame<'_> {
    /// Borrow from the online records before consulting the offline cache.
    fn lookup_profile(
        &self,
        matches: impl Fn(&Guest) -> bool,
        cached: impl FnOnce(&UserCache) -> Option<&NameAndId>,
    ) -> Option<GameProfile<'_>> {
        if let Some(online) = self
            .server
            .lan
            .iter()
            .flat_map(|host| host.hub.online_players())
            .find(|player| matches(player))
        {
            return Some(mod_api::__private::game::game_profile(
                online.uuid,
                &online.name,
                &online.properties,
            ));
        }
        let cached = cached(self.server.user_cache.as_ref()?)?;
        Some(mod_api::__private::game::game_profile(
            cached.id,
            &cached.name,
            &[],
        ))
    }
}
impl Profiles for ServerGame<'_> {
    fn resolve_name(&self, name: &str) -> ProfileResolution {
        self.native_resolve_profile(sunrise_level::profile_lookup::NameOrId::Name(
            name.to_owned(),
        ))
    }
    fn resolve_id(&self, uuid: u128) -> ProfileResolution {
        self.native_resolve_profile(sunrise_level::profile_lookup::NameOrId::Id(uuid))
    }
}
impl RuntimeLevels for ServerGame<'_> {
    fn open(&mut self, id: &str, level: RuntimeLevel) -> Result<LevelRef, RuntimeLevelError> {
        CallbackOwner::call_host(|| self.runtime_level_open(id, level))
    }
    fn unload(&mut self, id: &str) -> Result<(), RuntimeLevelError> {
        CallbackOwner::call_host(|| self.runtime_level_unload(id))
    }
    fn delete(&mut self, id: &str) -> Result<(), RuntimeLevelError> {
        CallbackOwner::call_host(|| self.runtime_level_delete(id))
    }
    fn is_open(&mut self, id: &str) -> bool {
        CallbackOwner::call_host(|| self.runtime_level_is_open(id))
    }
}
