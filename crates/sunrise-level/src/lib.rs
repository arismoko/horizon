//! Portable level data, chunks, lighting, block entities, biomes and engine utilities.
//! Zero desktop, GPU, window, audio, gamepad, clipboard or HTTP dependencies.
#![forbid(unsafe_code)]

#[cfg(any(test, feature = "test-support"))]
pub mod test_log;

pub use sunrise_assets::resource_source;
pub use sunrise_assets::zip;
pub use sunrise_command as command;
pub use sunrise_game_data::Result;
pub use sunrise_game_data::block_entity_nbt;
pub use sunrise_game_data::block_entity_types;
pub use sunrise_game_data::bossbar;
pub use sunrise_game_data::component;
pub use sunrise_game_data::fastutil_order;
pub use sunrise_game_data::gamerules;
pub use sunrise_game_data::lang;
pub use sunrise_game_data::nbt_path;
pub use sunrise_game_data::scoreboard;
pub use sunrise_game_data::snbt;
pub use sunrise_game_data::text;
pub use sunrise_items::blocks;
pub use sunrise_items::bundle;
pub use sunrise_items::cooldowns;
pub use sunrise_items::enchant;
pub use sunrise_items::item_data;
pub use sunrise_items::items;
pub use sunrise_items::random_sequences;
pub use sunrise_world::json_folder;
pub use sunrise_world::moving_colliders;

// --- lowered modules ---
pub mod api_context;
pub mod assets;
pub mod audio_capture;
pub mod bidi;
pub mod biome;
pub mod block_edit;
pub mod block_entities;
pub mod block_entity_ticks;
pub mod brewing;
pub mod camera_math;
pub mod carrier_fences;
pub mod chunk;
pub mod chunk_map;
pub mod chunk_streamer;
pub mod collisions;
pub mod columns;
pub mod command_block;
pub mod containers;
pub mod crash;
pub mod datapack;
pub mod dyes;
pub mod effects;
pub mod environment;
pub mod fast_hash;
pub mod java_text;
pub mod jigsaw_block;
pub mod level_loading;
pub mod light;
pub mod light_border;
pub mod lod_data;
pub mod logging;
pub mod mod_chunk_tickets;
pub mod options_file;
pub mod options_fixes;
pub mod pack_image;
pub mod pack_meta;
pub mod pack_tags;
pub mod par;
pub mod particle_options;
// Its sampler sources (`sunrise_threads::heap_sample`, `::profile`) and its only user
// (`sunrise-sim`'s `server_perf::samples::linux`) are Linux-only.
#[cfg(target_os = "linux")]
pub mod perf_capture;
pub mod poi_sections;
pub mod potent_sulfur_level;
pub mod profile_lookup;
pub mod providers;
pub mod random_source;
pub mod runtime_env;
pub mod sculk;
pub mod section_mask;
pub mod shulker_box;
pub mod signs_data;
pub mod sim_settings;
pub mod sound_events;
pub mod spans;
pub mod structure_block;
pub mod test_block;
pub mod test_instance_block;
pub mod testgen;
pub mod tick_mode;
pub mod vault;
pub mod world;
pub mod worldborder;
// --- end lowered modules ---
