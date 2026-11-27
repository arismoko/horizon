//! Matched resident ticker workload: the same keyed Part uses a view or a retained handle.
use super::{Recorder, world::FLOOR};
use mod_api::BlockEntityRef;
use std::{cell::RefCell, sync::Arc};
use sunrise_core::nbt::Tag;
use sunrise_game_data::Result;
use sunrise_level::{
    chunk::Chunk,
    containers::{BlockEntity, ModBlockEntity, ModBlockEntityFields},
};

const MACHINES: usize = 128;
const TICKS: i32 = 1_000;
const WARMUP: i32 = 20;
const INITIAL_PROGRESS: i32 = 23;
const KIND: &str = "hot_paths_listener:progress";

thread_local! {
    static SUBJECTS: RefCell<(Vec<BlockEntityRef>, usize)> = const { RefCell::new((Vec::new(), 0)) };
}

/// The benchmark Part already knows the generation-bound handle to its next subject.
/// The view Part advances the same fixture counter so both arms pay the same setup cost.
#[doc(hidden)]
pub fn hot_paths_next_block_entity() -> BlockEntityRef {
    SUBJECTS.with_borrow_mut(|(subjects, next)| {
        let handle = subjects[*next % MACHINES];
        *next += 1;
        handle
    })
}

impl crate::game::ServerGame<'_> {
    pub(super) fn hot_block_entity_view(
        &mut self,
        recorder: &mut Recorder,
        mode: &str,
    ) -> Result<()> {
        let (write, transaction, listening) = match mode {
            "view_read" | "handle_read" => (false, false, true),
            "view_write" | "handle_write" => (true, false, true),
            "view_tx_read" | "handle_tx_read" => (false, true, true),
            "view_tx_write" | "handle_tx_write" => (true, true, true),
            "no_listener" | "empty" => (false, false, false),
            _ => return Err(format!("invalid block-entity benchmark mode: {mode}").into()),
        };
        let empty = mode == "empty";
        let name = format!("mod.block_entity_view_v2.{mode}");
        self.resolve_parts()?;
        if listening
            != mod_api::__private::block_entity_type_listens(mod_api::HookId::BlockEntityTick, KIND)
        {
            return Err(format!("{name}: wrong listener setup for {KIND}").into());
        }
        self.common.world =
            sunrise_gameplay_common::game::world_for_mods(self.common.res.facts().air, 7);
        self.common
            .world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let block = if listening {
            let block = self
                .common
                .pack
                .assets
                .registry
                .default_state(KIND)
                .ok_or_else(|| format!("{name}: registered benchmark block {KIND} is missing"))?;
            let block_name = &self.common.res.facts().get(block).name;
            if sunrise_game_data::block_entity_types::valid(block_name, KIND) != Some(true) {
                return Err(format!(
                    "{name}: benchmark block {block_name} is not valid for block entity type {KIND}"
                )
                .into());
            }
            block
        } else {
            self.common
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .ok_or_else(|| format!("{name}: baseline block minecraft:stone is missing"))?
        };
        let positions: Vec<_> = (0..MACHINES as i32)
            .map(|i| (i % 16, FLOOR + 1, i / 16))
            .collect();
        for &(x, y, z) in &positions {
            self.common.world.set(x, y, z, block)?;
            let fields = ModBlockEntityFields {
                data: vec![INITIAL_PROGRESS, 7200],
                nbt: [
                    ("recipe".into(), Tag::String("test:mix".into())),
                    ("batch".into(), Tag::Int(64)),
                ]
                .into(),
                ..Default::default()
            };
            self.common.world.block_entities.insert(
                (x, y, z),
                BlockEntity::Mod(Box::new(ModBlockEntity::new(KIND.into(), fields))),
            );
        }
        if listening {
            let level = self
                .level(self.common.dim.id())
                .ok_or("benchmark level is not open")?;
            let handles = positions
                .iter()
                .map(|&(x, y, z)| {
                    self.block_entity(level, [x, y, z])
                        .ok_or("benchmark subject did not become resident")
                })
                .collect::<std::result::Result<_, _>>()?;
            SUBJECTS.set((handles, 0));
        }
        let pass = |game: &mut Self| {
            for &pos in &positions {
                if empty {
                    std::hint::black_box(pos);
                } else {
                    game.tick_mod_block_entity(pos);
                }
            }
        };
        let tick = |game: &mut Self, count| {
            for _ in 0..count {
                if transaction {
                    // One ambient frame per pass, not per row or listener. Every row's first
                    // write in a frame pays for the same before-image in both modes.
                    game.storage_scope(|game, _| {
                        pass(game);
                        ((), true)
                    });
                } else {
                    pass(game);
                }
            }
        };
        tick(self, WARMUP);
        let expected = INITIAL_PROGRESS + if write { WARMUP + TICKS } else { 0 };
        recorder.measure_checked(
            &name,
            MACHINES as u64 * TICKS as u64,
            || {
                tick(self, TICKS);
                self
            },
            |game| {
                if listening
                    && SUBJECTS.with_borrow(|s| s.1) != MACHINES * (TICKS + WARMUP) as usize
                {
                    return Err(format!("{name}: wrong number of ticker calls").into());
                }
                let mut sum = 0i32;
                for pos in &positions {
                    let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get(pos) else {
                        return Err(format!("{name}: subject at {pos:?} was removed").into());
                    };
                    if entity.data != [expected, 7200] {
                        return Err(format!(
                            "{name}: wrong progress at {pos:?} (expected {expected})"
                        )
                        .into());
                    }
                    sum += entity.data[0];
                }
                Ok(sum)
            },
        )?;
        SUBJECTS.set((Vec::new(), 0));
        Ok(())
    }
}

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::native_block_entity_read::NativeBlockEntityReadGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
