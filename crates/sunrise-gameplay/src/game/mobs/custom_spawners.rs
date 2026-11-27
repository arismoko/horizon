//! A level's custom spawners (`ServerLevel.customSpawners`) and their tick
//! (`ServerLevel.tickCustomSpawners`): vanilla's built-ins (for the Overworld; empty for other
//! dimensions, matching `MinecraftServer#createLevels`), minus what a mod's
//! `modify_custom_spawners` took out, plus the owned `CustomSpawner` objects it added.
use crate::game::ServerGame;
use mod_api::{__private::CustomSpawners, SpawnerKind};
use sunrise_gameplay_common::game::Dim;
use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

impl ServerGame<'_> {
    /// Build `dim`'s custom spawner list once per open logical-server level, firing
    /// `modify_custom_spawners` when the level opens (or lazily if ticked without
    /// `native_level_opened`). Does nothing on a logical client.
    pub fn init_custom_spawners(&mut self, dim: Dim) {
        if !self.common.is_logical_server() {
            return;
        }
        if self.common.creatures.custom_spawners.contains_key(&dim) {
            return;
        }
        let mut list = if dim == Dim::Overworld {
            CustomSpawners::vanilla()
        } else {
            CustomSpawners::empty()
        };
        if mod_api::__private::listened(mod_api::HookId::ModifyCustomSpawners) {
            let level = self.level_ref(self.lend_epoch(), dim.key());
            let mut editor = mod_api::CustomSpawnersEditor::new(&list);
            self.native_modify_custom_spawners(level, &mut editor);
            editor.commit(&mut list);
        }
        self.common.creatures.custom_spawners.insert(dim, list);
    }

    /// `ServerLevel.tickCustomSpawners`: every spawner in the list, in order, including
    /// `Phantom` before any mod-appended spawners. Skips logical clients, debug worlds
    /// (`ServerChunkCache.tickChunks`), and ticks where `spawn_mobs` is disabled. Passes
    /// the chunk source's stored `spawn_enemies` setting to Phantom and each registered
    /// `CustomSpawner`, which runs directly under its registering mod in a transaction.
    pub fn tick_custom_spawners(&mut self) {
        if !self.common.is_logical_server()
            || self.server.world_debug
            || !self
                .common
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::SPAWN_MOBS)
        {
            return;
        }
        let dim = self.common.dim;
        self.init_custom_spawners(dim);
        let spawn_enemies = self.server.chunk_spawn_enemies;
        let level = self.level_ref(self.lend_epoch(), dim.key());
        let mut at = 0;
        loop {
            let step = self
                .common
                .creatures
                .custom_spawners
                .get_mut(&dim)
                .and_then(|list| list.take_at(at));
            let Some(step) = step else {
                break;
            };
            match step {
                mod_api::__private::CustomSpawnerStep::Builtin(SpawnerKind::WanderingTrader) => {
                    self.tick_wandering_trader();
                }
                mod_api::__private::CustomSpawnerStep::Builtin(SpawnerKind::Patrol) => {
                    self.tick_patrol_spawner();
                }
                mod_api::__private::CustomSpawnerStep::Builtin(SpawnerKind::Cat) => {
                    self.tick_cat_spawner();
                }
                mod_api::__private::CustomSpawnerStep::Builtin(SpawnerKind::VillageSiege) => {
                    self.tick_village_siege();
                }
                mod_api::__private::CustomSpawnerStep::Builtin(SpawnerKind::Phantom) => {
                    self.tick_phantom_spawner(spawn_enemies);
                }
                mod_api::__private::CustomSpawnerStep::Mod(mut spawner) => {
                    let _ = self
                        .storage_transaction(|game, _| spawner.tick(game, level, spawn_enemies));
                    if let Some(list) = self.common.creatures.custom_spawners.get_mut(&dim) {
                        list.put_back(at, spawner);
                    }
                }
                _ => {}
            }
            at += 1;
        }
    }
}
