//! Server-owned fields of [`super::Game`].
use {crate::game::*, sunrise_gameplay_common::game::*};

#[macro_use]
#[path = "server_macros.rs"]
mod server_macros;
#[path = "world/creation_packs.rs"]
pub mod creation_packs;
#[cfg(test)]
#[path = "interface/fluid_passage_tests.rs"]
mod fluid_passage_tests;
#[cfg(test)]
#[path = "modding/native_block_metadata_reads_tests.rs"]
mod native_block_metadata_reads_tests;
#[path = "modding/native_chunk_requests.rs"]
pub(in crate::game) mod native_chunk_requests;
#[cfg(test)]
#[path = "modding/native_chunk_snapshot_server_tests.rs"]
mod native_chunk_snapshot_server_tests;
#[cfg(test)]
#[path = "modding/native_command_entity_tests.rs"]
mod native_command_entity_tests;
#[cfg(test)]
#[path = "modding/native_convention_tags_tests.rs"]
mod native_convention_tags_tests;
#[path = "modding/native_datapack_values_server.rs"]
pub(in crate::game) mod native_datapack_values_server;
#[path = "modding/native_detached_generation.rs"]
pub(in crate::game) mod native_detached_generation;
#[path = "modding/native_entity_previous_server.rs"]
pub(in crate::game) mod native_entity_previous_server;
#[cfg(test)]
#[path = "modding/native_entity_previous_tests_server.rs"]
mod native_entity_previous_tests_server;
#[cfg(test)]
#[path = "modding/native_heal_motion_view_tests.rs"]
mod native_heal_motion_view_tests;
#[path = "modding/native_mob_attack.rs"]
pub(in crate::game) mod native_mob_attack;
#[path = "modding/native_player_post_effects.rs"]
pub(in crate::game) mod native_player_post_effects;
#[path = "modding/native_saved_chunks.rs"]
pub(in crate::game) mod native_saved_chunks;
#[path = "world/world_border_server.rs"]
pub(in crate::game) mod world_border;

#[path = "network/lan_actions/acting_state.rs"]
pub(in crate::game) mod acting_state;
#[path = "interface/advancements/mod.rs"]
pub mod advancements;
#[path = "entities/ahead.rs"]
pub mod ahead;
#[path = "mobs/allays.rs"]
pub(in crate::game) mod allays;
#[path = "modding/api_values_server.rs"]
pub(in crate::game) mod api_values_server;
#[path = "entities/armor_stands.rs"]
pub(in crate::game) mod armor_stands;
#[path = "modding/attachment_index/mod.rs"]
pub(in crate::game) mod attachment_index;
#[path = "blocks/beds.rs"]
pub(in crate::game) mod beds;
#[path = "mobs/beehives.rs"]
pub(in crate::game) mod beehives;
#[path = "blocks/block_edits.rs"]
pub(in crate::game) mod block_edits;
#[path = "blocks/block_entity_anims.rs"]
pub mod block_entity_anims;
#[path = "blocks/block_ticks.rs"]
pub(in crate::game) mod block_ticks;
#[path = "modding/block_use_completion.rs"]
pub(in crate::game) mod block_use_completion;
#[path = "entities/boarding.rs"]
pub(in crate::game) mod boarding;
#[path = "entities/boat_authority_fixture.rs"]
pub(in crate::game) mod boat_authority_fixture;
#[path = "entities/boat_leads.rs"]
pub mod boat_leads;
#[path = "entities/boat_passengers.rs"]
pub mod boat_passengers;
#[path = "entities/boats/mod.rs"]
pub mod boats;
#[path = "interface/books_server.rs"]
pub(in crate::game) mod books_server;
#[path = "interface/bossbars.rs"]
pub mod bossbars;
#[path = "blocks/brewing_remainder.rs"]
pub(in crate::game) mod brewing_remainder;
#[path = "items/bucket_empty.rs"]
pub(in crate::game) mod bucket_empty;
#[path = "items/buckets.rs"]
pub(in crate::game) mod buckets;
#[path = "interface/building_players.rs"]
pub(in crate::game) mod building_players;
#[path = "blocks/campfire.rs"]
pub(in crate::game) mod campfire;
#[path = "world/carrier_transfers.rs"]
pub mod carrier_transfers;
#[path = "mobs/cat_spawner.rs"]
pub(in crate::game) mod cat_spawner;
#[path = "interface/chat_server.rs"]
pub(in crate::game) mod chat_server;
#[path = "items/combat.rs"]
pub mod combat;
#[path = "administration/command_blocks_server.rs"]
pub(in crate::game) mod command_blocks_server;
#[path = "administration/command_harness.rs"]
pub mod command_harness;
#[path = "administration/command_minecarts_server.rs"]
pub(in crate::game) mod command_minecarts_server;
#[path = "administration/commands/mod.rs"]
pub mod commands;
#[path = "administration/commands/mod_server.rs"]
pub mod commands_server;
#[path = "blocks/composter.rs"]
pub mod composter;
#[path = "items/consume.rs"]
pub mod consume;
#[path = "world/container_loot.rs"]
pub(in crate::game) mod container_loot;
#[path = "mobs/creaking.rs"]
pub(in crate::game) mod creaking;
#[path = "mobs/creatures/mod_server.rs"]
pub mod creatures_server;
#[path = "items/crossbow.rs"]
pub mod crossbow;
#[path = "entities/cushions.rs"]
pub(crate) mod cushions;
#[path = "mobs/custom_spawner_level.rs"]
pub(in crate::game) mod custom_spawner_level;
#[path = "mobs/custom_spawners.rs"]
pub(in crate::game) mod custom_spawners;
#[path = "blocks/debug_stick.rs"]
pub(in crate::game) mod debug_stick;
#[path = "modding/decided_action.rs"]
pub mod decided_action;
#[path = "interface/dedicated/mod.rs"]
pub mod dedicated;
#[path = "items/deflect.rs"]
pub(in crate::game) mod deflect;
#[path = "interface/dispense_fixture.rs"]
pub mod dispense_fixture;
#[path = "blocks/dispenser.rs"]
pub mod dispenser;
#[path = "mobs/dragon.rs"]
pub mod dragon;
#[path = "modding/dropped_item_setter.rs"]
pub(in crate::game) mod dropped_item_setter;
#[path = "world/dynamic_light_emission.rs"]
pub(in crate::game) mod dynamic_light_emission;
#[path = "world/early_game_rules.rs"]
pub(in crate::game) mod early_game_rules;
#[path = "items/enchant_effects.rs"]
pub(in crate::game) mod enchant_effects;
#[path = "world/end.rs"]
pub(in crate::game) mod end;
#[path = "modding/entity_initialization.rs"]
pub(in crate::game) mod entity_initialization;
#[path = "entities/entity_residency.rs"]
pub mod entity_residency;
#[path = "modding/entity_sizes.rs"]
pub mod entity_sizes;
#[path = "entities/explosion.rs"]
pub mod explosion;
#[path = "entities/explosion_detonate.rs"]
pub(in crate::game) mod explosion_detonate;
#[path = "entities/explosion_settings.rs"]
pub(in crate::game) mod explosion_settings;
#[path = "entities/extinguish.rs"]
pub(in crate::game) mod extinguish;
#[path = "blocks/falling_blocks.rs"]
pub(in crate::game) mod falling_blocks;
#[path = "entities/falling_damage.rs"]
pub(in crate::game) mod falling_damage;
#[path = "entities/falling_save.rs"]
pub(in crate::game) mod falling_save;
#[path = "entities/falling_tick.rs"]
pub(in crate::game) mod falling_tick;
#[path = "mobs/fangs.rs"]
pub mod fangs;
#[path = "mobs/finalize_spawn.rs"]
pub(in crate::game) mod finalize_spawn;
#[path = "items/fishing.rs"]
pub mod fishing;
#[path = "mobs/foxes.rs"]
pub(in crate::game) mod foxes;
#[path = "entities/frames.rs"]
pub mod frames;
#[path = "administration/functions.rs"]
pub mod functions;
#[path = "interface/game_mode.rs"]
pub mod game_mode;
#[path = "game_server.rs"]
pub(in crate::game) mod game_server;
#[path = "administration/game_tests.rs"]
pub mod game_tests;
#[path = "mobs/golems.rs"]
pub(in crate::game) mod golems;
#[path = "items/hand_server.rs"]
pub(in crate::game) mod hand_server;
#[path = "runtime/hot_paths/mod.rs"]
pub(in crate::game) mod hot_paths;
#[path = "world/infiniburn.rs"]
pub(in crate::game) mod infiniburn;
#[path = "world/initial_spawn.rs"]
pub(in crate::game) mod initial_spawn;
#[path = "world/integrated.rs"]
pub mod integrated;
#[path = "items/item_durability.rs"]
pub(in crate::game) mod item_durability;
#[path = "items/item_entities.rs"]
pub mod item_entities;
#[path = "entities/item_hurt.rs"]
pub(in crate::game) mod item_hurt;
#[path = "items/item_use.rs"]
pub mod item_use;
#[path = "modding/journal_level_write.rs"]
pub mod journal_level_write;
#[path = "network/lan/mod.rs"]
pub mod lan;
#[path = "network/lan_attributes.rs"]
pub mod lan_attributes;
#[path = "network/lan_camera.rs"]
pub(in crate::game) mod lan_camera;
#[path = "network/lan_chat.rs"]
pub mod lan_chat;
#[path = "network/lan_configuration_tasks.rs"]
pub(in crate::game) mod lan_configuration_tasks;
#[path = "network/lan_crafting.rs"]
pub(in crate::game) mod lan_crafting;
#[path = "network/lan_entities.rs"]
pub(in crate::game) mod lan_entities;
#[path = "network/lan_far.rs"]
pub(in crate::game) mod lan_far;
#[path = "network/lan_menus.rs"]
pub(in crate::game) mod lan_menus;
#[path = "network/lan_mobs.rs"]
pub mod lan_mobs;
#[path = "network/lan_particles.rs"]
pub mod lan_particles;
#[path = "network/lan_pickup.rs"]
pub(in crate::game) mod lan_pickup;
#[path = "network/lan_pvp.rs"]
pub mod lan_pvp;
#[path = "network/lan_quiet.rs"]
pub(in crate::game) mod lan_quiet;
#[path = "network/lan_trading.rs"]
pub mod lan_trading;
#[path = "network/lan_vehicles.rs"]
pub mod lan_vehicles;
#[path = "network/lan_waypoints.rs"]
pub(in crate::game) mod lan_waypoints;
#[path = "entities/leads.rs"]
pub mod leads;
#[path = "interface/level_particles.rs"]
pub(in crate::game) mod level_particles;
#[path = "interface/level_sounds.rs"]
pub(in crate::game) mod level_sounds;
#[path = "world/level_tick.rs"]
pub(in crate::game) mod level_tick;
#[path = "world/levels.rs"]
pub mod levels;
#[path = "entities/lightning_server.rs"]
pub(in crate::game) mod lightning_server;
#[path = "mobs/living_attributes.rs"]
pub mod living_attributes;
#[path = "mobs/llamas.rs"]
pub(in crate::game) mod llamas;
#[path = "mobs/loot_rolls.rs"]
pub(in crate::game) mod loot_rolls;
#[path = "items/mace.rs"]
pub mod mace;
#[path = "blocks/machines/mod.rs"]
pub mod machines;
#[path = "entities/melee.rs"]
pub(in crate::game) mod melee;
#[path = "runtime/memory.rs"]
pub(in crate::game) mod memory;
#[path = "mobs/mob_gear.rs"]
pub(in crate::game) mod mob_gear;
#[path = "mobs/mob_level.rs"]
pub(in crate::game) mod mob_level;
#[path = "mobs/mob_looting.rs"]
pub(in crate::game) mod mob_looting;
#[path = "modding/mod_api_host_server.rs"]
pub(in crate::game) mod mod_api_host_server;
#[path = "modding/mod_container_openers.rs"]
pub(in crate::game) mod mod_container_openers;
#[path = "side_ops/modapi_server.rs"]
pub(in crate::game) mod modapi_server;
#[path = "mobs/mounts.rs"]
pub mod mounts;
#[path = "modding/native_ai_pause.rs"]
pub(in crate::game) mod native_ai_pause;
#[path = "modding/native_biome_keys.rs"]
pub(in crate::game) mod native_biome_keys;
#[path = "modding/native_block_entity_refs.rs"]
pub(in crate::game) mod native_block_entity_refs;
#[path = "modding/native_book_apply.rs"]
pub(in crate::game) mod native_book_apply;
#[path = "modding/native_book_catalogue.rs"]
pub(in crate::game) mod native_book_catalogue;
#[path = "modding/native_book_grid.rs"]
pub mod native_book_grid;
#[path = "modding/native_book_invalidation.rs"]
pub(in crate::game) mod native_book_invalidation;
#[path = "modding/native_book_net.rs"]
pub(in crate::game) mod native_book_net;
#[path = "modding/native_book_settings.rs"]
pub(in crate::game) mod native_book_settings;
#[path = "modding/native_book_transport.rs"]
pub(in crate::game) mod native_book_transport;
#[path = "modding/native_boss_bar_refs.rs"]
pub(in crate::game) mod native_boss_bar_refs;
#[path = "modding/native_bots.rs"]
pub(in crate::game) mod native_bots;
#[path = "modding/native_chunk_areas.rs"]
pub(in crate::game) mod native_chunk_areas;
#[path = "modding/native_chunk_save.rs"]
pub(in crate::game) mod native_chunk_save;
#[path = "modding/native_colliders.rs"]
pub(in crate::game) mod native_colliders;
#[path = "modding/native_collision_exclusions.rs"]
pub(in crate::game) mod native_collision_exclusions;
#[path = "modding/native_console.rs"]
pub mod native_console;
#[path = "modding/native_criteria.rs"]
pub(in crate::game) mod native_criteria;
#[path = "modding/native_data_maps.rs"]
pub(in crate::game) mod native_data_maps;
#[path = "modding/native_display_refs.rs"]
pub(in crate::game) mod native_display_refs;
#[path = "modding/native_dragon_refs.rs"]
pub(in crate::game) mod native_dragon_refs;
#[path = "modding/native_entities.rs"]
pub(in crate::game) mod native_entities;
#[path = "modding/native_entity_commands.rs"]
pub(in crate::game) mod native_entity_commands;
#[path = "modding/native_entity_data.rs"]
pub(in crate::game) mod native_entity_data;
#[path = "modding/native_entity_invulnerable.rs"]
pub mod native_entity_invulnerable;
#[path = "modding/native_entity_kind.rs"]
pub(in crate::game) mod native_entity_kind;
#[path = "modding/native_entity_motion.rs"]
pub(in crate::game) mod native_entity_motion;
#[path = "modding/native_entity_moves.rs"]
pub(in crate::game) mod native_entity_moves;
#[path = "modding/native_entity_physics.rs"]
pub(in crate::game) mod native_entity_physics;
#[path = "modding/native_entity_refs.rs"]
pub mod native_entity_refs;
#[path = "modding/native_entity_remove.rs"]
pub(in crate::game) mod native_entity_remove;
#[path = "modding/native_entity_riding.rs"]
pub(in crate::game) mod native_entity_riding;
#[path = "modding/native_entity_sounds.rs"]
pub(in crate::game) mod native_entity_sounds;
#[path = "modding/native_entity_state.rs"]
pub mod native_entity_state;
#[path = "modding/native_explosion_destroy.rs"]
pub(in crate::game) mod native_explosion_destroy;
#[path = "modding/native_explosion_resistance.rs"]
pub(in crate::game) mod native_explosion_resistance;
#[path = "modding/native_explosions.rs"]
pub(in crate::game) mod native_explosions;
#[path = "modding/native_falling.rs"]
pub(in crate::game) mod native_falling;
#[path = "modding/native_fishing_hook_refs.rs"]
pub(in crate::game) mod native_fishing_hook_refs;
#[path = "modding/native_game_event_context.rs"]
pub(in crate::game) mod native_game_event_context;
#[path = "modding/native_goal_ticks/mod.rs"]
pub mod native_goal_ticks;
#[cfg(any(test, feature = "test-support"))]
#[path = "modding/native_guest_turn_tests.rs"]
pub mod native_guest_turn_tests;
#[cfg(test)]
#[path = "modding/native_player_inventory_tests.rs"]
mod native_player_inventory_tests;

#[path = "modding/native_hanging.rs"]
pub(in crate::game) mod native_hanging;
#[path = "modding/native_harvest_queries.rs"]
pub(in crate::game) mod native_harvest_queries;
#[path = "modding/native_inventories.rs"]
pub mod native_inventories;
#[path = "modding/native_item_places.rs"]
pub mod native_item_places;
#[cfg(test)]
#[path = "modding/native_item_places_effects_tests.rs"]
mod native_item_places_effects_tests;
#[path = "modding/native_level_border.rs"]
pub(in crate::game) mod native_level_border;
#[path = "modding/native_level_effects.rs"]
pub mod native_level_effects;
#[path = "modding/native_level_events.rs"]
pub(in crate::game) mod native_level_events;
#[path = "modding/native_level_items.rs"]
pub mod native_level_items;
#[cfg(test)]
#[path = "modding/native_level_notify_tests.rs"]
mod native_level_notify_tests;
#[path = "modding/native_level_players.rs"]
pub mod native_level_players;
#[path = "modding/native_level_refs.rs"]
pub mod native_level_refs;
#[cfg(any(test, feature = "test-support"))]
#[path = "modding/native_level_sound_except_tests.rs"]
pub mod native_level_sound_except_tests;
#[path = "modding/native_level_spawning.rs"]
pub(in crate::game) mod native_level_spawning;
#[path = "modding/native_level_ticking.rs"]
pub(in crate::game) mod native_level_ticking;
#[path = "modding/native_level_world.rs"]
pub(in crate::game) mod native_level_world;
#[path = "modding/native_lifecycle.rs"]
pub(in crate::game) mod native_lifecycle;
#[path = "modding/native_living_effects.rs"]
pub(in crate::game) mod native_living_effects;
#[path = "modding/native_living_flags.rs"]
pub mod native_living_flags;

#[path = "modding/native_living_position.rs"]
pub mod native_living_position;
#[path = "modding/native_living_refs.rs"]
pub mod native_living_refs;
#[path = "modding/native_living_writes.rs"]
pub(in crate::game) mod native_living_writes;
#[path = "modding/native_menu_places.rs"]
pub(in crate::game) mod native_menu_places;
#[path = "modding/native_menus.rs"]
pub mod native_menus;
#[path = "modding/native_merchant_refs.rs"]
pub mod native_merchant_refs;
#[path = "modding/native_minecart_refs.rs"]
pub(in crate::game) mod native_minecart_refs;
#[path = "modding/native_mob_goals.rs"]
pub(in crate::game) mod native_mob_goals;
#[path = "modding/native_mob_refs.rs"]
pub(in crate::game) mod native_mob_refs;
#[cfg(test)]
#[path = "modding/native_mob_refs_client.rs"]
mod native_mob_refs_client;
#[path = "modding/native_mob_wanted_position.rs"]
pub(in crate::game) mod native_mob_wanted_position;
#[path = "modding/native_mobs.rs"]
pub(in crate::game) mod native_mobs;
#[path = "modding/native_mounts.rs"]
pub(in crate::game) mod native_mounts;
#[path = "modding/native_pending_children.rs"]
pub mod native_pending_children;
#[path = "modding/native_place_feature.rs"]
pub(in crate::game) mod native_place_feature;
#[path = "modding/native_player_break.rs"]
pub(in crate::game) mod native_player_break;
#[path = "modding/native_player_cooldowns.rs"]
pub(in crate::game) mod native_player_cooldowns;
#[path = "modding/native_player_disconnect.rs"]
pub(in crate::game) mod native_player_disconnect;
#[path = "modding/native_player_ender_chest.rs"]
pub(in crate::game) mod native_player_ender_chest;
#[path = "modding/native_player_inventory.rs"]
pub(in crate::game) mod native_player_inventory;
#[path = "modding/native_player_modes.rs"]
pub(in crate::game) mod native_player_modes;
#[path = "modding/native_player_moves.rs"]
pub(in crate::game) mod native_player_moves;
#[path = "modding/native_player_persistent_data.rs"]
pub(in crate::game) mod native_player_persistent_data;
#[path = "modding/native_player_respawn.rs"]
pub(in crate::game) mod native_player_respawn;
#[path = "modding/native_player_stats.rs"]
pub(in crate::game) mod native_player_stats;
#[path = "modding/native_player_visibility.rs"]
pub(in crate::game) mod native_player_visibility;
#[path = "modding/native_projectile_launch.rs"]
pub(in crate::game) mod native_projectile_launch;
#[path = "modding/native_projectile_refs.rs"]
pub(in crate::game) mod native_projectile_refs;
#[path = "modding/native_projectiles.rs"]
pub mod native_projectiles;
#[path = "modding/native_quick_move.rs"]
pub mod native_quick_move;
#[path = "modding/native_raid_refs.rs"]
pub(in crate::game) mod native_raid_refs;
#[path = "modding/native_ride_player.rs"]
pub(in crate::game) mod native_ride_player;
#[path = "modding/native_scoreboard.rs"]
pub mod native_scoreboard;
#[path = "modding/native_server_distances.rs"]
pub(in crate::game) mod native_server_distances;
#[path = "modding/native_server_max_players.rs"]
pub(in crate::game) mod native_server_max_players;
#[path = "modding/native_singleplayer_owner.rs"]
pub mod native_singleplayer_owner;
#[path = "modding/native_sound_refs.rs"]
pub(in crate::game) mod native_sound_refs;
#[path = "modding/native_structure_placement.rs"]
pub(in crate::game) mod native_structure_placement;
#[path = "modding/native_structures.rs"]
pub(in crate::game) mod native_structures;
#[path = "modding/native_tick_time.rs"]
pub(in crate::game) mod native_tick_time;
#[path = "modding/native_unspawned_mob.rs"]
pub mod native_unspawned_mob;
#[path = "modding/native_world_edit.rs"]
pub(in crate::game) mod native_world_edit;
#[path = "mobs/natural_spawning.rs"]
pub mod natural_spawning;
#[path = "modding/neoforge_block_maps.rs"]
pub(in crate::game) mod neoforge_block_maps;
#[path = "modding/neoforge_mob_maps.rs"]
pub(in crate::game) mod neoforge_mob_maps;
#[path = "blocks/nether_portal.rs"]
pub mod nether_portal;
#[path = "mobs/ominous_item_spawner/mod.rs"]
pub(in crate::game) mod ominous_item_spawner;
#[path = "world/ordered_tick.rs"]
pub(in crate::game) mod ordered_tick;
#[path = "modding/original_player.rs"]
pub(in crate::game) mod original_player;
#[path = "entities/paintings.rs"]
pub(in crate::game) mod paintings;
#[path = "mobs/pandas.rs"]
pub(in crate::game) mod pandas;
#[path = "mobs/part_shearing.rs"]
pub(in crate::game) mod part_shearing;
#[path = "entities/passengers.rs"]
pub mod passengers;
#[path = "mobs/pending_births.rs"]
pub(in crate::game) mod pending_births;
#[cfg(feature = "test-support")]
pub use self::pending_births::escaping_fault as pending_birth_fault;
#[cfg(test)]
#[path = "modding/native_knockback_server_tests.rs"]
mod native_knockback_server_tests;
#[path = "interface/perf_remote.rs"]
pub(in crate::game) mod perf_remote;
#[path = "world/persist.rs"]
pub mod persist;
#[path = "items/pick_hotbar.rs"]
pub(in crate::game) mod pick_hotbar;
#[path = "mobs/piglins.rs"]
pub(in crate::game) mod piglins;
#[path = "blocks/piston_carry_state.rs"]
pub(in crate::game) mod piston_carry_state;
#[path = "blocks/player_destroy.rs"]
pub(in crate::game) mod player_destroy;
#[path = "items/player_hit_sources.rs"]
pub(in crate::game) mod player_hit_sources;
#[path = "entities/player_residency.rs"]
pub(in crate::game) mod player_residency;
#[path = "player_server.rs"]
pub(in crate::game) mod player_server;
#[path = "world/player_spawn.rs"]
pub(in crate::game) mod player_spawn;
#[path = "world/postprocess.rs"]
pub(in crate::game) mod postprocess;
#[path = "blocks/potent_sulfur.rs"]
pub(in crate::game) mod potent_sulfur;
#[path = "mobs/projectile_block_impacts.rs"]
pub(in crate::game) mod projectile_block_impacts;
#[path = "items/projectile_spawned.rs"]
pub(in crate::game) mod projectile_spawned;
#[path = "blocks/pumpkin.rs"]
pub(in crate::game) mod pumpkin;
#[path = "mobs/raids/mod.rs"]
pub mod raids;
#[path = "blocks/respawn_anchor.rs"]
pub mod respawn_anchor;
#[path = "world/respawn_position.rs"]
pub(in crate::game) mod respawn_position;
#[path = "world/runtime_levels.rs"]
pub mod runtime_levels;
#[path = "interface/scoreboard/mod.rs"]
pub(in crate::game) mod scoreboard;
#[path = "blocks/sculk_fixture.rs"]
pub(in crate::game) mod sculk_fixture;
#[path = "blocks/sculk_server.rs"]
pub mod sculk_server;
#[path = "entities/seats.rs"]
pub(in crate::game) mod seats;
#[path = "server_side/server_effects.rs"]
pub(in crate::game) mod server_effects;
#[path = "world/server_lifecycle/mod.rs"]
pub mod server_lifecycle;
#[cfg(test)]
#[path = "world/server_lifecycle/fixtures.rs"]
mod server_lifecycle_fixtures;
#[path = "world/server_reload/mod.rs"]
pub(in crate::game) mod server_reload;
#[path = "world/server_travel.rs"]
pub mod server_travel;
#[path = "items/shield.rs"]
pub mod shield;
#[path = "mobs/shoulders.rs"]
pub mod shoulders;
#[path = "interface/sign_edit_server.rs"]
pub(in crate::game) mod sign_edit_server;
#[path = "modding/slot_changes.rs"]
pub mod slot_changes;
#[path = "modding/slot_takes.rs"]
pub(in crate::game) mod slot_takes;
#[path = "mobs/spawner.rs"]
pub mod spawner;
#[path = "mobs/spawner_server.rs"]
pub(in crate::game) mod spawner_server;
#[path = "items/spear_players.rs"]
pub(in crate::game) mod spear_players;
#[path = "items/special_attack_fixture.rs"]
pub(in crate::game) mod special_attack_fixture;
#[path = "items/special_attacks.rs"]
pub(in crate::game) mod special_attacks;
#[path = "blocks/spill.rs"]
pub(in crate::game) mod spill;
#[path = "blocks/splash_water.rs"]
pub(in crate::game) mod splash_water;
#[cfg(feature = "state-digest")]
#[path = "world/state_digest.rs"]
pub(in crate::game) mod state_digest;
#[path = "interface/stats.rs"]
pub mod stats;
#[path = "administration/structure_blocks.rs"]
pub(in crate::game) mod structure_blocks;
#[path = "mobs/sulfur_cubes.rs"]
pub(in crate::game) mod sulfur_cubes;
#[path = "items/table_enchant.rs"]
pub(in crate::game) mod table_enchant;
#[path = "administration/test_environments.rs"]
pub(in crate::game) mod test_environments;
#[path = "administration/test_instances.rs"]
pub mod test_instances;
#[path = "entities/tnt.rs"]
pub mod tnt;
#[path = "items/trading.rs"]
pub(in crate::game) mod trading;
#[cfg(test)]
#[path = "runtime/transaction_entities_tests.rs"]
pub(in crate::game) mod transaction_entities_tests;
#[path = "runtime/transactions_server.rs"]
pub(in crate::game) mod transactions_server;
#[path = "mobs/trial_spawner.rs"]
pub(in crate::game) mod trial_spawner;
#[path = "items/trident.rs"]
pub(in crate::game) mod trident;
#[path = "items/use_hand.rs"]
pub mod use_hand;
#[path = "items/using_item.rs"]
pub(in crate::game) mod using_item;
#[path = "blocks/vault.rs"]
pub(in crate::game) mod vault;
#[path = "entities/vehicle_teleport.rs"]
pub mod vehicle_teleport;
#[path = "world/view_pass.rs"]
pub mod view_pass;
#[path = "world/villager_professions.rs"]
pub(in crate::game) mod villager_professions;
#[path = "mobs/villagers.rs"]
pub(in crate::game) mod villagers;
#[path = "world/wake_time.rs"]
pub(in crate::game) mod wake_time;
#[path = "mobs/wandering_trader.rs"]
pub(in crate::game) mod wandering_trader;
#[path = "interface/wind.rs"]
pub mod wind;
#[path = "mobs/wither.rs"]
pub(in crate::game) mod wither;
#[path = "blocks/world_fluids.rs"]
pub(in crate::game) mod world_fluids;
#[path = "world/world_loading.rs"]
pub mod world_loading;
#[cfg(any(test, feature = "test-support"))]
#[path = "world/world_session.rs"]
mod world_session;
#[path = "world/world_update_server.rs"]
pub(in crate::game) mod world_update_server;
#[path = "mobs/xp_orb_targeting.rs"]
pub(in crate::game) mod xp_orb_targeting;
#[path = "mobs/zombie_reinforcements.rs"]
pub(in crate::game) mod zombie_reinforcements;
#[path = "mobs/zombie_villagers.rs"]
pub mod zombie_villagers;

#[path = "runtime/agent/mod_server.rs"]
pub(in crate::game) mod agent_server;
#[path = "modding/attachment_sync.rs"]
pub(in crate::game) mod attachment_sync;
#[path = "world/autosave_server.rs"]
pub mod autosave_server;
#[path = "blocks/banners_server.rs"]
pub(in crate::game) mod banners_server;
#[path = "blocks/beacon_server.rs"]
pub(in crate::game) mod beacon_server;
#[path = "mobs/beehives_client.rs"]
pub(in crate::game) mod beehives_client;
#[path = "blocks/block_breaking_server.rs"]
pub(in crate::game) mod block_breaking_server;
#[path = "blocks/block_entity_ticks.rs"]
pub mod block_entity_ticks;
#[path = "blocks/block_entity_ticks_server.rs"]
pub(in crate::game) mod block_entity_ticks_server;
#[path = "blocks/block_items_server.rs"]
pub(in crate::game) mod block_items_server;
#[path = "blocks/block_particles_server.rs"]
pub(in crate::game) mod block_particles_server;
#[path = "mobs/breezes_server.rs"]
pub(in crate::game) mod breezes_server;
#[path = "blocks/brush_server.rs"]
pub(in crate::game) mod brush_server;
#[path = "blocks/campfire_douse_server.rs"]
pub(in crate::game) mod campfire_douse_server;
#[path = "blocks/cartography_server.rs"]
pub(in crate::game) mod cartography_server;
#[path = "blocks/cauldrons_server.rs"]
pub(in crate::game) mod cauldrons_server;
#[path = "interface/chat_common_server.rs"]
pub(in crate::game) mod chat_common_server;
#[path = "blocks/chiseled_bookshelf.rs"]
pub(in crate::game) mod chiseled_bookshelf;
#[path = "world/clocks_server.rs"]
pub(in crate::game) mod clocks_server;
#[path = "entities/collision_source_server.rs"]
pub(in crate::game) mod collision_source_server;
#[path = "administration/commands_common_server.rs"]
pub(in crate::game) mod commands_common_server;
#[path = "runtime/control_server.rs"]
pub(in crate::game) mod control_server;
#[path = "mobs/copper_golems_server.rs"]
pub(in crate::game) mod copper_golems_server;
#[path = "mobs/creatures/mod_client_server.rs"]
pub(in crate::game) mod creatures_client_server;
#[path = "interface/debug_lines_server.rs"]
pub(in crate::game) mod debug_lines_server;
#[path = "blocks/decorated_pot_server.rs"]
pub(in crate::game) mod decorated_pot_server;
#[path = "interface/developer_launch_server.rs"]
pub(in crate::game) mod developer_launch_server;
#[path = "blocks/dispense_bucket.rs"]
pub(in crate::game) mod dispense_bucket;
#[path = "items/durability_server.rs"]
pub(in crate::game) mod durability_server;
#[path = "world/end_client_server.rs"]
pub(in crate::game) mod end_client_server;
#[path = "mobs/enderman.rs"]
pub(in crate::game) mod enderman;
#[path = "modding/entity_sections_server.rs"]
pub(in crate::game) mod entity_sections_server;
#[path = "entities/entity_sounds_server.rs"]
pub(in crate::game) mod entity_sounds_server;
#[path = "entities/fireworks_server.rs"]
pub(in crate::game) mod fireworks_server;
#[path = "items/fishing_client_server.rs"]
pub(in crate::game) mod fishing_client_server;
#[path = "modding/fluid_interactions_server.rs"]
pub(in crate::game) mod fluid_interactions_server;
#[path = "interface/game_mode_common_server.rs"]
pub(in crate::game) mod game_mode_common_server;
#[path = "mobs/ghasts.rs"]
pub(in crate::game) mod ghasts;
#[path = "mobs/ghasts_server.rs"]
pub(in crate::game) mod ghasts_server;
#[path = "blocks/growth_server.rs"]
pub(in crate::game) mod growth_server;
#[cfg(test)]
#[path = "items/hand_part_tests_server.rs"]
pub(in crate::game) mod hand_part_tests_server;
#[path = "items/held_items_server.rs"]
pub(in crate::game) mod held_items_server;
#[path = "items/item_abilities_server.rs"]
pub(in crate::game) mod item_abilities_server;
#[path = "items/item_actions_server.rs"]
pub(in crate::game) mod item_actions_server;
#[path = "runtime/transactions/item_entity_server.rs"]
pub(in crate::game) mod item_entity_server;
#[path = "items/item_entity_spawn_server.rs"]
pub(in crate::game) mod item_entity_spawn_server;
#[path = "administration/jigsaw_blocks_server.rs"]
pub(in crate::game) mod jigsaw_blocks_server;
#[path = "blocks/jukebox_server.rs"]
pub mod jukebox_server;
#[path = "network/lan_actions/mod_server.rs"]
pub mod lan_actions_server;
#[path = "network/lan_mount_menu.rs"]
pub(in crate::game) mod lan_mount_menu;
#[path = "network/lan_mount_menu_server.rs"]
pub(in crate::game) mod lan_mount_menu_server;
#[path = "network/lan_stations.rs"]
pub(in crate::game) mod lan_stations;
#[path = "network/lan_stations_server.rs"]
pub(in crate::game) mod lan_stations_server;
#[path = "network/lan_trade_decisions.rs"]
pub(in crate::game) mod lan_trade_decisions;
#[path = "network/lan_trade_decisions_server.rs"]
pub(in crate::game) mod lan_trade_decisions_server;
#[path = "runtime/loading_server.rs"]
pub mod loading_server;
#[path = "entities/minecart_moves.rs"]
pub(in crate::game) mod minecart_moves;
#[path = "entities/minecart_moves_server.rs"]
pub(in crate::game) mod minecart_moves_server;
#[path = "entities/minecarts_ops_server.rs"]
pub(in crate::game) mod minecarts_ops_server;
#[path = "entities/minecarts_server.rs"]
pub mod minecarts_server;
#[path = "modding/mods/mod_server.rs"]
pub(in crate::game) mod mods_server;
#[path = "interface/names_server.rs"]
pub(in crate::game) mod names_server;
#[path = "modding/native_chunk_queries.rs"]
pub(in crate::game) mod native_chunk_queries;
#[path = "modding/native_command_reads.rs"]
pub(in crate::game) mod native_command_reads;
#[path = "modding/native_components_server.rs"]
pub(in crate::game) mod native_components_server;
#[path = "modding/native_death_weapon.rs"]
pub(in crate::game) mod native_death_weapon;
#[path = "modding/native_entity_extinguish.rs"]
pub(in crate::game) mod native_entity_extinguish;
#[path = "modding/native_entity_selectors_server.rs"]
pub(in crate::game) mod native_entity_selectors_server;
#[path = "modding/native_give_stack_server.rs"]
pub(in crate::game) mod native_give_stack_server;
pub use crate::game::native_hooks as native_hooks_server;
#[path = "modding/native_item_entity_refs_server.rs"]
pub(in crate::game) mod native_item_entity_refs_server;
#[path = "modding/native_knockback_credit_server.rs"]
pub(in crate::game) mod native_knockback_credit_server;
#[path = "modding/native_level_weather.rs"]
pub(in crate::game) mod native_level_weather;
#[path = "modding/native_living_absorption_server.rs"]
pub(in crate::game) mod native_living_absorption_server;
#[path = "modding/native_living_body_server.rs"]
pub(in crate::game) mod native_living_body_server;
#[cfg(test)]
#[path = "modding/native_menu_drop_tests.rs"]
mod native_menu_drop_tests;
#[path = "modding/native_menu_refs_server.rs"]
pub(in crate::game) mod native_menu_refs_server;
#[path = "modding/native_mob_navigation.rs"]
pub mod native_mob_navigation;
#[path = "modding/native_player_attack.rs"]
pub(in crate::game) mod native_player_attack;
#[path = "modding/native_player_death_server.rs"]
pub(in crate::game) mod native_player_death_server;
#[path = "modding/native_player_food_server.rs"]
pub(in crate::game) mod native_player_food_server;
#[path = "modding/native_player_place.rs"]
pub(in crate::game) mod native_player_place;
#[path = "modding/native_portal_destination.rs"]
pub(in crate::game) mod native_portal_destination;
#[path = "modding/native_profiles.rs"]
pub(in crate::game) mod native_profiles;
#[path = "modding/native_recipe_sync.rs"]
pub(in crate::game) mod native_recipe_sync;
#[path = "modding/native_recipes_server.rs"]
pub(in crate::game) mod native_recipes_server;
#[path = "modding/native_scoreboard_teams.rs"]
pub(in crate::game) mod native_scoreboard_teams;
#[path = "modding/native_server_server.rs"]
pub(in crate::game) mod native_server_server;
#[path = "modding/native_ticks_server.rs"]
pub(in crate::game) mod native_ticks_server;
#[path = "modding/native_world_columns_server.rs"]
pub(in crate::game) mod native_world_columns_server;
#[path = "mobs/nautili_server.rs"]
pub(in crate::game) mod nautili_server;
#[path = "interface/pad/mod_server.rs"]
pub(in crate::game) mod pad_server;
#[path = "entities/pick_entity_server.rs"]
pub(in crate::game) mod pick_entity_server;
#[path = "blocks/piston_push/carrying_server.rs"]
pub(in crate::game) mod piston_push_carrying_server;
#[path = "blocks/piston_push_server.rs"]
pub mod piston_push_server;
#[path = "blocks/piston_push/shoving_server.rs"]
pub(in crate::game) mod piston_push_shoving_server;
#[path = "interface/play_keys_server.rs"]
pub(in crate::game) mod play_keys_server;
#[path = "entities/player_equip_ops_server.rs"]
pub(in crate::game) mod player_equip_ops_server;
#[path = "entities/player_equip_server.rs"]
pub(in crate::game) mod player_equip_server;
#[path = "items/potions_server.rs"]
pub mod potions_server;
#[path = "blocks/powder_snow_server.rs"]
pub(in crate::game) mod powder_snow_server;
#[path = "items/primary_items_server.rs"]
pub(in crate::game) mod primary_items_server;
#[path = "interface/profile_capture_server.rs"]
pub(in crate::game) mod profile_capture_server;
#[path = "items/recipe_book_server.rs"]
pub(in crate::game) mod recipe_book_server;
#[path = "network/remote/mod_server.rs"]
pub mod remote_server;
#[path = "network/remote_tags_server.rs"]
pub(in crate::game) mod remote_tags_server;
#[path = "runtime/repro_server.rs"]
pub(in crate::game) mod repro_server;
#[path = "world/saving_server.rs"]
pub(in crate::game) mod saving_server;
#[path = "interface/shader_packs_server.rs"]
pub(in crate::game) mod shader_packs_server;
#[path = "mobs/sheep_dye_server.rs"]
pub(in crate::game) mod sheep_dye_server;
#[path = "blocks/shelf_server.rs"]
pub(in crate::game) mod shelf_server;
#[path = "mobs/sniffers_server.rs"]
pub(in crate::game) mod sniffers_server;
#[path = "items/spear_server.rs"]
pub(in crate::game) mod spear_server;
#[path = "modding/state_resolver_server.rs"]
pub(in crate::game) mod state_resolver_server;
#[path = "blocks/stations_server.rs"]
pub(in crate::game) mod stations_server;
#[path = "world/storage_server.rs"]
pub(in crate::game) mod storage_server;
#[path = "administration/test_blocks.rs"]
pub(in crate::game) mod test_blocks;
#[path = "administration/test_entities_server.rs"]
pub(in crate::game) mod test_entities_server;
#[path = "entities/text_displays_server.rs"]
pub(in crate::game) mod text_displays_server;
#[path = "interface/titles_server.rs"]
pub(in crate::game) mod titles_server;
#[path = "items/totem_server.rs"]
pub(in crate::game) mod totem_server;
#[path = "world/weather_server.rs"]
pub(in crate::game) mod weather_server;
#[path = "world/world_entry_server.rs"]
pub(in crate::game) mod world_entry_server;
#[path = "world/world_ops_server.rs"]
pub(in crate::game) mod world_ops_server;

/// An active player break waiting on `break_block` listeners. When a nested player break completes removal at the same `(dim, pos)`, `removed_by_nested` is set so the outer break does not apply removal effects again even if a non-air block or fluid remains (such as fewer turtle eggs or water).
pub(in crate::game) struct ActivePlayerBreak {
    pub(in crate::game) dim: Dim,
    pub(in crate::game) pos: [i32; 3],
    pub(in crate::game) removed_by_nested: bool,
}

pub struct ServerState {
    /// Wind bursts from a guest's turn, waiting for it to end (game/interface/wind.rs).
    pub(in crate::game::server_side) wind_bursts: Vec<crate::game::wind::Burst>,
    /// The wind charge whose burst is being handled, by entity id: which
    /// entities have already ticked in vanilla's spawn-ordered tick list
    /// (game/interface/wind.rs).
    pub(in crate::game::server_side) wind_source: Option<sunrise_sim::join_seq::JoinSeq>,
    /// Wind pushes held until the entity's own tick has run, by entity id
    /// (game/interface/wind.rs `apply_late_wind_pushes`).
    pub(in crate::game::server_side) late_wind_pushes:
        Vec<(u32, sunrise_gameplay_common::game::Vec3)>,
    /// The players a fishing hook can catch, taken as the hooks tick, and
    /// the pulls on them waiting for the hub (game/items/fishing.rs).
    pub(in crate::game::server_side) hook_targets: Vec<crate::game::fishing::HookTarget>,
    pub(in crate::game::server_side) hook_pulls: Vec<(u32, sunrise_core::geometry::Vec3)>,
    /// Mace smash knockback on the other players from a guest's turn, waiting
    /// for it to end (game/items/mace.rs).
    pub(in crate::game::server_side) smash_pushes: Vec<crate::game::mace::SmashPush>,
    /// Sweep attacks from a guest's turn, waiting for it to end
    /// (game/network/lan_pvp.rs).
    pub(in crate::game::server_side) sweeps: Vec<crate::game::lan_pvp::Sweep>,
    /// The wandering trader spawner's clock.
    pub(in crate::game::server_side) trader: sunrise_gameplay::wandering_trader::Spawner,
    /// The cat spawner (`CatSpawner`): its countdown.
    pub cat: sunrise_gameplay::cat_spawner::CatSpawner,
    /// Nether portals: the player's, items' and mobs' (game/blocks/nether_portal.rs).
    pub portals: nether_portal::Portals,
    pub(in crate::game::server_side) hold_server: bool,
    pub(in crate::game::server_side) collider_histories: native_colliders::Histories,
    /// This world is open to the network (game/lan.rs).
    pub lan: Option<lan::LanHost>,
    /// This game is that server thread's (game/world/integrated.rs).
    pub serving: Option<integrated::Serving>,
    pub(in crate::game::server_side) dedicated_services: Option<dedicated::Services>,
    pub streaming: sunrise_level::chunk_streamer::ChunkStreamer,
    pub be_anims: sunrise_level::block_entity_ticks::BlockEntityTicks,
    pub settings: sunrise_level::sim_settings::SimSettings,
    pub(in crate::game) profile: crate::game::server_side::profile_capture_server::ServerProfile,
    /// Contact hurts a vehicle tick queued; applied once its copy is stored.
    pub(in crate::game) vehicle_contact_hurts: Vec<crate::game::entity_server::ContactHurt>,
    /// The registries native mods registered data map types for, as of the
    /// server tables' last build: what `data_maps_updated` fires for.
    pub(in crate::game::server_side) data_map_registries: Vec<String>,
    /// The crafting table the 3x3 grid belongs to (`CraftingMenu.access`).
    pub craft_table: Option<[i32; 3]>,
    /// The open container and the slots it had when it opened:
    /// `Container.stillValid` holds only while every block entity the menu
    /// took is still there, so a half of a double chest (or the container)
    /// going away closes it.
    pub open_container_slots: Option<((i32, i32, i32), usize)>,
    pub destroy_delay: u32,
    /// The prepared world is installed. The session lock is taken earlier so tag
    /// reads see ready tables, and is not this marker: a final save before the
    /// prepared `Level` is installed would replace the world with placeholder state.
    pub world_adopted: bool,
    /// An early shutdown must not replace on-disk tickets while the server is
    /// still publishing levels or activating their tickets.
    pub(in crate::game::server_side) startup_ticket_restore_pending: bool,
    pub(crate) ticket_activation: Option<&'static mod_api::__private::game::TicketActivation>,
    /// The lethal damage type recorded while a lifted dropped item died with a listening `ItemDestroyed` hook, consumed by its caller once `dropped` is put back.
    pub(in crate::game) item_destroyed_damage: Option<(u32, String)>,
    pub next_entity: u32,
    /// The last entity id when this level's entity pass began: an item with a
    /// later one was added during that pass and first ticks on the next
    /// (`EntityTickList`). `u32::MAX` until a tick has run.
    pub ticked_entities_through: u32,
    /// The items `tick_blocks` dropped while applying the entities' block
    /// contacts (a boat breaking a lily pad). Vanilla runs those in each
    /// entity's own tick (`applyEffectsFromBlocks`), inside the entity pass,
    /// so they too first tick on the next tick.
    pub(in crate::game::server_side) contact_drops: std::ops::Range<u32>,
    /// Lecterns whose page-turn pulse is running (ticks left).
    pub(in crate::game::server_side) lectern_pulses: Vec<((i32, i32, i32), i32)>,
    /// The world spawn search's origin chunk and fallback height, once known.
    pub(in crate::game::server_side) spawn_origin: Option<((i32, i32), i32)>,
    /// `native_create_spawn_position` was asked, and a mod chose the spawn.
    pub(in crate::game::server_side) spawn_hook_fired: bool,
    pub(in crate::game::server_side) spawn_hook_cancelled: bool,
    pub(in crate::game::server_side) spawn_spiral: sunrise_sim::world_spawn::ChunkSpiral,
    /// A new player's spawn search, while its candidates' chunks load.
    pub(in crate::game::server_side) player_spawn:
        Option<sunrise_sim::world_spawn::PlayerSpawnFinder>,
    /// Chunks a respawn search waits for this tick, kept loading beside the
    /// player's own (taken by `update_world`).
    pub(in crate::game::server_side) spawn_wait: Vec<(i32, i32)>,
    pub attack_ticks: i32,
    /// The main hand's item at the last player tick, whose attack damage and
    /// speed modifiers apply (`LivingEntity.detectEquipmentUpdates`): a hotbar
    /// swap just before an attack keeps them, with the new item's enchantments
    /// (attribute swapping).
    pub(in crate::game::server_side) attack_item: Option<Arc<str>>,
    /// Mobs a player hurt (`LivingEntity.lastHurtByPlayer`), by player UUID.
    pub last_hurt_by_player: HashMap<u32, combat_common::PlayerCredit>,
    /// Leashed mobs a player killed, with the killer's UUID: the lead drops
    /// on the corpse's next tick (`Leashable.tickLeash`), for the mods. With
    /// the corpse's dimension: each level's tick prunes only its own.
    pub(in crate::game::server_side) lead_killers: HashMap<u32, (u128, Dim)>,
    /// Animals a player fed into love (`Animal.loveCause`), with the guest it was by
    /// connection (None: the host). Breeding credits them, wherever they are.
    pub love_cause: HashMap<u32, Option<u32>>,
    /// Mobs whose death (`LivingEntity.die`) has been handled.
    /// Looked up per mob each tick: a fast hasher (never iterated).
    pub died: sunrise_level::fast_hash::FxHashSet<u32>,
    /// The crossbow charge's start and middle loading sounds played (`game/items/crossbow.rs`):
    /// each charge plays each once, however far a mod's tick moved the charge at a time.
    pub(in crate::game::server_side) crossbow_sounds: [bool; 2],
    /// The shield's item cooldown after an axe disabled it (ticks).
    pub shield_cooldown: u32,
    /// The source of a command run through /execute (the player otherwise).
    pub command_source: Option<commands::execute::Source>,
    pub(in crate::game::server_side) native_command_filter: bool,
    /// Public chat broadcasts a `chat_recipient` listener's commands start while one runs.
    pub(in crate::game) chat_broadcasts: lan_chat::ChatBroadcasts,
    /// `MinecraftServer.ticksUntilAutosave`: the ticks left before the next
    /// autosave, counted down a tick at a time.
    pub ticks_until_autosave: i64,
    pub(in crate::game::server_side) applied_modifiers: Vec<(String, String)>,
    pub(in crate::game::server_side) xp_delay: u32,
    pub ordered: ordered_tick::OrderedTicks,
    pub level_up_sound: sunrise_sim::entities::LevelUpSound,
    /// Swings of uses whose client predicts them (`SwingSource.PREDICTED`):
    /// a server does not send these back to the player who swung.
    pub(in crate::game::server_side) predicted_swings: u32,
    /// The beds players sleep in that this game marked `occupied`, by level
    /// (game/blocks/beds.rs).
    pub player_beds: Vec<(Dim, [i32; 3])>,
    /// `SleepStatus`'s active and sleeping players at the last announcement.
    pub(in crate::game::server_side) sleep_seen: (i64, i64),
    pub world_root: PathBuf,
    pub(in crate::game::server_side) carrier_retention_snapshot: Vec<(u128, u32)>,
    /// Offline-to-account UUID migration: loaded entities retain the old UUID until saved.
    pub(in crate::game::server_side) owner_migration: Option<(u128, u128)>,
    /// The world is the Debug preset (`Level.isDebug`, for every dimension): its blocks
    /// cannot be changed and nothing ticks.
    pub world_debug: bool,
    /// A server's players on their way to another level, each waiting for
    /// the destination to load around them.
    pub server_trips: Vec<server_travel::Trip>,
    /// Players a command sends to another level, once it has run.
    pub level_teleports: Vec<server_travel::LevelTeleport>,
    /// Teleports a command made inside a level, for `native_player_teleported`
    /// once the player's move has reached their client (the put-back at the end of `guest_turn`).
    pub(in crate::game) teleport_notes: Vec<server_travel::TeleportNote>,
    /// Which pass of a server tick runs (`tick_server`): the first one reads
    /// the network, the last one ends the players' tick.
    pub level_pass: levels::Pass,
    /// The respawn point when it is not the world spawn or a bed: a respawn
    /// anchor (`game/blocks/respawn_anchor.rs`).
    pub respawn_config: Option<respawn_anchor::RespawnConfig>,
    /// The next enter_dimension call comes from an End portal.
    pub(in crate::game::server_side) end_portal_travel: bool,
    pub(in crate::game::server_side) tick_scratch: TickScratch,
    pub(in crate::game) menu_publications: HashMap<u32, lan_menus::MenuPublications>,
    /// Negative staged-open identity counter, separate from live protocol IDs.
    pub(in crate::game) staged_menu_open: i32,
    /// Linked containers from a mod menu’s `menu_links`, deduplicated in first-seen order each tick.
    pub(in crate::game) link_scratch: lan_crafting::LinkScratch,
    /// Reusable visited-cell set for `sweep_item_inside` and `sweep_projectile_inside`.
    pub(in crate::game) inside_visited_scratch: sunrise_level::fast_hash::FxHashSet<i64>,
    pub(in crate::game) natural_admission: Option<natural_spawning::NaturalAdmission>,
    pub tick_chunks: sunrise_sim::tick_chunk_membership::TickChunkMembership,
    /// Worn armor as of the last equipment check (frost walker).
    pub(in crate::game::server_side) location_armor: [Option<ItemStack>; 4],
    /// The boats a mob's lead can hang on, for this tick's mob ticks.
    pub(in crate::game::server_side) leash_boats: Vec<(u32, sunrise_sim::mobs::leash::HolderState)>,
    /// The boats and minecarts `native_vehicle_moved` follows (lan_vehicles.rs).
    pub(in crate::game::server_side) watched_vehicles: Vec<lan_vehicles::WatchedVehicle>,
    /// The ender dragon fight (game/mobs/dragon.rs).
    pub fight: dragon::Fight,
    /// The entity ids a hosted server gives the knots its mobs hang on, by
    /// block, while a lead is tied there.
    pub knot_ids: HashMap<[i32; 3], u32>,
    /// Fence posts whose natural knot was killed and whose leads drop next tick.
    pub broken_knots: Vec<[i32; 3]>,
    /// This window's player's place in `riding`'s passengers, as `Guest.vehicle_order` is a guest's.
    pub(in crate::game::server_side) riding_order: i64,
    /// Vehicles that gained a rider this tick whose `START_RIDING_TRIGGER` was
    /// not fired yet (`game/entities/boarding.rs`), for a guest's turn.
    pub(in crate::game::server_side) started_riding: Vec<u32>,
    /// A rotation-only write (`/rotate`, `Entity.forceSetRotation`) to the player of this
    /// turn: the put-back at the end of `guest_turn` sends it as `ClientboundPlayerRotationPacket`, not a teleport.
    pub(in crate::game::server_side) rotation_forced: bool,
    /// The vehicle the player's file names, waiting for it to be in the world
    /// (`ServerPlayer.loadAndSpawnParentVehicle`).
    pub(in crate::game::server_side) pending_vehicle: Option<[i32; 4]>,
    /// The ender inventory (27 slots) and the ender chest showing it.
    pub ender_items: Vec<Option<ItemStack>>,
    pub ender_open: Option<(i32, i32, i32)>,
    pub(in crate::game::server_side) creaking: creaking::CreakingState,
    /// The active level's stored `ServerChunkCache.spawnEnemies` setting.
    pub(in crate::game) chunk_spawn_enemies: bool,
    /// Reused candidate buffers for natural spawning.
    pub(in crate::game::server_side) spawn_scratch: natural_spawning::SpawnScratch,
    pub hoppers: HashMap<(i32, i32, i32), sunrise_gameplay_common::hoppers::HopperState>,
    /// The source of the running dispenser behaviour, including across mod callbacks.
    pub(in crate::game::server_side) dispense_source: Option<dispenser::DispenseSource>,
    /// Open mod menus whose `menu_slot_changed` dispatch is currently on the call stack.
    pub(in crate::game::server_side) slot_changed_active: Vec<slot_changes::SlotChangedTarget>,
    /// Backing blocks whose contents notice is on the call stack, innermost last.
    pub contents_changing: Vec<(mod_api::LevelRef, BlockPos)>,
    /// The players with each mod container open, by level (`tick_mod_container_viewers`).
    pub mod_container_viewers:
        HashMap<(&'static str, (i32, i32, i32)), mod_container_openers::ModContainerOpeners>,
    /// Where item entities are, while the block entities tick (hoppers).
    pub item_columns: Option<sunrise_gameplay_common::hoppers::ItemColumns>,
    /// The item columns for one pass of the players' pickups
    /// (`tick_guests`), rebuilt after any write to the items (a pickup, a
    /// command, an explosion): see `item_columns`.
    pub(in crate::game::server_side) pickup_columns:
        Option<sunrise_gameplay_common::hoppers::StampedItemColumns>,
    /// Where the mobs are, while the block entities tick (potent sulfur gas).
    pub pass_mobs: Option<potent_sulfur::PassMobs>,
    /// Entities whose copies are held out of storage; their handles refuse until stored back.
    pub held_out: Vec<u32>,
    /// What the boats' and minecarts' ticks know about the mobs: their box reach
    /// (`vehicle_mob_reach`) and a grid of their boxes (`mobs_meeting`).
    pub vehicle_mobs: boat_passengers::VehicleMobs,
    /// Missing profession IDs already reported for this world session.
    pub missing_professions: HashSet<String>,
    /// What changed in entities' colliders this tick, for the players who track them.
    pub(in crate::game::server_side) collider_changes: Vec<native_colliders::ColliderChange>,
    /// The shapes of the open world's levels, from its dimension types.
    pub level_shapes: sunrise_level::chunk::LevelShapes,
    /// `IntegratedServer`'s single-player profile, fixed when the server thread is created,
    /// before its startup hooks (game/modding/native_singleplayer_owner.rs). None elsewhere.
    pub singleplayer_profile: Option<native_singleplayer_owner::SingleplayerProfile>,
    /// `StructureTemplateManager.structureRepository`: the templates in memory
    /// by id, a miss included (game/administration/structure_blocks.rs).
    pub structure_templates: HashMap<String, Option<structure_blocks::TemplateEntry>>,
    pub(in crate::game::server_side) structure_spawns:
        sunrise_sim::mobs::spawn_selection::StructureSpawns,
    /// `ServerChunkCache.lastSpawnState` of each level: its spawnable chunks
    /// and each `MobCategory`'s count.
    pub(in crate::game::server_side) spawn_counts: HashMap<Dim, (i32, [i32; 8])>,
    /// The natural spawning policy mods set on each level (`LevelRef::set_spawn_limits`,
    /// `set_spawn_flags`); a level with none spawns as vanilla. Kept while the world is open.
    pub(in crate::game::server_side) spawn_policies:
        HashMap<mod_api::DimensionKey, natural_spawning::SpawnPolicy>,
    /// Non-blocking server chunk load/generation requests (`native_chunk_requests.rs`).
    pub(in crate::game) async_chunk_requests: native_chunk_requests::AsyncChunkRequests,
    /// Open-state flags for `SavedChunkReader` handles per dimension (`native_saved_chunks.rs`).
    pub(in crate::game) saved_chunk_readers:
        std::cell::RefCell<HashMap<Dim, Arc<std::sync::atomic::AtomicBool>>>,
    /// Open detached chunk generators handed out by `LevelRef::detached_generator`.
    pub(in crate::game) detached_generators: native_detached_generation::DetachedGenerators,
    /// Scheduled end of the server tick (`MinecraftServer.haveTime`).
    pub server_tick_deadline: Option<Instant>,
    /// The block a guest is acting on (game/network/lan_actions/interaction.rs).
    pub forced_hit: Option<Hit>,
    /// The part of a right click a guest's packet asks for (game/network/lan_actions/interaction.rs).
    pub guest_use: Option<GuestUse>,
    /// The last `use_hand` that ended the click did so with `FAIL`, which
    /// does not consume the action (handleUseItemOn's ANY_BLOCK_USE).
    pub use_failed: bool,
    pub block_use_original: Vec<(Hand, Option<ItemStack>)>,
    /// Level events of this tick for the guests, with their excluded source.
    pub lan_events: Vec<QueuedLevelEvent>,
    /// Jukebox level events 1010 (song name) and 1011 (`None`) for the guests;
    /// the song's network id is only known once the hub is in hand.
    pub lan_songs: Vec<([i32; 3], Option<String>)>,
    pub(in crate::game::server_side) lan_cushion_breaks:
        Vec<(sunrise_world::registry::BlockStateId, [f64; 3])>,
    /// Block events (position, b0, b1, state) and entity events (id, event) for the guests.
    pub lan_block_events: Vec<([i32; 3], i32, i32, u32)>,
    pub lan_entity_events: Vec<(i32, i8)>,
    /// Successful crits (attacking guest, hit entity), queued while the hub is taken.
    pub(in crate::game::server_side) lan_critical_hits: Vec<(Option<u32>, i32, bool)>,
    /// Spear stabs that landed (the stabbing guest), queued while the hub is
    /// taken: `broadcastEntityEvent(player, 2)`.
    pub(in crate::game::server_side) lan_kinetic_hits: Vec<Option<u32>>,
    /// `LivingEntity.recentKineticEnemies`: when each player (a guest, or the
    /// window's own) last stabbed each entity this use.
    pub(in crate::game::server_side) stabbed: HashMap<(Option<u32>, u32), i64>,
    /// The other players the acting player's spear can reach this turn,
    /// taken from the hub before it (`spear_players`).
    pub(in crate::game::server_side) stab_players: Vec<spear_players::StabPlayer>,
    /// Spear stabs on players worked out in a guest's turn, landed once the
    /// hub is back (`land_player_stabs`).
    pub(in crate::game::server_side) player_stabs: Vec<spear_players::StabBatch>,
    /// The movement a guest's client reported this tick
    /// (`ServerPlayer.getKnownMovement`), for their turn.
    pub(in crate::game::server_side) known_movement: Vec3,
    /// Global level events for every guest (`ServerLevel.globalLevelEvent`), with their level.
    pub lan_global_events: Vec<(&'static str, i32, [i32; 3], i32)>,
    /// `ContainerOpenersCounter`: how many players (the host and the guests)
    /// have each chest open.
    pub lid_openers: HashMap<(i32, i32, i32), i32>,
    pub lan_acting: Option<u32>,
    /// The abilities of the server player whose turn it is (`as_guest`): their record's
    /// copy, which the turn reads and writes and the put-back at the end of `guest_turn` stores. None outside a turn.
    pub(in crate::game::server_side) acting_abilities:
        Option<sunrise_sim::lan::abilities::PlayerAbilities>,
    /// A guest's `/save-all`: saved once their turn ends (`run_guest_command`),
    /// when their own state is back in their record.
    pub(in crate::game::server_side) save_after_guest_turn: bool,
    /// Guards `begin_world_save` against re-entry while `world_save_start` runs.
    pub(in crate::game::server_side) world_save_starting: bool,
    /// Guests whose records are displaced by nested player swaps, outer first.
    pub lan_guest_swaps: Vec<u32>,
    /// Connections whose turn a panic interrupted this tick: the turn helpers record the
    /// player before re-raising, the end-of-turn path records it before its callbacks run and
    /// clears it after a clean put-back. Cleared at the start of every server tick; the crash
    /// save leaves these players' files as last saved (`server_lifecycle`).
    pub interrupted_turns: Vec<u32>,
    /// What teardown decided about the interrupted turns, for the writes after it.
    pub crash_save: server_lifecycle::CrashSave,
    /// Ender chests a mod opened for a guest inside that guest's turn
    /// (`player_open_ender_chest`), by connection, with their titles: opened as the turn
    /// ends, once the guest's own ender chest is back in their record.
    pub(in crate::game::server_side) lan_ender_menus: Vec<(u32, sunrise_core::nbt::Tag)>,
    /// Active player breaks waiting on `break_block` listeners.
    pub(in crate::game) active_player_breaks: Vec<ActivePlayerBreak>,
    /// The name of the guest `lan_acting` acts for (`Player.getName`).
    pub lan_acting_name: Option<String>,
    /// Their UUID (`Player.getUUID`).
    pub lan_acting_uuid: Option<u128>,
    /// Where the mods refused to prime TNT: the sound and game event the level
    /// queued right after the prime are dropped (`prime_tnt_event`).
    pub(in crate::game::server_side) tnt_refused: Option<BlockPos>,
    /// Their permission level: an op's above 0 (`PlayerList.isOp`).
    pub lan_acting_permission: Option<u8>,
    /// The spawn protection radius that holds the acting guest back
    /// (`DedicatedServer.isUnderSpawnProtection` before the position test):
    /// None for an operator, without operators, or off a dedicated server.
    pub lan_acting_spawn_protection: Option<i32>,
    /// Their entity id, which the hub (often taken out of `lan` while they
    /// act) would otherwise have to be asked for.
    pub lan_acting_entity: Option<u32>,
    /// Whether the player currently swapped into the game is connectionless.
    pub lan_acting_fake: bool,
    /// Command feedback to forward to a guest.
    pub chat_capture: Option<Vec<sunrise_game_data::text::Text>>,
    /// Packets for the guest acting now, sent once its action returns.
    pub guest_packets: Vec<sunrise_protocol::packets::play::Clientbound>,
    /// The acting guest's cooldowns, as (item, ticks): their packet needs the
    /// hub's item registry, which the guest's turn does not hold.
    pub guest_cooldowns: Vec<(Arc<str>, i32)>,
    /// The acting guest's own entity events (`ServerPlayer.connection.send`),
    /// sent with their entity id, which the guest's turn does not hold.
    pub guest_entity_events: Vec<i8>,
    /// The acting guest's equipment breaks (`onEquippedItemBroken`'s entity
    /// events), broadcast with their entity id once their turn ends.
    pub guest_equipment_breaks: Vec<i8>,
    /// Block sounds queued by the acting guest, excluding that guest.
    pub(in crate::game::server_side) guest_block_sounds: Vec<(String, [f64; 3], f32, f32)>,
    /// `Player.playSound` of the acting guest (event, position, volume,
    /// pitch): `level.playSound(player, ...)` leaves the guest out, whose
    /// own client plays it.
    pub guest_player_sounds: Vec<(String, [f64; 3], f32, f32)>,
    /// Sounds bound to an entity this tick, for the players (`game/interface/level_sounds.rs`).
    pub bound_sounds: Vec<level_sounds::BoundSound>,
    /// `broadcastToAdmins` lines for the other operators that a command queued,
    /// sent once it returns (`send_admin_feedback`), each with the guest whose
    /// command it was (`commandSource() != source`): None for a block, a
    /// minecart, the console or the host.
    pub admin_feedback: Vec<(sunrise_game_data::text::Text, Option<u32>)>,
    /// The server console is running a command: whether its feedback goes to
    /// the operators (`DedicatedServer.shouldInformAdmins`,
    /// `broadcast-console-to-ops`).
    pub console_broadcast: Option<bool>,
    /// The command the console is running came over RCON: its source is named `Rcon`
    /// (`RconConsoleSource.createCommandSourceStack`), not `Server`.
    pub rcon_source: bool,
    pub bosses: bossbars::BossState,
    pub rand_sequences: sunrise_items::random_sequences::RandomSequences,
    /// `BrushableBlockEntity`'s brush counts of the blocks being brushed.
    pub(in crate::game::server_side) brushing: brush::Brushing,
    pub(in crate::game::server_side) restore_player_state: bool,
    pub(in crate::game::server_side) acting_input: acting_state::ServerActingInput,
    pub(in crate::game::server_side) pending_guest_ui: acting_state::ServerPendingGuestUi,
    pub audio_capture_active: bool,
    pub pending_sounds: Vec<sunrise_game_data::sound_source::ServerSound>,
    #[cfg(any(test, feature = "test-support"))]
    pub(in crate::game) test_positional_sound_counts: std::collections::HashMap<String, u32>,
    pub pending_particles: Vec<sunrise_sim::lan::server_effects::ServerQueuedParticle>,
    pub(in crate::game) pending_explosions: Vec<sunrise_sim::lan::server_effects::ServerExplosion>,
    pub pending_native_sounds: Vec<sunrise_gameplay_common::game::creatures::NativeSound>,
    pub effect_rng: sunrise_core::random::LegacyRandom,
    pub sculk: sculk_server::SculkState,

    pub save: ServerShared<Arc<SaveDir>>,
    /// Must precede session_lock: dropping the writer drains writes while the world is locked.
    pub(in crate::game::server_side) player_autosaves:
        ServerOnly<sunrise_sim::lan::playerdata::Autosaves>,
    pub session_lock: ServerShared<Option<sunrise_sim::save::SessionLock>>,
    pub paused_mob_hurt: ServerOnly<Option<Box<sunrise_sim::mobs::PausedMobHurt>>>,
    pub raiding: ServerOnly<raids::Raiding>,
    /// The world's data packs' content tables, when it has packs (`None`
    /// keeps `Resources.rest`'s own in use).
    pub world_tables: ServerShared<Option<WorldTables>>,
    /// Server-owned loot and item snapshots, independent of client preloads.
    pub server_loot: ServerOnly<sunrise_sim::structure_blocks::LootSnapshot>,
    /// A server's levels other than the one in place (game/world/levels.rs).
    pub levels: ServerOnly<levels::Levels>,
    pub tickets: ServerShared<sunrise_sim::ticket_levels::ChunkTickets>,
    pub(in crate::game::server_side) random_tick_sections:
        ServerOnly<sunrise_sim::random_tick_sections::RandomTickSections>,
    /// Entities of types Sunrise does not simulate.
    pub others: ServerShared<sunrise_sim::join_seq::ResidentList<commands::OtherEntity>>,
    /// Item entities spawned and block entities removed while the block
    /// behaviour ran (spilled contents, popped resources).
    pub spill: ServerOnly<spill_common::Spill>,
    pub creative_catalog: ServerOnly<Arc<sunrise_items::creative_catalog::Catalog>>,
    /// `GameTestTicker.SINGLETON`: the game tests running on this server.
    pub game_tests: ServerOnly<game_tests::GameTests>,
    /// A mod asked for the command trees again (`native_refresh_commands`):
    /// the next server tick asks every mod for its commands and resends them.
    pub native_commands:
        ServerOnly<sunrise_gameplay_common::game::native_server::NativeCommandState>,
    /// Other players in the current guest turn, including borrowed ancestor bodies.
    pub lan_building_boxes: ServerOnly<building_players::BuildingPlayers>,
    /// Bodies and their statistics displaced by guest turns, outer first.
    pub(in crate::game::server_side) lan_displaced_players:
        ServerOnly<Vec<lan_actions_server::DisplacedPlayer>>,
    /// A fake player's `perform_action` in progress, and what it refused.
    pub fake_action:
        ServerOnly<std::sync::Arc<sunrise_gameplay_common::game::native_server::FakeAction>>,
    /// A dedicated server's operator, whitelist and ban lists, for `/op`,
    /// `/ban` and friends (`DedicatedPlayerList`).
    pub server_lists: ServerOnly<Option<std::sync::Arc<std::sync::Mutex<dedicated::Lists>>>>,
    /// `Services.nameToIdCache`: the server thread's usercache.json.
    pub(in crate::game) user_cache: Option<sunrise_server_files::user_cache::UserCache>,
    /// State of `/spectate`, `/posteffect`, `/debug`, `/waypoint` and friends.
    pub cmds: ServerOnly<commands::extra::CommandExtras>,
    /// The retained server reload listeners of the last successful reload.
    pub(in crate::game::server_side) reload_resources: ServerOnly<mod_api::RetainedListeners>,
    pub combat: ServerOnly<combat::CombatTracker>,
    /// Each guest's own `CombatTracker`, out of `combat` between turns.
    pub guest_combat: ServerOnly<std::collections::HashMap<u32, combat::CombatTracker>>,
    pub fns: ServerOnly<functions::FnState>,
    pub exec: ServerOnly<commands::execute::ExecState>,
    pub(in crate::game::server_side) profile_resolver:
        Box<dyn sunrise_level::profile_lookup::ProfileResolver>,
}

impl ServerState {
    // Kept out-of-line so its field temporaries do not bloat `ServerGame::new`'s unoptimised frame.
    #[inline(never)]
    pub(in crate::game) fn build(
        (streaming, be_anims, settings, profile): (
            sunrise_level::chunk_streamer::ChunkStreamer,
            sunrise_level::block_entity_ticks::BlockEntityTicks,
            sunrise_level::sim_settings::SimSettings,
            crate::game::server_side::profile_capture_server::ServerProfile,
        ),
        structure_spawns: sunrise_sim::mobs::spawn_selection::StructureSpawns,
        profile_resolver: Box<dyn sunrise_level::profile_lookup::ProfileResolver>,
        save: ServerShared<Arc<SaveDir>>,
        creative_catalog: ServerOnly<Arc<sunrise_items::creative_catalog::Catalog>>,
        ws: Box<super::WrappedServerInit>,
    ) -> Box<Self> {
        Box::new(Self {
            wind_bursts: Default::default(),
            wind_source: Default::default(),
            late_wind_pushes: Default::default(),
            hook_targets: Default::default(),
            hook_pulls: Default::default(),
            smash_pushes: Default::default(),
            sweeps: Default::default(),
            trader: Default::default(),
            cat: Default::default(),
            portals: Default::default(),
            hold_server: false,
            collider_histories: Default::default(),
            lan: None,
            serving: None,
            dedicated_services: None,
            streaming,
            be_anims,
            settings,
            profile,
            structure_spawns,
            vehicle_contact_hurts: Vec::new(),
            data_map_registries: Vec::new(),
            craft_table: None,
            open_container_slots: None,
            destroy_delay: 0,
            world_adopted: false,
            startup_ticket_restore_pending: false,
            ticket_activation: None,
            item_destroyed_damage: None,
            next_entity: 0,
            ticked_entities_through: u32::MAX,
            contact_drops: 0..0,
            lectern_pulses: Vec::new(),
            spawn_origin: None,
            spawn_hook_fired: false,
            spawn_hook_cancelled: false,
            spawn_spiral: Default::default(),
            player_spawn: None,
            spawn_wait: Vec::new(),
            attack_ticks: 100,
            attack_item: None,
            last_hurt_by_player: HashMap::new(),
            lead_killers: HashMap::new(),
            love_cause: HashMap::new(),
            died: Default::default(),
            crossbow_sounds: [false; 2],
            shield_cooldown: 0,
            command_source: None,
            native_command_filter: false,
            chat_broadcasts: Default::default(),
            // `computeNextAutosaveInterval` at the clock's 20 ticks a second.
            ticks_until_autosave: 6000,
            applied_modifiers: Vec::new(),
            xp_delay: 0,
            ordered: Default::default(),
            level_up_sound: Default::default(),
            predicted_swings: 0,
            player_beds: Vec::new(),
            sleep_seen: (0, 0),
            world_root: PathBuf::new(),
            carrier_retention_snapshot: Vec::new(),
            owner_migration: None,
            world_debug: false,
            server_trips: Vec::new(),
            level_teleports: Vec::new(),
            teleport_notes: Vec::new(),
            level_pass: Default::default(),
            respawn_config: None,
            end_portal_travel: false,
            tick_scratch: TickScratch::default(),
            menu_publications: HashMap::new(),
            staged_menu_open: 0,
            link_scratch: Default::default(),
            inside_visited_scratch: Default::default(),
            natural_admission: None,
            tick_chunks: Default::default(),
            location_armor: Default::default(),
            leash_boats: Vec::new(),
            watched_vehicles: Vec::new(),
            fight: dragon::Fight::default(),
            knot_ids: Default::default(),
            broken_knots: Vec::new(),
            riding_order: 0,
            started_riding: Vec::new(),
            rotation_forced: false,
            pending_vehicle: None,
            ender_items: vec![None; 27],
            ender_open: None,
            creaking: Default::default(),
            chunk_spawn_enemies: true,
            spawn_scratch: Default::default(),
            hoppers: HashMap::new(),
            dispense_source: None,
            slot_changed_active: Vec::new(),
            contents_changing: Vec::new(),
            mod_container_viewers: HashMap::new(),
            item_columns: None,
            pickup_columns: None,
            pass_mobs: None,
            held_out: Vec::with_capacity(4),
            vehicle_mobs: Default::default(),
            missing_professions: Default::default(),
            collider_changes: Vec::new(),
            level_shapes: Default::default(),
            singleplayer_profile: None,
            structure_templates: HashMap::new(),
            spawn_counts: HashMap::new(),
            spawn_policies: HashMap::new(),
            async_chunk_requests: Default::default(),
            saved_chunk_readers: Default::default(),
            detached_generators: Default::default(),
            server_tick_deadline: None,
            forced_hit: None,
            guest_use: None,
            use_failed: false,
            block_use_original: Vec::new(),
            lan_events: Vec::new(),
            lan_songs: Vec::new(),
            lan_cushion_breaks: Vec::new(),
            lan_block_events: Vec::new(),
            lan_entity_events: Vec::new(),
            lan_critical_hits: Vec::new(),
            lan_kinetic_hits: Vec::new(),
            stabbed: HashMap::new(),
            stab_players: Vec::new(),
            player_stabs: Vec::new(),
            known_movement: Vec3::ZERO,
            lan_global_events: Vec::new(),
            lid_openers: HashMap::new(),
            lan_acting: None,
            acting_abilities: None,
            save_after_guest_turn: false,
            world_save_starting: false,
            lan_guest_swaps: Vec::new(),
            interrupted_turns: Vec::new(),
            crash_save: Default::default(),
            lan_ender_menus: Vec::new(),
            guest_packets: Vec::new(),
            guest_cooldowns: Vec::new(),
            guest_entity_events: Vec::new(),
            guest_equipment_breaks: Vec::new(),
            guest_block_sounds: Vec::new(),
            guest_player_sounds: Vec::new(),
            bound_sounds: Vec::new(),
            active_player_breaks: Vec::new(),
            lan_acting_name: None,
            lan_acting_uuid: None,
            tnt_refused: None,
            lan_acting_permission: None,
            lan_acting_spawn_protection: None,
            lan_acting_entity: None,
            lan_acting_fake: false,
            chat_capture: None,
            admin_feedback: Vec::new(),
            console_broadcast: None,
            rcon_source: false,
            bosses: Default::default(),
            rand_sequences: Default::default(),
            brushing: Default::default(),
            restore_player_state: true,
            acting_input: acting_state::ServerActingInput::default(),
            pending_guest_ui: acting_state::ServerPendingGuestUi::default(),
            audio_capture_active: false,
            pending_sounds: Vec::new(),
            #[cfg(any(test, feature = "test-support"))]
            test_positional_sound_counts: Default::default(),
            pending_particles: Vec::new(),
            pending_explosions: Vec::new(),
            pending_native_sounds: Vec::new(),
            effect_rng: sunrise_core::random::LegacyRandom::new(0),
            sculk: Default::default(),
            save,
            player_autosaves: ws.player_autosaves,
            session_lock: ws.session_lock,
            paused_mob_hurt: ws.paused_mob_hurt,
            raiding: ws.raiding,
            world_tables: ws.world_tables,
            server_loot: ws.server_loot,
            levels: ws.levels,
            tickets: ws.tickets,
            random_tick_sections: ws.random_tick_sections,
            others: ServerShared::new(Default::default()),
            spill: ws.spill,
            creative_catalog,
            game_tests: ws.game_tests,
            native_commands: ws.native_commands,
            lan_building_boxes: ws.lan_building_boxes,
            lan_displaced_players: ws.lan_displaced_players,
            fake_action: ws.fake_action,
            server_lists: ws.server_lists,
            user_cache: None,
            cmds: ws.cmds,
            reload_resources: ws.reload_resources,
            combat: ws.combat,
            guest_combat: ws.guest_combat,
            fns: ws.fns,
            exec: ws.exec,
            profile_resolver,
        })
    }
}

#[allow(dead_code)]
#[repr(transparent)]
pub struct ServerOnly<T>(pub T);

#[allow(dead_code)]
impl<T> ServerOnly<T> {
    #[inline(always)]
    pub(in crate::game) fn new(value: T) -> Self {
        Self(value)
    }
}

#[allow(dead_code)]
#[repr(transparent)]
pub struct ServerShared<T>(pub T);

#[allow(dead_code)]
impl<T> ServerShared<T> {
    #[inline(always)]
    pub(in crate::game) fn new(value: T) -> Self {
        Self(value)
    }
}

impl sunrise_gameplay_common::game::Side for ServerState {
    type Nominal<'g> = crate::game::ServerGame<'g>;

    #[inline(always)]
    fn streaming_state(&self) -> &sunrise_level::chunk_streamer::ChunkState {
        self.streaming.state()
    }

    #[inline(always)]
    fn shulkers(
        &self,
    ) -> &sunrise_level::fast_hash::FxHashMap<[i32; 3], sunrise_level::shulker_box::Shulker> {
        &self.be_anims.shulkers
    }

    #[inline(always)]
    fn auto_jump(&self) -> bool {
        self.settings.auto_jump
    }

    #[inline(always)]
    fn main_hand_left(&self) -> bool {
        self.settings.main_hand_left
    }

    #[inline(always)]
    fn operator_items_tab(&self) -> bool {
        self.settings.operator_items_tab
    }

    #[inline(always)]
    fn mods_list(&self) -> &sunrise_mod_package::mod_list::ModList {
        &self.settings.mods
    }

    #[inline(always)]
    fn mods_list_mut(&mut self) -> &mut sunrise_mod_package::mod_list::ModList {
        &mut self.settings.mods
    }

    const IS_SERVER: bool = true;
    fn host_side<'a, 'g>(game: &'g mut crate::game::ServerGame<'a>) -> mod_api::HostSide<'g> {
        mod_api::HostSide::Server(game)
    }
    fn as_host<'a, 'g>(game: &'g crate::game::ServerGame<'a>) -> &'g dyn mod_api::Host {
        game
    }
    fn as_block_states_backend<'a, 'g>(
        game: &'g crate::game::ServerGame<'a>,
    ) -> &'g dyn mod_api::prelude::BlockStatesBackend {
        game
    }
    fn as_config_context<'a, 'g>(
        game: &'g crate::game::ServerGame<'a>,
    ) -> &'g dyn mod_api::ConfigContext {
        game
    }
    fn as_reload_context<'a, 'g>(
        game: &'g crate::game::ServerGame<'a>,
    ) -> &'g dyn mod_api::ReloadContext {
        game
    }
    fn as_pack_discovery_context_mut<'a, 'g>(
        game: &'g mut crate::game::ServerGame<'a>,
    ) -> &'g mut dyn mod_api::PackDiscoveryContext {
        game
    }
    fn as_server_host_mut<'a, 'g>(
        game: &'g mut crate::game::ServerGame<'a>,
    ) -> Option<&'g mut dyn mod_api::ServerHost> {
        Some(game)
    }
    fn attachment_store<'a, 'g>(
        game: &'g mut crate::game::ServerGame<'a>,
    ) -> &'g mut dyn mod_api::AttachmentStore {
        game
    }
    fn world_tables(&self) -> Option<&sunrise_gameplay_common::game::WorldTables> {
        self.world_tables.0.as_ref()
    }
    fn lan_acting_uuid(&self) -> Option<u128> {
        self.lan_acting_uuid
    }
    fn levels_generation(&self, id: &str) -> u32 {
        self.levels.0.runtime.generation_of(id)
    }
    fn session_lock_active(&self) -> bool {
        self.session_lock.0.is_some()
    }
    fn level_ref(
        &self,
        dim: sunrise_gameplay_common::game::Dim,
        epoch: Option<mod_api::WorldEpoch>,
    ) -> Option<mod_api::LevelRef> {
        epoch.and_then(|epoch| {
            let key = dim.key();
            let id = mod_api::__private::dimension_id(key)?;
            Some(mod_api::LevelRef::with_generation(
                epoch,
                key,
                self.levels.0.runtime.generation_of(id),
            ))
        })
    }
    fn held_out(&self) -> &[u32] {
        &self.held_out
    }
    fn lan_acting(&self) -> Option<u32> {
        self.lan_acting
    }
    fn lan_acting_entity(&self) -> Option<u32> {
        self.lan_acting_entity
    }
    fn lan_acting_fake(&self) -> bool {
        self.lan_acting_fake
    }
    fn lan_guest_swaps(&self) -> &[u32] {
        &self.lan_guest_swaps
    }
    fn acting_abilities(&self) -> Option<sunrise_sim::lan::abilities::PlayerAbilities> {
        self.acting_abilities
    }
    fn native_command_filter(&self) -> bool {
        self.native_command_filter
    }
    fn riding_order(&self) -> i64 {
        self.riding_order
    }
    fn lan_acting_name(&self) -> Option<&str> {
        self.lan_acting_name.as_deref()
    }
    fn server_tick_deadline(&self) -> Option<std::time::Instant> {
        self.server_tick_deadline
    }
}

#[path = "world/owner_link_server.rs"]
pub(in crate::game) mod owner_link_server;

#[path = "runtime/resource_load_server.rs"]
pub mod resource_load_server;

#[path = "runtime/transactions/decision_engine_server.rs"]
pub mod decision_engine_server;

#[path = "blocks/maps/mod.rs"]
pub(in crate::game) mod maps_server;

#[path = "blocks/cauldron_interactions_server.rs"]
pub(in crate::game) mod cauldron_interactions_server;

#[path = "entities/entity.rs"]
pub mod entity_server;

#[path = "entities/general_server.rs"]
pub(in crate::game) mod general_server;

#[path = "modding/native_world_read_server.rs"]
pub mod native_world_read_server;

#[path = "modding/native_passages_server.rs"]
pub(in crate::game) mod native_passages_server;

#[path = "modding/native_registry_entries_server.rs"]
pub(in crate::game) mod native_registry_entries_server;

#[path = "administration/lan_op.rs"]
pub(in crate::game) mod lan_op;

#[path = "items/durability.rs"]
pub(in crate::game) mod durability;

pub(in crate::game) struct ServerConstructPrep {
    pub(in crate::game) level: Level,
    pub(in crate::game) save: Arc<SaveDir>,
    pub(in crate::game) painting_variants:
        sunrise_gameplay_common::painting_variants::PaintingVariants,
    pub(in crate::game) item_features: sunrise_items::creative_catalog::ItemFeatureRequirements,
    pub(in crate::game) creative_catalog: Arc<sunrise_items::creative_catalog::Catalog>,
    pub(in crate::game) world: World,
    pub(in crate::game) now: Instant,
    pub(in crate::game) pack: PackResources,
    pub(in crate::game) options: Options,
    pub(in crate::game) settings: sunrise_level::sim_settings::SimSettings,
    pub(in crate::game) streaming: sunrise_level::chunk_streamer::ChunkStreamer,
}

pub(in crate::game) struct ServerConstructEffects {
    pub(in crate::game) structure_spawns: sunrise_sim::mobs::spawn_selection::StructureSpawns,
    pub(in crate::game) clock: FixedStep,
    pub(in crate::game) tick: sunrise_sim::parallel::TickThread,
    pub(in crate::game) profile_resolver: Box<dyn sunrise_level::profile_lookup::ProfileResolver>,
    pub(in crate::game) profile: profile_capture_server::ServerProfile,
    pub(in crate::game) ui: chat_common::WorldCommandState,
}

impl<'a> ServerGame<'a> {
    /// A logical server game (dedicated server or integrated server thread).
    pub fn new(
        res: &'a Arc<Resources>,
        pack: PackResources,
        options: Options,
        settings: sunrise_level::sim_settings::SimSettings,
        prepared: Option<resource_cpu_common::ServerCpuResources>,
        profile_resolver: integrated_link::ProfileResolverFactory,
    ) -> Result<Self> {
        debug_assert!(options.execution_role.is_logical_server());
        let prep = Self::prepare_construct(res, pack, options, settings, prepared)?;
        let effects = Self::prepare_effects(&prep, profile_resolver);
        Ok(Self::finish_construct(res, prep, effects))
    }

    #[inline(never)]
    fn prepare_construct(
        res: &'a Arc<Resources>,
        pack: PackResources,
        options: Options,
        settings: sunrise_level::sim_settings::SimSettings,
        prepared: Option<resource_cpu_common::ServerCpuResources>,
    ) -> Result<Box<ServerConstructPrep>> {
        #[cfg(test)]
        {
            static INITIALIZED: std::sync::Once = std::sync::Once::new();
            INITIALIZED.call_once(|| {
                mod_platform::__private::test_platform::install();
                mod_platform::__private::initialize()
                    .expect("engine test runtime initializes before mod dispatch");
            });
        }
        // A scoped hook on this thread asks this game's parts.
        mod_api::__private::mark_game_thread();
        let root = options.resources.clone();
        let role = options.execution_role;
        let loop_side = role.loop_side();
        let item_data = res.core.item_data.context();
        let inventory = sunrise_mod_package::enabled::EnabledMods::read(&root)?;
        let server_owners: std::collections::BTreeSet<_> = inventory.server_ids().collect();
        let loop_context = if item_data.origin() == role.data_load_origin() {
            item_data.captured_loop_context(loop_side)
        } else {
            let registered =
                sunrise_gameplay_common::game::native_hooks::mod_settings::registered_setting_keys(
                )
                .filter(|(owner, _)| server_owners.contains(owner));
            item_data.loop_context_with_registered(loop_side, [], registered)
        };
        let _ = options
            .mods
            .settings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .sync_from_loop_context(&loop_context, |owner| server_owners.contains(owner));
        let registry = pack.assets.registry;
        let level = Level::default();
        let save = Arc::new(SaveDir::new(
            std::env::current_dir()?
                .join("saves")
                .join(".sunrise-title"),
            sunrise_sim::save::LevelShape::OVERWORLD,
            None,
        ));
        let prepared = prepared.map(Ok).unwrap_or_else(|| {
            resource_cpu_common::ServerCpuResources::load(
                &sunrise_assets::resource_source::ResourceRoot::new(
                    root.clone(),
                    packs_common::cache_dir(),
                    pack.source.clone(),
                ),
            )
        })?;
        let world = sunrise_gameplay_common::game::authoritative_world_for_mods(res, 0);
        let now = Instant::now();
        // Until a world is opened the streamer is idle.
        let mut streaming = sunrise_level::chunk_streamer::ChunkStreamer::new(
            Arc::new(TestTerrain::new(registry, res.biomes())?),
            pack.chunk_context.clone(),
            1,
        );
        streaming.paused = true;
        let painting_variants = prepared.painting_variants;
        let item_features = prepared.item_features;
        let creative_catalog = prepared.creative_catalog;
        Ok(Box::new(ServerConstructPrep {
            level,
            save,
            painting_variants,
            item_features,
            creative_catalog,
            world,
            now,
            pack,
            options,
            settings,
            streaming,
        }))
    }

    #[inline(never)]
    fn prepare_effects(
        prep: &ServerConstructPrep,
        profile_resolver: integrated_link::ProfileResolverFactory,
    ) -> Box<ServerConstructEffects> {
        #[cfg(not(test))]
        let structure_spawns =
            sunrise_sim::mobs::spawn_selection::StructureSpawns::load(&prep.options.resources);
        #[cfg(test)]
        let structure_spawns =
            sunrise_sim::mobs::spawn_selection::StructureSpawns::load(&prep.options.resources);
        let clock = FixedStep::default();
        let tick = sunrise_sim::parallel::TickThread::mint();
        let profile_resolver = profile_resolver();
        let profile = profile_capture_server::ServerProfile::default();
        let ui = chat_common::WorldCommandState::default();
        Box::new(ServerConstructEffects {
            structure_spawns,
            clock,
            tick,
            profile_resolver,
            profile,
            ui,
        })
    }

    #[inline(never)]
    fn finish_construct(
        res: &'a Arc<Resources>,
        prep: Box<ServerConstructPrep>,
        effects: Box<ServerConstructEffects>,
    ) -> Self {
        let (wp, server) =
            crate::game::ConstructInit::<true>::wrap_server_prep_and_effects(prep, effects);
        let mut game = crate::game::ServerGame {
            common: sunrise_gameplay_common::game::ConstructInit::<true>::assemble_common(res, wp),
            server: *server,
        };
        game.reset_entity_residency();
        game
    }
}
