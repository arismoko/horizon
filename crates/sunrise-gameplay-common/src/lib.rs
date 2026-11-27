//! Portable gameplay rules and data shared across both logical sides.
#![forbid(unsafe_code)]

#[cfg(test)]
#[used]
static KEEP_TEST_PLATFORM: fn() = mod_platform::__private::test_platform::keep;
pub mod brushing;
pub mod cauldron_contents;
pub mod chests;
pub mod class_tags;
pub mod closest_space;
pub mod command_blocks;
pub mod container_openers;
pub mod crash_save;
pub mod entity_uuids;
pub mod execution_role;
pub mod game_master_blocks;
pub mod game_modes;
pub mod game_tests;
pub mod glide_wear;
pub mod hit;
pub mod hoppers;
pub mod interaction_shapes;
pub mod inventory_tick;
pub mod item_areas;
pub mod lan;
pub mod leash_knot;
pub mod lectern;
pub mod minecart_cells;
pub mod minecart_seat;
pub mod mob_index;
pub mod mount_rules;
pub mod painting_variants;
pub mod particle_options;
pub mod potent_sulfur;
pub mod projectile_spawned_validation;
pub mod projectile_world;
pub mod smithing_icons;
pub mod stats;
pub mod structure_blocks;
pub mod structures;
pub mod test_blocks;
pub mod test_entities;
pub mod test_environments;
pub mod test_instances;
pub mod world_clock;
pub mod worn_boots;

extern crate self as sunrise_gameplay_common;
pub mod game;
