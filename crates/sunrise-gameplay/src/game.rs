#[macro_use]
#[path = "game/server_side.rs"]
pub mod server_side;
#[cfg(test)]
#[path = "game/entities/entity_tick_families_tests.rs"]
mod entity_tick_families_tests;
#[allow(unused_imports)]
pub(in crate::game) use server_side::acting_state;
#[allow(unused_imports)]
pub(in crate::game) use server_side::advancements;
#[allow(unused_imports)]
pub(in crate::game) use server_side::ahead;
#[allow(unused_imports)]
pub(in crate::game) use server_side::allays;
#[allow(unused_imports)]
pub(in crate::game) use server_side::api_values_server;
#[allow(unused_imports)]
pub(in crate::game) use server_side::armor_stands;
#[allow(unused_imports)]
pub(in crate::game) use server_side::attachment_index;
#[allow(unused_imports)]
pub(in crate::game) use server_side::attachment_sync;
#[allow(unused_imports)]
pub(in crate::game) use server_side::beds;
#[allow(unused_imports)]
pub(in crate::game) use server_side::beehives;
#[allow(unused_imports)]
pub(in crate::game) use server_side::block_edits;
#[allow(unused_imports)]
pub(in crate::game) use server_side::block_entity_anims;
#[allow(unused_imports)]
pub use server_side::block_entity_ticks;
#[allow(unused_imports)]
pub(in crate::game) use server_side::block_ticks;
#[allow(unused_imports)]
pub(in crate::game) use server_side::block_use_completion;
#[allow(unused_imports)]
pub(in crate::game) use server_side::boarding;
#[allow(unused_imports)]
pub(in crate::game) use server_side::boat_authority_fixture;
#[allow(unused_imports)]
pub(in crate::game) use server_side::boat_leads;
#[allow(unused_imports)]
pub(in crate::game) use server_side::boat_passengers;
#[allow(unused_imports)]
pub(in crate::game) use server_side::boats;
#[allow(unused_imports)]
pub(in crate::game) use server_side::bossbars;
#[allow(unused_imports)]
pub(in crate::game) use server_side::brewing_remainder;
#[allow(unused_imports)]
pub(in crate::game) use server_side::bucket_empty;
#[allow(unused_imports)]
pub(in crate::game) use server_side::buckets;
#[allow(unused_imports)]
pub(in crate::game) use server_side::building_players;
#[allow(unused_imports)]
pub(in crate::game) use server_side::campfire;
#[allow(unused_imports)]
pub(in crate::game) use server_side::carrier_transfers;
#[allow(unused_imports)]
pub(in crate::game) use server_side::cat_spawner;
#[allow(unused_imports)]
pub(in crate::game) use server_side::chiseled_bookshelf;
#[allow(unused_imports)]
pub(in crate::game) use server_side::combat;
#[allow(unused_imports)]
pub use server_side::command_harness;
#[allow(unused_imports)]
pub use server_side::commands;
#[allow(unused_imports)]
pub(in crate::game) use server_side::composter;
#[allow(unused_imports)]
pub(in crate::game) use server_side::consume;
#[allow(unused_imports)]
pub(in crate::game) use server_side::container_loot;
#[allow(unused_imports)]
pub(in crate::game) use server_side::creaking;
#[allow(unused_imports)]
pub use server_side::crossbow;
#[allow(unused_imports)]
pub(crate) use server_side::cushions;
#[allow(unused_imports)]
pub(in crate::game) use server_side::custom_spawner_level;
#[allow(unused_imports)]
pub(in crate::game) use server_side::custom_spawners;
#[allow(unused_imports)]
pub(in crate::game) use server_side::debug_stick;
#[allow(unused_imports)]
pub(in crate::game) use server_side::decided_action;
#[allow(unused_imports)]
pub use server_side::dedicated;
#[allow(unused_imports)]
pub(in crate::game) use server_side::deflect;
#[allow(unused_imports)]
pub(in crate::game) use server_side::dispense_bucket;
#[allow(unused_imports)]
pub use server_side::dispense_fixture;
#[allow(unused_imports)]
pub(in crate::game) use server_side::dispenser;
#[allow(unused_imports)]
pub(in crate::game) use server_side::dragon;
#[allow(unused_imports)]
pub(in crate::game) use server_side::dropped_item_setter;
#[allow(unused_imports)]
pub(in crate::game) use server_side::dynamic_light_emission;
#[allow(unused_imports)]
pub(in crate::game) use server_side::early_game_rules;
#[allow(unused_imports)]
pub(in crate::game) use server_side::enchant_effects;
#[allow(unused_imports)]
pub(in crate::game) use server_side::end;
#[allow(unused_imports)]
pub(in crate::game) use server_side::enderman;
#[allow(unused_imports)]
pub(in crate::game) use server_side::entity_initialization;
#[allow(unused_imports)]
pub use server_side::entity_residency;
#[allow(unused_imports)]
pub(in crate::game) use server_side::entity_sizes;
#[allow(unused_imports)]
pub(in crate::game) use server_side::server_effects;
pub(in crate::game) use server_side::world_border;
#[allow(dead_code)]
pub struct ServerGame<'a> {
    pub common: sunrise_gameplay_common::game::CommonGame<'a>,
    pub server: server_side::ServerState,
}
impl<'a> std::ops::Deref for ServerGame<'a> {
    type Target = sunrise_gameplay_common::game::CommonGame<'a>;
    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        &self.common
    }
}

impl std::ops::DerefMut for ServerGame<'_> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.common
    }
}

#[allow(unused_imports)]
pub(in crate::game) use server_side::explosion;
#[allow(unused_imports)]
pub(in crate::game) use server_side::explosion_detonate;
#[allow(unused_imports)]
pub(in crate::game) use server_side::explosion_settings;
#[allow(unused_imports)]
pub(in crate::game) use server_side::extinguish;
#[allow(unused_imports)]
pub(in crate::game) use server_side::falling_blocks;
#[allow(unused_imports)]
pub(in crate::game) use server_side::falling_damage;
#[allow(unused_imports)]
pub(in crate::game) use server_side::falling_save;
#[allow(unused_imports)]
pub(in crate::game) use server_side::falling_tick;
#[allow(unused_imports)]
pub(in crate::game) use server_side::fangs;
#[allow(unused_imports)]
pub(in crate::game) use server_side::finalize_spawn;
#[allow(unused_imports)]
pub use server_side::fishing;
#[allow(unused_imports)]
pub(in crate::game) use server_side::foxes;
#[allow(unused_imports)]
pub(in crate::game) use server_side::frames;
#[allow(unused_imports)]
pub use server_side::functions;
#[allow(unused_imports)]
pub use server_side::game_mode;
#[allow(unused_imports)]
pub use server_side::game_tests;
#[allow(unused_imports)]
pub(in crate::game) use server_side::ghasts;
#[allow(unused_imports)]
pub(in crate::game) use server_side::golems;
#[allow(unused_imports)]
pub(in crate::game) use server_side::hot_paths;
#[allow(unused_imports)]
pub(in crate::game) use server_side::infiniburn;
#[allow(unused_imports)]
pub(in crate::game) use server_side::initial_spawn;
#[allow(unused_imports)]
pub(in crate::game) use server_side::integrated;
#[allow(unused_imports)]
pub(in crate::game) use server_side::item_durability;
#[allow(unused_imports)]
pub use server_side::item_entities;
#[allow(unused_imports)]
pub(in crate::game) use server_side::item_hurt;
#[allow(unused_imports)]
pub use server_side::item_use;
#[allow(unused_imports)]
pub use server_side::journal_level_write;
#[allow(unused_imports)]
pub use server_side::lan;
#[allow(unused_imports)]
pub(in crate::game) use server_side::lan_attributes;
#[allow(unused_imports)]
pub(in crate::game) use server_side::lan_camera;
#[allow(unused_imports)]
pub(in crate::game) use server_side::lan_chat;
#[allow(unused_imports)]
pub(in crate::game) use server_side::lan_configuration_tasks;
#[allow(unused_imports)]
pub(in crate::game) use server_side::lan_crafting;
#[allow(unused_imports)]
pub(in crate::game) use server_side::lan_entities;
#[allow(unused_imports)]
pub(in crate::game) use server_side::lan_far;
#[allow(unused_imports)]
pub(in crate::game) use server_side::lan_menus;
#[allow(unused_imports)]
pub use server_side::lan_mobs;
#[allow(unused_imports)]
pub(in crate::game) use server_side::lan_mount_menu;
#[allow(unused_imports)]
pub use server_side::lan_particles;
#[allow(unused_imports)]
pub(in crate::game) use server_side::lan_pickup;
#[allow(unused_imports)]
pub(in crate::game) use server_side::lan_pvp;
#[allow(unused_imports)]
pub(in crate::game) use server_side::lan_quiet;
#[allow(unused_imports)]
pub(in crate::game) use server_side::lan_stations;
#[allow(unused_imports)]
pub(in crate::game) use server_side::lan_trade_decisions;
#[allow(unused_imports)]
pub(in crate::game) use server_side::lan_trading;
#[allow(unused_imports)]
pub use server_side::lan_vehicles;
#[allow(unused_imports)]
pub(in crate::game) use server_side::lan_waypoints;
#[allow(unused_imports)]
pub use server_side::leads;
#[allow(unused_imports)]
pub(in crate::game) use server_side::level_particles;
#[allow(unused_imports)]
pub(in crate::game) use server_side::level_sounds;
#[allow(unused_imports)]
pub(in crate::game) use server_side::level_tick;
#[allow(unused_imports)]
pub(in crate::game) use server_side::levels;
#[allow(unused_imports)]
pub use server_side::living_attributes;
#[allow(unused_imports)]
pub(in crate::game) use server_side::llamas;
#[allow(unused_imports)]
pub(in crate::game) use server_side::loot_rolls;
#[allow(unused_imports)]
pub(in crate::game) use server_side::mace;
#[allow(unused_imports)]
pub use server_side::machines;
#[allow(unused_imports)]
pub(in crate::game) use server_side::melee;
#[allow(unused_imports)]
pub(in crate::game) use server_side::memory;
#[allow(unused_imports)]
pub(in crate::game) use server_side::minecart_moves;
#[allow(unused_imports)]
pub(in crate::game) use server_side::mob_gear;
#[allow(unused_imports)]
pub(in crate::game) use server_side::mob_level;
#[allow(unused_imports)]
pub(in crate::game) use server_side::mob_looting;
#[allow(unused_imports)]
pub(in crate::game) use server_side::mod_api_host_server;
#[allow(unused_imports)]
pub(in crate::game) use server_side::mod_container_openers;
#[allow(unused_imports)]
pub(in crate::game) use server_side::modapi_server;
#[allow(unused_imports)]
pub(in crate::game) use server_side::mounts;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_ai_pause;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_biome_keys;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_block_entity_refs;
#[cfg(test)]
#[path = "game/modding/native_block_entity_transform_tests.rs"]
mod native_block_entity_transform_tests;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_book_apply;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_book_catalogue;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_book_grid;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_book_invalidation;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_book_net;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_book_settings;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_book_transport;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_boss_bar_refs;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_bots;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_chunk_areas;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_chunk_queries;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_chunk_save;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_colliders;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_collision_exclusions;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_command_reads;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_console;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_criteria;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_data_maps;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_death_weapon;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_display_refs;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_dragon_refs;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_entities;
#[path = "game/modding/native_entity_base_server.rs"]
mod native_entity_base_server;
#[cfg(test)]
#[path = "game/modding/native_entity_base_tests.rs"]
mod native_entity_base_tests;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_entity_commands;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_entity_data;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_entity_extinguish;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_entity_invulnerable;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_entity_kind;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_entity_motion;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_entity_moves;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_entity_physics;
#[allow(unused_imports)]
pub use server_side::native_entity_refs;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_entity_remove;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_entity_riding;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_entity_sounds;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_entity_state;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_explosion_destroy;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_explosion_resistance;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_explosions;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_falling;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_fishing_hook_refs;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_game_event_context;
#[allow(unused_imports)]
pub use server_side::native_goal_ticks;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_hanging;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_harvest_queries;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_inventories;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_item_places;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_level_border;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_level_effects;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_level_events;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_level_items;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_level_players;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_level_refs;
#[cfg(test)]
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_level_sound_except_tests;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_level_spawning;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_level_ticking;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_level_weather;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_level_world;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_lifecycle;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_living_effects;
#[allow(unused_imports)]
pub use server_side::native_living_flags;
#[allow(unused_imports)]
pub use server_side::native_living_position;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_living_refs;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_living_writes;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_menu_places;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_menus;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_merchant_refs;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_minecart_refs;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_mob_goals;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_mob_navigation;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_mob_refs;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_mob_wanted_position;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_mobs;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_mounts;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_pending_children;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_place_feature;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_player_attack;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_player_break;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_player_cooldowns;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_player_disconnect;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_player_ender_chest;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_player_inventory;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_player_modes;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_player_moves;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_player_persistent_data;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_player_place;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_player_respawn;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_player_stats;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_player_visibility;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_portal_destination;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_profiles;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_projectile_launch;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_projectile_refs;
#[allow(unused_imports)]
pub use server_side::native_projectiles;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_quick_move;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_raid_refs;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_recipe_sync;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_ride_player;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_scoreboard;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_scoreboard_teams;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_server_distances;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_singleplayer_owner;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_sound_refs;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_structure_placement;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_structures;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_tick_time;
#[allow(unused_imports)]
pub use server_side::native_unspawned_mob;
#[allow(unused_imports)]
pub(in crate::game) use server_side::native_world_edit;
#[allow(unused_imports)]
pub(in crate::game) use server_side::natural_spawning;
#[allow(unused_imports)]
pub(in crate::game) use server_side::neoforge_block_maps;
#[allow(unused_imports)]
pub(in crate::game) use server_side::neoforge_mob_maps;
#[allow(unused_imports)]
pub use server_side::nether_portal;
#[allow(unused_imports)]
pub(in crate::game) use server_side::ominous_item_spawner;
#[allow(unused_imports)]
pub(in crate::game) use server_side::ordered_tick;
#[allow(unused_imports)]
pub(in crate::game) use server_side::original_player;
#[allow(unused_imports)]
pub(in crate::game) use server_side::paintings;
#[allow(unused_imports)]
pub(in crate::game) use server_side::pandas;
#[allow(unused_imports)]
pub(in crate::game) use server_side::part_shearing;
#[allow(unused_imports)]
pub(in crate::game) use server_side::passengers;
#[allow(unused_imports)]
pub(in crate::game) use server_side::pending_births;
#[allow(unused_imports)]
pub(in crate::game) use server_side::perf_remote;
#[allow(unused_imports)]
pub use server_side::persist;
#[allow(unused_imports)]
pub(in crate::game) use server_side::pick_hotbar;
#[allow(unused_imports)]
pub(in crate::game) use server_side::piglins;
#[allow(unused_imports)]
pub(in crate::game) use server_side::piston_carry_state;
#[allow(unused_imports)]
pub(in crate::game) use server_side::player_destroy;
#[allow(unused_imports)]
pub(in crate::game) use server_side::player_hit_sources;
#[allow(unused_imports)]
pub(in crate::game) use server_side::player_residency;
#[allow(unused_imports)]
pub(in crate::game) use server_side::player_spawn;
#[allow(unused_imports)]
pub(in crate::game) use server_side::postprocess;
#[allow(unused_imports)]
pub(in crate::game) use server_side::potent_sulfur;
#[allow(unused_imports)]
pub(in crate::game) use server_side::projectile_block_impacts;
#[allow(unused_imports)]
pub(in crate::game) use server_side::projectile_spawned;
#[allow(unused_imports)]
pub(in crate::game) use server_side::pumpkin;
#[allow(unused_imports)]
pub(in crate::game) use server_side::raids;
#[allow(unused_imports)]
pub use server_side::respawn_anchor;
#[allow(unused_imports)]
pub(in crate::game) use server_side::respawn_position;
#[allow(unused_imports)]
pub(in crate::game) use server_side::runtime_levels;
#[cfg(test)]
#[allow(unused_imports)]
#[cfg(test)]
#[path = "game/runtime/runtime_reset_tests.rs"]
mod runtime_reset_tests;
#[allow(unused_imports)]
pub(in crate::game) use server_side as server_state;
pub use server_side::creation_packs;
#[allow(unused_imports)]
pub(in crate::game) use server_side::scoreboard;
#[allow(unused_imports)]
pub(in crate::game) use server_side::sculk_fixture;
#[allow(unused_imports)]
pub(in crate::game) use server_side::sculk_server;
#[allow(unused_imports)]
pub(in crate::game) use server_side::seats;
#[allow(unused_imports)]
pub(in crate::game) use server_side::server_lifecycle;
pub use server_side::server_lifecycle::ServerRunOutcome;
#[allow(unused_imports)]
pub(in crate::game) use server_side::server_reload;
#[allow(unused_imports)]
pub use server_side::server_travel;
#[allow(unused_imports)]
pub use server_side::shield;
#[allow(unused_imports)]
pub use server_side::shoulders;
#[allow(unused_imports)]
pub use server_side::slot_changes;
#[cfg(test)]
#[path = "game/modding/slot_rules_tests.rs"]
pub mod slot_rules_tests;
#[allow(unused_imports)]
pub(in crate::game) use server_side::slot_takes;
#[allow(unused_imports)]
pub use server_side::spawner;
#[allow(unused_imports)]
pub(in crate::game) use server_side::spear_players;
#[allow(unused_imports)]
pub(in crate::game) use server_side::special_attack_fixture;
#[allow(unused_imports)]
pub(in crate::game) use server_side::special_attacks;
#[allow(unused_imports)]
pub(in crate::game) use server_side::spill;
#[allow(unused_imports)]
pub(in crate::game) use server_side::splash_water;
#[cfg(any(test, feature = "test-support"))]
#[path = "game/modding/stacked_on_other_tests.rs"]
pub mod stacked_on_other_tests;
use crate::tick_profile;
pub use hot_paths::{HotPathsMods, hot_paths_next_block_entity, note_hot_paths_listener_call};
#[allow(unused_imports)]
pub(in crate::game) use server_side::books_server;
#[allow(unused_imports)]
pub(in crate::game) use server_side::chat_server;
#[allow(unused_imports)]
pub(in crate::game) use server_side::command_blocks_server;
#[allow(unused_imports)]
pub(in crate::game) use server_side::command_minecarts_server;
#[allow(unused_imports)]
pub(in crate::game) use server_side::commands_server;
#[allow(unused_imports)]
pub(in crate::game) use server_side::creatures_server;
#[allow(unused_imports)]
pub(in crate::game) use server_side::game_server;
#[allow(unused_imports)]
pub(in crate::game) use server_side::hand_server;
#[allow(unused_imports)]
pub(in crate::game) use server_side::lightning_server;
#[allow(unused_imports)]
use server_side::loading_server::start_dedicated_game;
#[allow(unused_imports)]
pub(in crate::game) use server_side::player_server;
#[allow(unused_imports)]
pub(in crate::game) use server_side::sign_edit_server;
#[allow(unused_imports)]
pub(in crate::game) use server_side::spawner_server;
#[allow(unused_imports)]
#[allow(unused_imports)]
#[cfg(feature = "state-digest")]
#[allow(unused_imports)]
pub(in crate::game) use server_side::state_digest;
#[allow(unused_imports)]
pub use server_side::stats;
#[allow(unused_imports)]
pub(in crate::game) use server_side::structure_blocks;
#[allow(unused_imports)]
pub(in crate::game) use server_side::sulfur_cubes;
#[allow(unused_imports)]
pub(in crate::game) use server_side::table_enchant;
#[allow(unused_imports)]
pub(in crate::game) use server_side::test_blocks;
#[allow(unused_imports)]
pub(in crate::game) use server_side::test_environments;
#[allow(unused_imports)]
pub use server_side::test_instances;
#[allow(unused_imports)]
pub(in crate::game) use server_side::tnt;
#[allow(unused_imports)]
pub(in crate::game) use server_side::trading;
#[cfg(test)]
#[allow(unused_imports)]
pub(in crate::game) use server_side::transaction_entities_tests;
#[allow(unused_imports)]
pub(in crate::game) use server_side::transactions_server;
#[allow(unused_imports)]
pub(in crate::game) use server_side::trial_spawner;
#[allow(unused_imports)]
pub(in crate::game) use server_side::trident;
#[allow(unused_imports)]
pub(in crate::game) use server_side::use_hand;
#[allow(unused_imports)]
pub(in crate::game) use server_side::using_item;
#[allow(unused_imports)]
pub(in crate::game) use server_side::vault;
#[allow(unused_imports)]
pub use server_side::vehicle_teleport;
#[allow(unused_imports)]
pub(in crate::game) use server_side::view_pass;
#[allow(unused_imports)]
pub(in crate::game) use server_side::villager_professions;
#[allow(unused_imports)]
pub(in crate::game) use server_side::villagers;
#[allow(unused_imports)]
pub(in crate::game) use server_side::wake_time;
#[allow(unused_imports)]
pub(in crate::game) use server_side::wandering_trader;
#[allow(unused_imports)]
pub use server_side::wind;
#[allow(unused_imports)]
pub(in crate::game) use server_side::wither;
#[allow(unused_imports)]
pub(in crate::game) use server_side::world_fluids;
#[allow(unused_imports)]
pub use server_side::world_loading;
#[allow(unused_imports)]
pub(in crate::game) use server_side::world_update_server;
#[allow(unused_imports)]
pub(in crate::game) use server_side::xp_orb_targeting;
#[allow(unused_imports)]
pub(in crate::game) use server_side::zombie_reinforcements;
#[allow(unused_imports)]
pub(in crate::game) use server_side::zombie_villagers;
#[allow(unused_imports)]
use server_state::ServerState;

pub(in crate::game) struct WrappedServerInit {
    player_autosaves: server_side::ServerOnly<sunrise_sim::lan::playerdata::Autosaves>,
    session_lock: server_side::ServerShared<Option<sunrise_sim::save::SessionLock>>,
    paused_mob_hurt: server_side::ServerOnly<Option<Box<sunrise_sim::mobs::PausedMobHurt>>>,
    raiding: server_side::ServerOnly<raids::Raiding>,
    world_tables: server_side::ServerShared<Option<WorldTables>>,
    server_loot: server_side::ServerOnly<sunrise_sim::structure_blocks::LootSnapshot>,
    levels: server_side::ServerOnly<levels::Levels>,
    tickets: server_side::ServerShared<sunrise_sim::ticket_levels::ChunkTickets>,
    random_tick_sections:
        server_side::ServerOnly<sunrise_sim::random_tick_sections::RandomTickSections>,
    spill: server_side::ServerOnly<spill_common::Spill>,
    game_tests: server_side::ServerOnly<game_tests::GameTests>,
    native_commands:
        server_side::ServerOnly<sunrise_gameplay_common::game::native_server::NativeCommandState>,
    lan_building_boxes: server_side::ServerOnly<server_side::building_players::BuildingPlayers>,
    lan_displaced_players:
        server_side::ServerOnly<Vec<server_side::lan_actions_server::DisplacedPlayer>>,
    fake_action: server_side::ServerOnly<
        std::sync::Arc<sunrise_gameplay_common::game::native_server::FakeAction>,
    >,
    server_lists:
        server_side::ServerOnly<Option<std::sync::Arc<std::sync::Mutex<dedicated::Lists>>>>,
    cmds: server_side::ServerOnly<commands::extra::CommandExtras>,
    reload_resources: server_side::ServerOnly<mod_api::RetainedListeners>,
    combat: server_side::ServerOnly<combat::CombatTracker>,
    guest_combat: server_side::ServerOnly<std::collections::HashMap<u32, combat::CombatTracker>>,
    fns: server_side::ServerOnly<functions::FnState>,
    exec: server_side::ServerOnly<commands::execute::ExecState>,
}

use sunrise_gameplay_common::game::spill_common;
use sunrise_gameplay_common::game::{WorldTables, WrappedPrepInit};
struct ConstructInit<const NO_SLOT: bool>;
impl<const NO_SLOT: bool> ConstructInit<NO_SLOT> {
    #[inline(never)]
    fn build_server_be_ticks(
        registry: &'static sunrise_world::registry::BlockRegistry,
    ) -> sunrise_level::block_entity_ticks::BlockEntityTicks {
        let mut anims = sunrise_level::block_entity_ticks::BlockEntityTicks::default();
        anims.classify(registry);
        anims
    }
    #[inline(never)]
    fn build_wrapped_server() -> Box<WrappedServerInit> {
        Box::new(WrappedServerInit {
            player_autosaves: server_side::ServerOnly::new(Default::default()),
            session_lock: server_side::ServerShared::new(None),
            paused_mob_hurt: server_side::ServerOnly::new(None),
            raiding: server_side::ServerOnly::new(Default::default()),
            world_tables: server_side::ServerShared::new(None),
            server_loot: server_side::ServerOnly::new(Default::default()),
            levels: server_side::ServerOnly::new(Default::default()),
            tickets: server_side::ServerShared::new(Default::default()),
            random_tick_sections: server_side::ServerOnly::new(Default::default()),
            spill: server_side::ServerOnly::new(Default::default()),
            game_tests: server_side::ServerOnly::new(game_tests::GameTests::default()),
            native_commands: server_side::ServerOnly::new(Default::default()),
            lan_building_boxes: server_side::ServerOnly::new(Default::default()),
            lan_displaced_players: server_side::ServerOnly::new(Vec::new()),
            fake_action: server_side::ServerOnly::new(Default::default()),
            server_lists: server_side::ServerOnly::new(None),
            cmds: server_side::ServerOnly::new(Default::default()),
            reload_resources: server_side::ServerOnly::new(Default::default()),
            combat: server_side::ServerOnly::new(Default::default()),
            guest_combat: server_side::ServerOnly::new(Default::default()),
            fns: server_side::ServerOnly::new(Default::default()),
            exec: server_side::ServerOnly::new(Default::default()),
        })
    }
    #[inline(never)]
    fn wrap_server_prep_and_effects(
        prep: Box<server_side::ServerConstructPrep>,
        effects: Box<server_side::ServerConstructEffects>,
    ) -> (Box<WrappedPrepInit>, Box<ServerState>) {
        let ws = Self::build_wrapped_server();
        let server = ServerState::build(
            (
                prep.streaming,
                Self::build_server_be_ticks(prep.pack.assets.registry),
                prep.settings,
                effects.profile,
            ),
            effects.structure_spawns,
            effects.profile_resolver,
            server_side::ServerShared::new(prep.save),
            server_side::ServerOnly::new(prep.creative_catalog),
            ws,
        );
        let wp = Box::new(WrappedPrepInit {
            item_features: prep.item_features,
            level: prep.level,
            painting_variants: prep.painting_variants,
            world: prep.world,
            now: prep.now,
            pack: prep.pack,
            options: prep.options,
            clock: effects.clock,
            tick: effects.tick,
            ui: effects.ui,
        });
        (wp, server)
    }
}

pub(in crate::game) use server_side::durability;

#[path = "game/modding/native_registry_entries.rs"]
pub(in crate::game) mod native_registry_entries;

#[path = "game/modding/native_resources.rs"]
pub(in crate::game) mod native_resources;
#[cfg(any(test, feature = "test-support"))]
pub use server_side::entity_server;
#[cfg(not(any(test, feature = "test-support")))]
pub(in crate::game) use server_side::entity_server;
pub(in crate::game) use server_side::minecarts_server;
use sunrise_gameplay_common::game::SideGame;
impl<'g> SideGame<'g> for ServerGame<'g> {
    type State = server_side::ServerState;
    fn side_state(&self) -> &Self::State {
        &self.server
    }
    fn side_state_mut(&mut self) -> &mut Self::State {
        &mut self.server
    }

    fn common_and_state_mut(
        &mut self,
    ) -> (
        &mut sunrise_gameplay_common::game::CommonGame<'g>,
        &mut Self::State,
    ) {
        (&mut self.common, &mut self.server)
    }
}

#[path = "game/side_ops/mod.rs"]
pub(in crate::game) mod side_ops;

#[path = "game/runtime/transactions.rs"]
pub(in crate::game) mod transactions;

#[path = "game/network/remote/mod.rs"]
pub mod remote;

#[path = "game/network/lan_actions/mod.rs"]
pub mod lan_actions;

pub mod native_hooks;

#[path = "game/modding/native_server.rs"]
pub mod native_server;

#[path = "game/modding/native_player_death.rs"]
pub mod native_player_death;

#[path = "game/modding/native_passages.rs"]
pub mod native_passages;

#[path = "game/modding/native_configs.rs"]
mod native_configs;

#[path = "game/modding/mods/mod.rs"]
pub mod mods;

#[path = "game/modding/api_values.rs"]
pub(crate) mod api_values;

#[path = "game/mobs/creatures/mod.rs"]
pub mod creatures;

mod host;

#[path = "game/client_side.rs"]
pub mod client_side;

impl sunrise_sim::crafting::CraftingRemainder for ServerGame<'_> {
    fn crafting_remainder(
        &self,
        stack: &sunrise_items::items::ItemStack,
    ) -> Option<sunrise_items::items::ItemStack> {
        sunrise_gameplay_common::game::native_block_states::NativeBlockStatesGameExt1::crafting_remainder(self, stack)
    }
}

impl sunrise_sim::menu::SlotRules for ServerGame<'_> {
    fn listens(&self, menu: &str) -> bool {
        sunrise_gameplay_common::game::slot_rules::GameSlotRules::listens(self, menu)
    }

    fn rule(
        &self,
        menu: &str,
        slot: usize,
        stack: &sunrise_items::items::ItemStack,
        context: sunrise_sim::menu::SlotContext<'_>,
        may_place: &mut bool,
        may_pickup: &mut bool,
        max_stack: &mut u32,
    ) {
        sunrise_gameplay_common::game::slot_rules::GameSlotRules::rule(
            self, menu, slot, stack, context, may_place, may_pickup, max_stack,
        );
    }

    fn pick_all_listens(&self, menu: &str) -> bool {
        sunrise_gameplay_common::game::slot_rules::GameSlotRules::pick_all_listens(self, menu)
    }

    fn pick_all(
        &self,
        menu: &str,
        slot: usize,
        carried: &sunrise_items::items::ItemStack,
        stack: &sunrise_items::items::ItemStack,
        context: sunrise_sim::menu::SlotContext<'_>,
        allowed: &mut bool,
    ) {
        sunrise_gameplay_common::game::slot_rules::GameSlotRules::pick_all(
            self, menu, slot, carried, stack, context, allowed,
        );
    }
}

impl mod_api::AttachmentHost for ServerGame<'_> {
    fn validate_attachment_key(
        &self,
        write: &mod_api::AttachmentStoreKey<'_>,
    ) -> std::result::Result<(), mod_api::AttachmentError> {
        sunrise_gameplay_common::game::mod_api_host_common::GameAttachmentHost::validate_attachment_key(
            self, write,
        )
    }

    fn validate_attachment(
        &self,
        namespace: &str,
        name: &str,
        sync: bool,
    ) -> std::result::Result<(), mod_api::AttachmentError> {
        sunrise_gameplay_common::game::mod_api_host_common::GameAttachmentHost::validate_attachment(
            self, namespace, name, sync,
        )
    }

    fn read_attachments(
        &mut self,
        holder: mod_api::Holder,
    ) -> std::result::Result<Option<&mod_api::Attachments>, mod_api::AttachmentError> {
        self.attachment_map_read(holder)
    }

    fn attachment_changed(&mut self, holder: mod_api::Holder, persist: bool, sync: bool) {
        if self.native_authoritative() {
            sunrise_gameplay_common::game::mod_api_host::world_attachment_changed(
                &mut self.common.world,
                holder,
                persist,
                sync,
            );
        }
    }

    fn attachments(
        &mut self,
        holder: mod_api::Holder,
        access: mod_api::AttachmentAccess,
    ) -> std::result::Result<&mut mod_api::Attachments, mod_api::AttachmentError> {
        sunrise_gameplay_common::game::mod_api_host_common::GameAttachmentHost::attachments(
            self, holder, access,
        )
    }
}

#[cfg(any(test, feature = "test-support"))]
pub use server_side::fireworks_server::impact::ROCKET_IMPACT_PREPARES;
#[cfg(any(test, feature = "test-support"))]
pub use server_side::native_guest_turn_tests;
