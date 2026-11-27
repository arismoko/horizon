//! Sunrise native client: resources, world streaming, rendering and the
//! bindings between the validated simulation pieces and the game loop.
#![forbid(unsafe_code)]
extern crate self as sunrise_client;
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: sunrise_threads::allocation_probe::Allocator =
    sunrise_threads::allocation_probe::Allocator;
pub use sunrise_sim::advancements;
pub mod ambience;
pub mod animate_tick;
pub use sunrise_sim::api_text;
pub use sunrise_sim::bed_geometry;
pub use sunrise_sim::block_contact;
pub use sunrise_sim::block_sounds;
pub use sunrise_sim::entities::arrows;
pub use sunrise_sim::item_damage;
pub use sunrise_sim::mod_packet_builders;
#[cfg(test)]
mod fresh_process;
pub mod packet_source;
pub use sunrise_sim::parallel;
pub use sunrise_sim::powder_snow;
#[cfg(test)]
mod test_log;
#[cfg(test)]
mod test_resources;
pub use sunrise_client_render::moving_colliders;
pub use sunrise_client_render::{
    assets, atlas_sources, audio, auth, autotune, ban_notice, bidi, biome, block_edit,
    block_entities, block_entity_anim, block_entity_render, block_entity_ticks, block_sprites,
    block_states, bluemap, border_render, breaking, brewing, camera, chat, chunk, chunk_map, cit,
    cit_armor, cit_assets, cit_glint, client_resources, clouds, collisions, columns, command_block,
    containers, controller_ui, crash, custom_sky, datapack, dynamic_light, dynamic_resolution,
    effects, environment, extras, fast_hash, fluid_models, fluid_render, fog, font, gfx, graphics,
    graphics_style, graphics_ui, hdr_calibration, hud, hud_kit, hud_layout, hud_overlays,
    inventory_effects, java_text, jigsaw_block, json_folder, level_loading, light, lod, logging,
    mark_sprite, menu_blur, menu_style, mesher, mipmap, mod_chunk_tickets, mod_hooks, mod_list,
    mod_pipelines, model_library, models, mods_screen, monitor, multiline_edit, native_emissive,
    options_file, options_fixes, options_ui, overlay, pack_browser, pack_formats, pack_hooks,
    pack_image, pack_palette, pack_screen, packs, pad_cursor, pad_glyph, par, particle_options,
    particles, poi_sections, polytone_creative_tabs, polytone_post, post_chain, potent_sulfur,
    preedit, providers, pvp, pvp_hud, random_source, resource_source, screen_draw, screen_effects,
    sculk, section_mask, server_terrain, settings, shader_options, shader_pack_list, shulker_box,
    signs, skins, social, sound_engine, sound_events, sound_preview, spans, sprites, streaming,
    structure_block, tab_list, terrain_model, test_block, test_instance_block, testgen, trims, ui,
    vault, weather, widgets, world, worldborder, zip,
};
pub use sunrise_sim::attack_strength;
pub use sunrise_sim::attribute_updates;
pub use sunrise_sim::auto_jump;
pub use sunrise_sim::banner;
pub use sunrise_sim::beehive;
pub mod benchmark;
pub use sunrise_sim::block_entity_item;
pub use sunrise_sim::block_pattern;
pub use sunrise_sim::block_transform;
pub mod boss_overlay;
pub use sunrise_sim::breathing;
pub use sunrise_sim::cauldron;
pub use sunrise_sim::chunk_io;
pub mod client_perf;
pub use sunrise_sim::clock;
pub use sunrise_sim::colliders;
pub use sunrise_sim::collision_parts;
pub use sunrise_sim::command_storage;
pub mod confirm_screen;
pub mod container_screen;
pub use sunrise_sim::content_identity;
pub use sunrise_sim::control;
pub use sunrise_sim::crafting;
pub mod creative;
pub use sunrise_sim::cushion;
pub use sunrise_sim::data_cmd;
pub use sunrise_sim::datapack_net;
pub mod debug_chart;
pub mod debug_entries;
pub mod debug_keys;
pub mod debug_overlay;
pub use sunrise_sim::debugworld;
pub mod desktop;
pub use sunrise_sim::dialog;
pub mod dialog_screen;
pub use sunrise_sim::enchantment_levels;
pub mod end_crystal;
pub use sunrise_sim::end_gateway;
pub use sunrise_sim::entities;
pub use sunrise_sim::entity_nbt;
pub use sunrise_sim::entity_rules;
pub use sunrise_sim::entity_solids;
pub mod extras_hud;
pub use sunrise_sim::falling_block;
pub mod fire_render;
pub mod first_person;
pub use sunrise_sim::first_person_sim;
pub use sunrise_sim::flatgen;
pub mod fluid_tooltip;
pub use sunrise_sim::furnace;
pub mod game_mode_switcher;
pub mod game_rules_screen;
pub mod gizmos;
pub use sunrise_sim::gossip;
pub mod gui_layers;
pub use sunrise_sim::health;
pub use sunrise_sim::hotbar;
pub use sunrise_sim::hunger;
pub use sunrise_sim::inv_sort;
pub mod item_activation;
pub use sunrise_sim::item_attributes;
pub use sunrise_sim::item_models;
pub use sunrise_sim::item_names;
pub mod item_render;
pub use sunrise_sim::item_sync;
pub mod item_tooltip;
pub use sunrise_sim::impl_joined;
pub use sunrise_sim::join_seq;
pub use sunrise_sim::move_gate;
#[cfg(test)]
mod test_game_dir;
/// Game-owned JSON metadata conversion, shared by recipe and API adapters.
pub use sunrise_core::json_nbt;
pub use sunrise_sim::lan;
pub use sunrise_sim::landing;
pub use sunrise_sim::lang_load;
pub use sunrise_sim::living_attributes;
pub mod locator_bar;
pub use sunrise_sim::lod_net;
pub use sunrise_sim::map_decoration_types;
pub use sunrise_sim::menu;
pub use sunrise_sim::menu_extra_net;
pub use sunrise_sim::merchant;
pub mod mobs;
pub mod mobs_render;
pub use sunrise_sim::mod_content;
pub mod mod_content_client;
pub use sunrise_sim::modgen;
pub use sunrise_sim::motion;
pub mod multiplayer;
pub mod music;
pub use sunrise_sim::native_client_hooks;
pub use sunrise_sim::native_stack_hooks;
pub use sunrise_sim::neoforge_mob_maps;
pub mod net;
pub use sunrise_sim::nether_portal;
pub mod no_resources;
pub use sunrise_sim::offline_name;
pub use sunrise_sim::old_world;
pub mod osk;
pub use sunrise_sim::patrol;
pub use sunrise_sim::perf_net;
pub use sunrise_sim::perf_types;
pub use sunrise_sim::pick_item;
pub mod player_sync;
pub use sunrise_sim::portal_sections;
pub use sunrise_sim::pose;
pub use sunrise_sim::pregen;
pub use sunrise_sim::projectiles;
pub use sunrise_sim::raid;
pub use sunrise_sim::random_tick_sections;
pub use sunrise_sim::raw_payloads;
pub mod realms;
pub use sunrise_sim::recipe_book;
pub mod recipe_book_screen;
pub use sunrise_sim::save;
// The carrier ownership store: an internal storage layer of this
// library, used by the game crate's transfers (game/world/), like
// save.rs below it. Not part of the mods' API surface.
#[cfg(test)]
mod carrier_recover_tests;
pub use sunrise_sim::save::carrier as carrier_store;
#[cfg(test)]
mod carrier_store_tests;
pub use sunrise_sim::scoreboard_cmd;
pub mod screen_layout;
pub mod screen_narrator;
pub mod skull_models;
pub mod social_screen;
pub mod world_geometry;
pub use sunrise_server_files as server;
pub mod controller_notice;
pub mod server_packs;
pub use sunrise_sim::server_perf;
pub mod shadows;
pub mod spear;
pub use sunrise_sim::spear_sim;
pub mod spectator_menu;
pub mod splashes;
pub use sunrise_sim::sprint;
pub use sunrise_sim::stand_up;
pub use sunrise_sim::stations;
pub use sunrise_sim::stopwatches;
pub use sunrise_sim::structure_blocks;
pub mod studio;
pub mod telemetry;
pub mod text_display;
pub use sunrise_sim::text_display_data;
pub use sunrise_sim::text_grammar;
pub use sunrise_sim::tick_chunk_membership;
pub use sunrise_sim::ticket_levels;
pub mod title;
pub mod toasts;
pub mod tooltip_components;
pub use sunrise_sim::travel_gate;
pub mod ui_theme;
pub use sunrise_sim::vanillagen;
pub use sunrise_sim::viewed_levels_net;
pub mod voxel_occlusion;
// Keep the old path for native callers while giving the implementation a name
// that does not collide with the gameplay adapter in `game/interface/entity_culling/mod.rs`.
pub use voxel_occlusion as entity_culling;
pub mod win_screen;
pub mod world_primitives;
pub use sunrise_client_input::actions;
pub use sunrise_client_input::clipboard;
pub use sunrise_client_input::gamepad;
pub use sunrise_client_input::gameplay_guide;
pub use sunrise_client_input::held;
pub use sunrise_client_input::input;
pub use sunrise_client_input::input_source;
pub use sunrise_client_input::interaction_input;
pub use sunrise_client_input::key_names;
pub use sunrise_client_input::keybinds;
pub use sunrise_client_input::keymapping;
pub use sunrise_client_input::mouse;
pub use sunrise_client_input::window;
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
pub use sunrise_items::flowers;
pub use sunrise_items::item_data;
pub use sunrise_items::items;
pub use sunrise_items::random_sequences;
pub use sunrise_sim::world_options;
pub use sunrise_sim::world_spawn;

#[cfg(test)]
mod brewing_speed_load_tests;
