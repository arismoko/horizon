//! Portable game data shared by client and server, below commands and gameplay: chat text and
//! translations, SNBT and NBT paths, text components, the scoreboard, boss
//! bars, game rules, block entity NBT tables, and the registry, particle and
//! sound source tables commands name, and the `pack.mcmeta` filter regex.
//! sunrise-client re-exports each module under its old path
//! (`sunrise_client::text`, ...).
#![forbid(unsafe_code)]
pub mod block_entity_nbt;
pub mod block_entity_types;
pub mod bossbar;
pub mod component;
pub mod core_pack;
pub mod fastutil_order;
pub mod gamerules;
pub mod identifier;
pub mod json_nbt;
pub mod key_display;
pub mod key_mappings;
pub mod key_names;
pub mod lang;
pub mod move_input;
pub use sunrise_world::nbt_path;
pub mod nbt_value;
pub use sunrise_world::particle_type;
pub mod pattern;
pub mod potions;
pub mod registries;
pub mod scoreboard;
pub mod snbt;
pub mod sound_source;
pub mod suggestion;
pub mod text;
pub mod uuid;
pub mod who;
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
