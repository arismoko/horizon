//! Horizon's initial simulation primitives, targeting the Minecraft Java Edition version in [`version`].
//!
//! This is not a playable game. Differential checks cover only the operations
//! listed in docs/rust-parity.md; the rest of vanilla remains unimplemented.
#![forbid(unsafe_code)]

pub mod arm_pose;
pub mod biome;
pub mod bounding_box;
pub mod carving;
pub mod color;
pub mod connection_kind;
pub mod density;
pub mod dimension;
pub mod equipment_slot;
pub mod fast_hash;
pub mod game_mode;
pub mod geometry;
pub mod identifier;
pub mod java_hash_map;
pub mod jmath;
pub mod nbt;
pub mod neighbour_update;
pub mod no_ai;
pub mod noise;
pub mod paletted;
pub mod position;
pub mod prepared_files;
pub mod random;
pub mod release;
pub mod storage;
pub mod text;
pub mod ticks;
pub mod trading;
pub mod use_animation;
pub mod version;

pub use version::VANILLA_VERSION;
