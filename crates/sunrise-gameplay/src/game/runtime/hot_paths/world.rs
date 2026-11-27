//! The benchmark's level: a stone floor at y 63 over a square of otherwise empty chunks.
use crate::game::ServerGame;
use std::sync::Arc;
use sunrise_game_data::Result;
use sunrise_level::chunk::Chunk;
use sunrise_world::level::Ticks;

/// The floor's height; entities stand at `FLOOR + 1`.
pub(super) const FLOOR: i32 = 63;

impl ServerGame<'_> {
    /// Replace the level with a fresh one seeded `seed`: the chunks within `radius` of the
    /// origin, plains, lit by the sky, with the floor, and no entity or block entity.
    pub(super) fn hot_level(&mut self, seed: i64, radius: i32) -> Result<()> {
        self.common.world =
            sunrise_gameplay_common::game::world_for_mods(self.common.res.facts().air, seed);
        let plains = u32::from(world_biomes!(self).id("minecraft:plains"));
        for cx in -radius..=radius {
            for cz in -radius..=radius {
                let mut chunk = Chunk::empty(cx, cz);
                for section in &mut chunk.sections {
                    let section = Arc::make_mut(section);
                    section.biomes = sunrise_core::paletted::PalettedContainer::new(
                        sunrise_level::chunk::BIOME_STRATEGY,
                        plains,
                    );
                    section.light = sunrise_level::chunk::Light::Uniform(15 << 4);
                }
                self.common.world.chunks.insert((cx, cz), Arc::new(chunk));
            }
        }
        self.common.block_ticks = Ticks::new(0);
        self.server.tick_chunks.clear();
        self.sync_tick_chunks();
        self.common.mobs.clear();
        self.common.dropped.clear();
        self.common.orbs.clear();
        self.server.hoppers.clear();
        self.common.level_events.clear();
        self.common.ui.rules = Default::default();
        self.common.dim = super::super::Dim::Overworld;
        self.common.world.shape = self.common.dim.shape();
        self.common.ui.difficulty = 2;
        self.common.level.seed = seed;
        self.server.rand_sequences = Default::default();
        self.common.world.game_time = 0;
        self.common.world.day_time = 6000;
        for cx in -radius..=radius {
            for cz in -radius..=radius {
                let (x, z) = (cx * 16, cz * 16);
                self.hot_command(&format!(
                    "fill {x} {FLOOR} {z} {} {FLOOR} {} minecraft:stone",
                    x + 15,
                    z + 15
                ))?;
            }
        }
        // A server ticks only the chunks someone holds: no player is here, so they are forced.
        // The level before this one may have left its own forced (the command fails with none).
        self.run_command_quietly("forceload remove all");
        // Set the tickets without the live command's load barrier. This fixture supplies its
        // own columns and leaves streaming paused; the outer ring has no complete 3x3.
        for cx in -radius..=radius {
            for cz in -radius..=radius {
                self.server.streaming.set_forced(cx, cz, true);
            }
        }
        for rule in ["spawn_mobs", "advance_time", "advance_weather"] {
            self.hot_command(&format!("gamerule {rule} false"))?;
        }
        Ok(())
    }

    /// A setup command that must succeed; its failure message is the error.
    pub(super) fn hot_command(&mut self, command: &str) -> Result<()> {
        let outer = self.server.chat_capture.replace(Vec::new());
        let ran = self.run_command(command);
        let said = std::mem::replace(&mut self.server.chat_capture, outer).unwrap_or_default();
        if ran {
            return Ok(());
        }
        let said: Vec<String> = said.iter().map(|text| text.plain()).collect();
        Err(format!(
            "the benchmark's setup command failed: {command}: {}",
            said.join("; ")
        )
        .into())
    }
}
