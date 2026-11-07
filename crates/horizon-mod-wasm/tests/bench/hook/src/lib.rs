//! One chat hook doing the work its message names, for `tests/bench.rs`. The test runs this
//! same source natively and as `mod.wasm` and compares the two.
//!
//! - `tick` (the server tick hook, with its small event): nothing; the cost of a call.
//! - `empty`: nothing, from the chat hook, whose event carries strings and messages.
//! - `text`: writes a new message, which is lowered back to the game.
//! - `imports N`: N calls into the game (`host.level`).
//! - `compute N`: plain mod logic on N values: fill, sort, hash-map counts, a checksum.
//!
//! It also listens at the game's tick point, doing nothing: the cost of a point filled by a wasm
//! mod (the `point` row; `tests/bench.rs` lists the point in the manifest it loads the mod with).
//!
//! It also registers the structure type `bench_mod:spin`, whose start is found after some
//! compute work, for the test of worldgen workers sharing the mod.
use std::collections::HashMap;
use std::sync::Arc;

use mod_api::{
    BoundingBox, Compound, Flow, FoundStructure, GenerationContext, Hooks, PieceContext, Registrar,
    RegistrationContext, ServerChatEvent, ServerHost, ServerTickEvent, StructureError,
    StructurePiece, StructureType, Text,
};

#[derive(Default)]
pub struct Mod;

pub fn tick(_host: mod_api::HostSide<'_>, _args: &mut mod_points::TickArgs) {}

#[cfg(target_arch = "wasm32")]
include!(concat!(env!("OUT_DIR"), "/guest_injects.rs"));

#[mod_api::horizon_mod]
impl Hooks for Mod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        r.register_structure_type("spin", Spin);
        let kind = r.block_entity_type("counter").blocks(&[]).build();
        r.block_entity_part(&kind, Counter).unwrap();
    }

    fn server_tick(&self, _host: &mut dyn ServerHost, _event: &mut ServerTickEvent<'_>) {}

    fn server_chat(&self, host: &mut dyn ServerHost, event: &mut ServerChatEvent<'_>) -> Flow {
        let (work, n) = event
            .raw_text
            .split_once(' ')
            .unwrap_or((event.raw_text, "0"));
        let n: u32 = n.parse().unwrap_or(0);
        match work {
            "text" => *event.message = Text::literal("a message the mod wrote"),
            "imports" => {
                let found = (0..n)
                    .filter(|_| host.level("minecraft:overworld").is_some())
                    .count();
                *event.message = Text::literal(found.to_string());
            }
            "entities" => {
                let found = (0..n).filter(|i| host.entity(*i).is_some()).count();
                *event.message = Text::literal(found.to_string());
            }
            "tickstats" => {
                let found = (0..n).filter(|_| host.tick_stats().0 >= 0.0).count();
                *event.message = Text::literal(found.to_string());
            }
            "compute" => *event.message = Text::literal(compute(n).to_string()),
            _ => {}
        }
        Flow::Continue
    }
}

struct Counter;

#[mod_api::horizon_part]
impl mod_api::BlockPart for Counter {}

#[mod_api::horizon_part]
impl mod_api::BlockEntityPart for Counter {
    fn block_entity_tick(
        &self,
        _: &mut dyn ServerHost,
        event: &mut mod_api::BlockEntityTickEvent<'_>,
    ) {
        event
            .entity
            .nbt
            .insert("counter".into(), mod_api::Tag::Int(17));
        event.entity.data = vec![23];
        event.entity.items.clear();
    }
}

/// A checksum over `n` pseudo-random values, sorted and counted: the same answer native and
/// wasm, so the test checks both did the work.
fn compute(n: u32) -> u64 {
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut values: Vec<u32> = (0..n)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 32) as u32
        })
        .collect();
    values.sort_unstable();
    let mut counts: HashMap<u32, u32> = HashMap::new();
    for value in &values {
        *counts.entry(value % 64).or_default() += 1;
    }
    let spread: u64 = counts.values().map(|&c| u64::from(c) * u64::from(c)).sum();
    values.iter().enumerate().fold(spread, |sum, (i, &v)| {
        sum.wrapping_mul(31).wrapping_add(u64::from(v) ^ i as u64)
    })
}

/// A structure type that works a while before it answers: one block at the chunk's corner.
struct Spin;

impl StructureType for Spin {
    fn find_generation_point(&self, context: &mut dyn GenerationContext) -> Option<FoundStructure> {
        let chunk = context.chunk();
        let y = 64 + (compute(100_000) % 8) as i32;
        let at = [chunk.x * 16, y, chunk.z * 16];
        Some(FoundStructure {
            position: at,
            pieces: vec![Arc::new(Block { at })],
        })
    }
    fn load_piece(&self, _: &Compound) -> Option<Arc<dyn StructurePiece>> {
        None
    }
}

#[derive(Debug)]
struct Block {
    at: [i32; 3],
}

impl StructurePiece for Block {
    fn bounding_box(&self) -> BoundingBox {
        BoundingBox::new(self.at, self.at)
    }
    fn post_process(&self, _: &mut dyn PieceContext) -> Result<(), StructureError> {
        Ok(())
    }
    fn add_additional_save_data(&self, _: &mut Compound) {}
}
