//! Per-recipient recipe selection on the server thread.
use crate::game::Game;

impl Game<'_> {
    pub(in crate::game) fn prepare_recipe_join(&mut self, conn: u32) {
        self.ask_recipe_for_conn(conn);
        self.native_filter_commands(Some(conn));
    }

    /// Ask once per online player before rebuilding their recipe payloads.
    /// No online players means no calls.
    pub(in crate::game) fn sync_recipe_content(&mut self) {
        let connections: Vec<_> = self
            .lan
            .as_ref()
            .map(|host| {
                host.hub
                    .guests
                    .iter()
                    .filter(|g| g.placed && g.participates_online())
                    .map(|g| g.conn)
                    .collect()
            })
            .unwrap_or_default();
        for conn in connections {
            self.ask_recipe_for_conn(conn);
        }
    }

    fn ask_for(&mut self, player: mod_api::PlayerRef) -> Vec<String> {
        let mut recipe_types = Vec::new();
        self.native_datapack_sync(mod_api::__private::DatapackSyncCall {
            player,
            recipe_types: &mut recipe_types,
        });
        recipe_types
    }

    fn ask_recipe_for_conn(&mut self, conn: u32) {
        let (Some(epoch), Some(host)) = (self.native_lifecycle.epoch(), self.lan.as_ref()) else {
            return;
        };
        let Some(g) = host.hub.guest(conn) else {
            return;
        };
        let player = mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(epoch, g.entity_id as u32),
            g.uuid,
        );
        let types = self.ask_for(player);
        let sources = if types.is_empty() {
            Vec::new()
        } else {
            horizon_items::recipe_sources::sources(&self.options.resources, &self.pack_files())
                .unwrap_or_else(|e| {
                    log::warn!("Horizon: the recipes for the client were not read: {e}");
                    Vec::new()
                })
        };
        if let Some(host) = self.lan.as_mut() {
            host.hub.update_guest(conn, |g| {
                g.recipe_sync.replace(g.brand == "horizon", types, &sources);
            });
        }
    }
}
