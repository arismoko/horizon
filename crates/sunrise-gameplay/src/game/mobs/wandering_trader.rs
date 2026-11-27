//! `WanderingTraderSpawner`: every 1200 ticks the spawn delay counts down;
//! when it runs out a trader may appear near the player (or the nearest
//! bell), bringing two llamas on leads, and wanders off after two days.
use super::Dim;
use crate::game::ServerGame;
use sunrise_gameplay::wandering_trader::{TraderLevel, roll};
use sunrise_sim::save::{read_dat, write_dat};

impl crate::game::ServerGame<'_> {
    fn trader_path(&self) -> std::path::PathBuf {
        self.server
            .world_root
            .join("data")
            .join("minecraft")
            .join("wandering_trader.dat")
    }
    fn load_trader_data(&mut self) {
        if self.server.trader.is_loaded() {
            return;
        }
        let tag = read_dat(&self.trader_path());
        self.server
            .trader
            .load(tag.as_ref().and_then(|t| t.get("data")));
    }
    pub(in crate::game::server_side) fn save_wandering_trader(&mut self) {
        let Some(tag) = self.server.trader.to_save() else {
            return;
        };
        if write_dat(&self.trader_path(), tag).is_ok() {
            self.server.trader.saved();
        }
    }
    /// `WanderingTraderSpawner.tick`.
    pub(in crate::game::server_side) fn tick_wandering_trader(&mut self) {
        if self.common.dim != Dim::Overworld
            || !self
                .common
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::SPAWN_WANDERING_TRADERS)
        {
            return;
        }
        self.load_trader_data();
        let Some(chance) = self.server.trader.due() else {
            return;
        };
        if roll(self, chance) {
            self.server.trader.spawned();
        }
    }
}
impl TraderLevel for ServerGame<'_> {
    fn motion_top(&self, x: i32, z: i32) -> i32 {
        sunrise_gameplay_common::game::CommonGame::motion_top(self, x, z)
    }
    fn chunk_biome_has(&self, tag: &str, pos: [i32; 3]) -> bool {
        self.common
            .world
            .chunks
            .get(&(pos[0] >> 4, pos[2] >> 4))
            .map(|c| {
                self.common
                    .res
                    .biomes()
                    .get(c.biome(pos[0], pos[1], pos[2]))
                    .name
                    .clone()
            })
            .is_some_and(|b| self.common.res.mob_data().biome_has(tag, &b))
    }
}
