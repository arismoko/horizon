use crate::game::creatures::NativeSound;
use mod_api::{BlockPos, PlayerRef};
use sunrise_sim::lan::server_effects::ServerQueuedParticle;
use sunrise_world::{fluid::FluidKind, level::states::BlockId};

pub enum PendingWrite {
    BlockTick(super::Dim, BlockPos, BlockId, i32),
    FluidTick(super::Dim, BlockPos, FluidKind, i32),
    /// A level or entity-bound sound. `seed` is the caller's (`ServerLevel.playSeededSound`), or
    /// `None` to draw the level's next sound seed when the sound is published (`Level.playSound`).
    Sound {
        sound: NativeSound,
        seed: Option<i64>,
    },
    Particles(ServerQueuedParticle),
    Event {
        level: super::Dim,
        id: i32,
        pos: [i32; 3],
        data: i32,
    },
    GlobalEvent {
        level: super::Dim,
        id: i32,
        pos: [i32; 3],
        data: i32,
    },
    EntityEvent {
        level: super::Dim,
        entity_id: i32,
        event: i8,
        at: [f64; 3],
    },
    PlayerSound {
        level: super::Dim,
        player: PlayerRef,
        sound: String,
        at: [f64; 3],
        source: usize,
        volume: f32,
        pitch: f32,
    },
    PlayerStopSound {
        level: super::Dim,
        player: PlayerRef,
        sound: Option<String>,
        source: Option<usize>,
    },
}
