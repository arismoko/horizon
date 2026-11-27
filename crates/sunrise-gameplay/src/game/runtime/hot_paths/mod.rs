//! The hot-path benchmark (docs/hot-path-trends.md): with `SUNRISE_HOT_PATHS=OUTPUT` the game
//! loads its resources, runs fixed, seeded workloads through the code a playing game runs and
//! writes one JSON line per workload: its name, how many operations it ran, a checksum of what
//! they did and the wall time. Each measured region is bracketed by Callgrind requests
//! (`sunrise_threads::callgrind`), so the same run under Valgrind gives the instructions of each
//! region; setup is never counted. `SUNRISE_HOT_PATHS_ONLY=a,b` keeps the workloads whose names
//! start with one of the prefixes. In single-listener mode (`SUNRISE_HOT_PATHS_MODS=native`),
//! `sunrise-hot-paths-game` installs one entity tick listener. `fanout-N` instead installs N
//! separately registered native listeners for the fan-out workloads.
//!
//! A workload builds its own world on a server `Game`, as the component harnesses do. Nothing is
//! compared with vanilla here: the checksum only shows that two runs did the same work.
use crate::game::ServerGame;
use std::{
    ffi::CString,
    hash::{DefaultHasher, Hash, Hasher},
    io::Write as _,
    time::Instant,
};
use sunrise_game_data::Result;

mod chunks;
mod commands;
mod fanout;
mod mods;
mod ticks;
mod views;
mod world;

pub use fanout::{HotPathsMods, note_hot_paths_listener_call};
pub use views::hot_paths_next_block_entity;

/// Where the measured workloads report.
pub(in crate::game) struct Recorder {
    out: std::io::BufWriter<std::fs::File>,
    only: Vec<String>,
}

impl Recorder {
    /// Whether the workload `name` was asked for; a workload skips its setup otherwise.
    fn wants(&self, name: &str) -> bool {
        self.only.is_empty()
            || self
                .only
                .iter()
                .any(|prefix| name.starts_with(prefix.as_str()))
    }

    /// Run `work` as the measured region of `name`: `operations` is what its cost divides by,
    /// and what it returns is hashed into the line's checksum.
    fn measure<T: Hash>(
        &mut self,
        name: &str,
        operations: u64,
        work: impl FnOnce() -> T,
    ) -> Result<()> {
        self.measure_checked(name, operations, work, Ok)
    }

    /// Check and derive the checksum from the measured work after stopping the counters.
    fn measure_checked<T, C: Hash>(
        &mut self,
        name: &str,
        operations: u64,
        work: impl FnOnce() -> T,
        checksum: impl FnOnce(T) -> Result<C>,
    ) -> Result<()> {
        let label = CString::new(name)?;
        let alloc = sunrise_threads::alloc_count::ON.then(sunrise_threads::alloc_count::here);
        let started = Instant::now();
        sunrise_threads::callgrind::start();
        let done = work();
        sunrise_threads::callgrind::stop(&label);
        let wall = started.elapsed();
        let alloc = alloc.map(|before| {
            let after = sunrise_threads::alloc_count::here();
            (after.allocs - before.allocs, after.bytes - before.bytes)
        });
        let mut hash = DefaultHasher::new();
        checksum(done)?.hash(&mut hash);
        writeln!(
            self.out,
            "{}",
            serde_json::json!({
                "name": name,
                "operations": operations,
                "checksum": format!("{:016x}", hash.finish()),
                "wall_ns": wall.as_nanos() as u64,
                "allocations": alloc.map(|(calls, _)| calls),
                "allocated_bytes": alloc.map(|(_, bytes)| bytes),
            })
        )?;
        self.out.flush()?;
        Ok(())
    }
}

impl ServerGame<'_> {
    /// Run the workloads and write their lines to `output`.
    pub(in crate::game::server_side) fn hot_paths(&mut self, output: &str) -> Result<()> {
        assert!(
            !self.options.execution_role.is_client(),
            "the benchmark requires a server Game"
        );
        let mut recorder = Recorder {
            out: std::io::BufWriter::new(std::fs::File::create(output)?),
            only: std::env::var("SUNRISE_HOT_PATHS_ONLY")
                .map(|only| only.split(',').map(str::to_owned).collect())
                .unwrap_or_default(),
        };
        // No world opens here: the command tree's registries and the console's rights come
        // from the resources, as a world's opening sets them.
        self.build_registries();
        self.common.ui.registries.permission = 4;
        // The levels' handles, which a world's opening publishes to mods.
        self.reset_entity_residency();
        self.common
            .native_lifecycle
            .begin_open(Some(mod_api::Authority::DedicatedServer));
        if let Ok(mode) = std::env::var("SUNRISE_BLOCK_ENTITY_VIEW_BENCH") {
            return self.hot_block_entity_view(&mut recorder, &mode);
        }
        match HotPathsMods::from_env()? {
            HotPathsMods::None => {
                self.hot_ticks(&mut recorder)?;
                self.hot_commands(&mut recorder)?;
                self.hot_chunks(&mut recorder)?;
                self.hot_mods(&mut recorder, false)
            }
            HotPathsMods::Native => self.hot_mods(&mut recorder, true),
            HotPathsMods::Fanout(count) => self.hot_hook_fanout(&mut recorder, count),
        }
    }
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
