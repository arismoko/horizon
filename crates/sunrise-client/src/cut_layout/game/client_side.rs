#![cfg(test)]

#[cfg(test)]
pub(crate) use super::server_side::native_entity_refs;

#[cfg(feature = "client")]
#[cfg(test)]
#[path = "blocks/beacon_cull_tests.rs"]
pub(crate) mod beacon_cull_tests;
#[cfg(feature = "client")]
#[cfg(test)]
#[path = "runtime/core_tests_client.rs"]
pub(crate) mod core_tests_client;
#[cfg(feature = "client")]
#[cfg(test)]
#[path = "entities/crosshair_pick_tests.rs"]
pub(crate) mod crosshair_pick_tests;
#[cfg(feature = "client")]
#[cfg(test)]
#[path = "modding/disabled_block_tests_client.rs"]
pub(crate) mod disabled_block_tests_client;
#[cfg(feature = "client")]
#[cfg(test)]
#[path = "interface/dragon_boss_bar_name_tests.rs"]
pub(crate) mod dragon_boss_bar_name_tests;
#[cfg(feature = "client")]
#[cfg(test)]
#[path = "blocks/enchant_power_beam_tests.rs"]
pub(crate) mod enchant_power_beam_tests;
#[cfg(feature = "client")]
#[cfg(test)]
#[path = "modding/gui_layer_tests.rs"]
pub(crate) mod gui_layer_tests;
#[cfg(feature = "client")]
#[cfg(test)]
#[path = "items/hand_part_tests.rs"]
pub(crate) mod hand_part_tests;
#[cfg(feature = "client")]
#[cfg(test)]
#[path = "interface/highlight_tip_tests.rs"]
pub(crate) mod highlight_tip_tests;
#[cfg(feature = "client")]
#[cfg(test)]
#[path = "modding/initial_client_models_tests_client.rs"]
pub(crate) mod initial_client_models_tests_client;
#[cfg(feature = "client")]
#[cfg(test)]
#[path = "modding/mod_menu_decisions_tests.rs"]
pub(crate) mod mod_menu_decisions_tests;
#[cfg(feature = "client")]
#[cfg(test)]
#[path = "modding/mod_raycast_tests.rs"]
pub(crate) mod mod_raycast_tests;
#[cfg(feature = "client")]
#[cfg(test)]
#[path = "interface/mod_recipe_property_tests.rs"]
pub(crate) mod mod_recipe_property_tests;
#[cfg(feature = "client")]
#[cfg(test)]
#[path = "modding/native_general_renderers_tests.rs"]
pub(crate) mod native_general_renderers_tests;
#[cfg(feature = "client")]
#[cfg(test)]
#[path = "modding/native_guest_turn_tests_client.rs"]
pub(crate) mod native_guest_turn_tests_client;
#[cfg(feature = "client")]
#[cfg(test)]
#[path = "modding/native_key_mappings_tests.rs"]
pub(crate) mod native_key_mappings_tests;
#[cfg(feature = "client")]
#[cfg(test)]
#[path = "modding/native_level_sound_except_tests_client.rs"]
pub(crate) mod native_level_sound_except_tests_client;
#[cfg(feature = "client")]
#[cfg(test)]
#[path = "modding/native_scene_live_tests.rs"]
pub(crate) mod native_scene_live_tests;
#[cfg(feature = "client")]
#[cfg(test)]
#[path = "modding/native_small_reads_tests.rs"]
pub(crate) mod native_small_reads_tests;
#[cfg(feature = "client")]
#[cfg(test)]
#[path = "interface/options_close_tests.rs"]
pub(crate) mod options_close_tests;
#[cfg(feature = "client")]
#[cfg(test)]
#[path = "interface/polytone_frame_tests.rs"]
pub(crate) mod polytone_frame_tests;
#[cfg(feature = "client")]
#[cfg(test)]
#[path = "items/projectile_spawned_tests_client.rs"]
pub(crate) mod projectile_spawned_tests_client;
#[cfg(feature = "client")]
#[cfg(test)]
#[path = "interface/recipe_properties_tests.rs"]
pub(crate) mod recipe_properties_tests;
#[cfg(feature = "client")]
#[cfg(test)]
#[path = "interface/screen_render_phase_tests.rs"]
pub(crate) mod screen_render_phase_tests;
#[cfg(feature = "client")]
#[path = "world/server_packs.rs"]
pub(crate) mod server_packs;
#[cfg(feature = "client")]
#[cfg(test)]
#[path = "interface/tests/sign_chain_tests.rs"]
pub(crate) mod sign_chain_tests;
#[cfg(feature = "client")]
#[cfg(test)]
#[path = "mobs/spawner_client_tick_tests.rs"]
pub(crate) mod spawner_client_tick_tests;
#[cfg(feature = "client")]
#[cfg(test)]
#[path = "modding/stacked_on_other_tests_client.rs"]
pub(crate) mod stacked_on_other_tests_client;
#[cfg(feature = "client")]
#[cfg(test)]
#[path = "blocks/sulfur_bucket_tests.rs"]
pub(crate) mod sulfur_bucket_tests;
#[cfg(feature = "client")]
#[cfg(test)]
#[path = "entities/vehicle_removal_tests_client.rs"]
pub(crate) mod vehicle_removal_tests_client;
#[cfg(feature = "client")]
#[cfg(test)]
#[path = "blocks/world_fluids_tests_client.rs"]
pub(crate) mod world_fluids_tests_client;

#[path = "world/packs/mod.rs"]
pub(crate) mod packs;

#[cfg(feature = "client")]
#[path = "runtime/launch_timeout.rs"]
pub(crate) mod launch_timeout;

#[path = "runtime/agent/mod.rs"]
pub(crate) mod agent;

#[path = "network/viewed_worlds/mod.rs"]
pub(crate) mod viewed_worlds;

#[path = "network/remote_vehicles.rs"]
pub(crate) mod remote_vehicles;

#[cfg(feature = "client")]
#[path = "network/remote_tags.rs"]
pub(crate) mod remote_tags;

#[cfg(feature = "client")]
#[path = "network/remote_removal.rs"]
pub(crate) mod remote_removal;

#[path = "network/remote_projectiles.rs"]
pub(crate) mod remote_projectiles;

#[path = "network/remote_chunks.rs"]
pub(crate) mod remote_chunks;

#[path = "network/remote_camera.rs"]
pub(crate) mod remote_camera;

#[path = "network/remote/mod_client.rs"]
pub(crate) mod remote_client;

#[cfg(feature = "client")]
#[path = "network/lan_particles_client.rs"]
pub(crate) mod lan_particles_client;

#[cfg(feature = "client")]
#[path = "network/lan/mod_client.rs"]
pub(crate) mod lan_client;

#[cfg(feature = "client")]
#[path = "native_hooks/mod_client.rs"]
pub(crate) mod native_hooks_client;

#[cfg(feature = "client")]
#[path = "modding/native_visuals.rs"]
pub(crate) mod native_visuals;

#[cfg(feature = "client")]
#[path = "modding/native_viewed_levels.rs"]
pub(crate) mod native_viewed_levels;

#[cfg(feature = "client")]
#[path = "modding/native_special_renderers.rs"]
pub(crate) mod native_special_renderers;

#[cfg(feature = "client")]
#[path = "modding/native_panel.rs"]
pub(crate) mod native_panel;

#[cfg(feature = "client")]
#[path = "modding/native_packet_builders.rs"]
pub(crate) mod native_packet_builders;

#[path = "modding/native_net.rs"]
pub(crate) mod native_net;

#[cfg(feature = "client")]
#[path = "modding/native_local_player_writes.rs"]
pub(crate) mod native_local_player_writes;

#[cfg(feature = "client")]
#[path = "modding/native_living_effects_client.rs"]
pub(crate) mod native_living_effects_client;

#[cfg(feature = "client")]
#[path = "modding/native_key_mappings.rs"]
pub(crate) mod native_key_mappings;

#[cfg(feature = "client")]
#[path = "modding/native_item_particles.rs"]
pub(crate) mod native_item_particles;

#[path = "modding/native_hotbar_slot.rs"]
pub(crate) mod native_hotbar_slot;

#[cfg(feature = "client")]
#[path = "modding/native_entity_riding_client.rs"]
pub(crate) mod native_entity_riding_client;

#[cfg(feature = "client")]
#[path = "modding/native_entity_refs_client.rs"]
pub(crate) mod native_entity_refs_client;

#[path = "modding/native_connection_groups.rs"]
pub(crate) mod native_connection_groups;

#[cfg(feature = "client")]
#[path = "modding/native_client_particles.rs"]
pub(crate) mod native_client_particles;

#[cfg(feature = "client")]
#[path = "modding/native_block_sprites.rs"]
pub(crate) mod native_block_sprites;

#[cfg(feature = "client")]
#[path = "modding/native_block_entity_draw.rs"]
pub mod native_block_entity_draw;

#[cfg(feature = "client")]
#[path = "modding/gui_clips.rs"]
pub(crate) mod gui_clips;

#[cfg(feature = "client")]
#[path = "modding/entity_sizes_client.rs"]
pub(crate) mod entity_sizes_client;

#[cfg(feature = "client")]
#[path = "modding/client_executor.rs"]
pub(crate) mod client_executor;

#[cfg(feature = "client")]
#[path = "modding/client_cracks.rs"]
pub(crate) mod client_cracks;

#[cfg(feature = "client")]
#[path = "mobs/sniffers.rs"]
pub(crate) mod sniffers;

#[cfg(feature = "client")]
#[path = "mobs/ominous_item_spawner/mod_client.rs"]
pub(crate) mod ominous_item_spawner_client;

#[cfg(feature = "client")]
#[path = "mobs/nautili.rs"]
pub(crate) mod nautili;

#[cfg(feature = "client")]
#[path = "mobs/dragon_client.rs"]
pub(crate) mod dragon_client;

#[cfg(feature = "client")]
#[path = "mobs/creatures/mod_client.rs"]
pub(crate) mod creatures_client;

#[cfg(feature = "client")]
#[path = "mobs/breezes_client.rs"]
pub(crate) mod breezes_client;

#[cfg(feature = "client")]
#[path = "items/use_hand_client.rs"]
pub(crate) mod use_hand_client;

#[cfg(feature = "client")]
#[path = "items/trident_client.rs"]
pub(crate) mod trident_client;

#[cfg(feature = "client")]
#[path = "items/trading_client.rs"]
pub(crate) mod trading_client;

#[path = "items/recipe_book.rs"]
pub(crate) mod recipe_book;

#[path = "items/potions.rs"]
pub(crate) mod potions;

#[cfg(feature = "client")]
#[path = "items/item_use_client.rs"]
pub(crate) mod item_use_client;

#[cfg(feature = "client")]
#[path = "items/item_entities_client.rs"]
pub(crate) mod item_entities_client;

#[cfg(feature = "client")]
#[path = "items/fishing_client.rs"]
pub(crate) mod fishing_client;

#[cfg(feature = "client")]
#[path = "items/crossbow_client.rs"]
pub(crate) mod crossbow_client;

#[cfg(feature = "client")]
#[path = "items/combat_client.rs"]
pub(crate) mod combat_client;

#[cfg(feature = "client")]
#[path = "items/client_attack_targets.rs"]
pub(crate) mod client_attack_targets;

#[cfg(feature = "client")]
#[path = "interface/wind_client.rs"]
pub(crate) mod wind_client;

#[cfg(feature = "client")]
#[path = "interface/tutorial.rs"]
pub(crate) mod tutorial;

#[cfg(feature = "client")]
#[path = "interface/titles.rs"]
pub(crate) mod titles;

#[cfg(feature = "client")]
#[path = "interface/startup_menu.rs"]
pub(crate) mod startup_menu;

#[cfg(feature = "client")]
#[path = "interface/spectator.rs"]
pub(crate) mod spectator;

#[cfg(feature = "client")]
#[path = "interface/sign_edit.rs"]
pub(crate) mod sign_edit;

#[cfg(feature = "client")]
#[path = "interface/shader_packs.rs"]
pub(crate) mod shader_packs;

#[path = "interface/screen_build.rs"]
pub(crate) mod screen_build;

#[path = "interface/render_tick.rs"]
pub(crate) mod render_tick;

#[cfg(feature = "client")]
#[path = "interface/pointer_capture.rs"]
pub(crate) mod pointer_capture;

#[cfg(feature = "client")]
#[path = "interface/play_keys.rs"]
pub(crate) mod play_keys;

#[cfg(feature = "client")]
#[path = "interface/pad/mod.rs"]
pub(crate) mod pad;

#[path = "interface/options.rs"]
pub(crate) mod options;

#[cfg(feature = "client")]
#[path = "interface/native_client_screens.rs"]
pub(crate) mod native_client_screens;

#[path = "interface/names.rs"]
pub(crate) mod names;

#[cfg(feature = "client")]
#[path = "interface/music.rs"]
pub(crate) mod music;

#[path = "interface/mount_menu.rs"]
pub(crate) mod mount_menu;

#[cfg(feature = "client")]
#[path = "interface/mod_toasts.rs"]
pub(crate) mod mod_toasts;

#[cfg(feature = "client")]
#[path = "interface/locator_bar.rs"]
pub(crate) mod locator_bar;

#[cfg(feature = "client")]
#[path = "interface/level_particles_client.rs"]
pub(crate) mod level_particles_client;

#[path = "interface/item_draw.rs"]
pub(crate) mod item_draw;

#[cfg(feature = "client")]
#[path = "interface/gui_entity.rs"]
pub(crate) mod gui_entity;

#[cfg(feature = "client")]
#[path = "interface/extras.rs"]
pub(crate) mod extras;

#[path = "interface/dialogs.rs"]
pub(crate) mod dialogs;

#[cfg(feature = "client")]
#[path = "interface/demo_intro.rs"]
pub(crate) mod demo_intro;

#[cfg(feature = "client")]
#[path = "interface/debug_options.rs"]
pub(crate) mod debug_options;

#[path = "interface/debug_keys.rs"]
pub(crate) mod debug_keys;

#[cfg(feature = "client")]
#[path = "interface/container_hooks.rs"]
pub(crate) mod container_hooks;

#[cfg(feature = "client")]
#[path = "interface/clipboard.rs"]
pub(crate) mod clipboard;

#[cfg(feature = "client")]
#[path = "interface/client_reload/mod.rs"]
pub(crate) mod client_reload;

#[cfg(feature = "client")]
#[path = "interface/client_ambient.rs"]
pub(crate) mod client_ambient;

#[cfg(feature = "client")]
#[path = "interface/chat.rs"]
pub(crate) mod chat;

#[path = "interface/books.rs"]
pub(crate) mod books;

#[cfg(feature = "client")]
#[path = "interface/advancements_screen.rs"]
pub(crate) mod advancements_screen;

#[cfg(feature = "client")]
#[path = "interface/advancements/mod_client.rs"]
pub(crate) mod advancements_client;

#[path = "entities/text_displays.rs"]
pub(crate) mod text_displays;

#[cfg(feature = "client")]
#[path = "entities/pick_accumulator.rs"]
pub(crate) mod pick_accumulator;

#[cfg(feature = "client")]
#[path = "entities/minecarts_client.rs"]
pub(crate) mod minecarts_client;

#[path = "entities/lightning.rs"]
pub(crate) mod lightning;

#[cfg(feature = "client")]
#[path = "entities/fireworks_client.rs"]
pub(crate) mod fireworks_client;

#[path = "blocks/stations.rs"]
pub(crate) mod stations;

#[cfg(feature = "client")]
#[path = "blocks/shelf.rs"]
pub(crate) mod shelf;

#[path = "blocks/sculk.rs"]
pub(crate) mod sculk;

#[cfg(feature = "client")]
#[path = "blocks/piston_push_client.rs"]
pub(crate) mod piston_push_client;

#[cfg(feature = "client")]
#[path = "blocks/nether_portal_client.rs"]
pub(crate) mod nether_portal_client;

#[cfg(feature = "client")]
#[path = "blocks/cauldron_interactions_client.rs"]
pub(crate) mod cauldron_interactions_client;

#[cfg(feature = "client")]
#[path = "blocks/block_entity_anims_client.rs"]
pub(crate) mod block_entity_anims_client;

#[cfg(feature = "client")]
#[path = "administration/commands/mod_client.rs"]
pub(crate) mod commands_client;

#[cfg(feature = "client")]
#[path = "administration/command_minecarts.rs"]
pub(crate) mod command_minecarts;
